#!/bin/sh
# yunjian Device Farm 判据测量 —— 宿主侧编排。
#
# ## 这一版做了什么
#
# PR #102 那一版**只测 adb 能测的东西**，把 `rms` / `sha256_verified` / `entered_text`
# 一律标成 `MEASURE-UNAVAILABLE`，理由是「那三个 instrumented 测试类在仓库里不存在」。
# 现在它们存在了（`mobile/android/spike/androidTest/`，随 `yunjian-spike-androidtest.apk`
# 一起装到设备上），所以这一版负责：装测试包、`am instrument` 调起三个类、把设备端写下的
# 测量文件搬回宿主侧的日志。
#
# **仍然不在设备侧下结论。** 阈值判定归 `xtask acceptance --platform android --set spike`。
#
# ## 为什么不再启动云笺主界面
#
# PR #102 在真机上实测：`top.onethinker.yunjian/.MainActivity` 一启动就 SIGABRT
# （`ndk-context` 在没有 Android context 时是 `panic!` 而不是 `Err`，于是
# `AppState::new` 首行 `KeyStore::open` 那条本该降级到 `session_memory` 的分支没机会执行）。
# 那个缺陷**没在本轮修**：绕过它要 `catch_unwind`，而那会让密钥静默退化成仅本进程内存，
# 是用户可见的凭据存储行为变化、属安全边界，且修它不会让任何判据变 PASS。
#
# 三条判据问的是「设备与平台能否支撑这些能力」，用来决定 `tauri_mobile` 还是
# `uniffi_native`；产品完整启动流程是 todo 71 的事。所以三个测试类刻意不依赖主界面。
# 主界面的启动结果仍然照测并如实上报（`app_launch`），只是不作为判据依据。
#
# ## 已实测的能力边界（PR #99）
#
#   - 设备上没有 tinycap / arecord，`/system/bin/tiny*` 不存在；PCM 只能由应用内
#     `AudioRecord` 采。
#   - Device Farm 的 `ScheduleRun.configuration` **没有音频注入**，所以非零 RMS 只能来自
#     机架环境噪声。判据①要的正是「不是静音」，这一点它够用；但它不蕴含 todo 71 的
#     `voice_recitation_round_succeeds_end_to_end`——那条要识别**已知**背诵。
#   - Gboard 147 个 subtype 里有且仅有一个中文 subtype `zh_CN`。
#   - run configuration 的 zh_CN 落到 `persist.sys.locale=zh-CN`。

PKG=top.onethinker.yunjian
TEST_PKG="$PKG.test"
RUNNER="$TEST_PKG/androidx.test.runner.AndroidJUnitRunner"
TEST_APK=yunjian-spike-androidtest.apk
APP_APK=yunjian-spike.apk
CLASS_PREFIX=top.onethinker.yunjian.spike

p() { printf '\n===== %s =====\n' "$1"; }
measure() { printf 'YUNJIAN-MEASURE %s %s=%s\n' "$1" "$2" "$3"; }
unavailable() { printf 'YUNJIAN-MEASURE-UNAVAILABLE %s %s reason=%s\n' "$1" "$2" "$3"; }

# 三条判据在设备端测不到时必须逐项标注。集中写在这里，因为「装不上测试包」与
# 「instrument 挂了」要标注的是同一组键，重复三遍容易漏项。
mark_all_unavailable() {
  REASON="$1"
  for KEY in sample_rate_hz channel_count rms permission_plugin; do
    unavailable microphone_capture "$KEY" "$REASON"
  done
  for KEY in artifact_bytes sha256_verified duration_seconds atomic_install crashed production_path; do
    unavailable corpus_materialization "$KEY" "$REASON"
  done
  for KEY in entered_text keyboard_overlap_px input_visible visual_viewport_updated edge_to_edge; do
    unavailable chinese_ime "$KEY" "$REASON"
  done
}

p "DEVICE IDENTITY"
MODEL=$(adb shell getprop ro.product.model 2>/dev/null | tr -d '\r\n')
RELEASE=$(adb shell getprop ro.build.version.release 2>/dev/null | tr -d '\r\n')
SDK=$(adb shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r\n')
FINGERPRINT=$(adb shell getprop ro.build.fingerprint 2>/dev/null | tr -d '\r\n')
echo "model=$MODEL release=$RELEASE sdk=$SDK"
echo "fingerprint=$FINGERPRINT"

p "CRITERION 0 - DID DEVICE FARM INSTALL ITS RE-SIGNED COPY"
INSTALLED=false
if adb shell pm list packages 2>/dev/null | grep -q "$PKG"; then
  INSTALLED=true
fi
measure app_install device_model "$MODEL"
measure app_install os_build "$RELEASE/$SDK"
measure app_install package "$PKG"
measure app_install installed "$INSTALLED"

if [ "$INSTALLED" != "true" ]; then
  echo "云笺未安装，后续判据无法执行"
  adb shell pm list packages 2>/dev/null | head -30
  mark_all_unavailable app_under_test_not_installed
  exit 0
fi

p "DEVICE FARM RE-SIGNED THE APP; REPLACE IT WITH OUR OWN COPY"
# AWS 文档明写：`Device Farm re-signs the app`，且 `skipAppResign` **只对私有设备有效，
# 公共设备一律重签**（本项目的设备池是 fleetType PUBLIC）。因此 Device Farm 装上去的
# 应用带的是它自己的证书，而测试包 zip 里的 APK 不经重签，还是我们的调试证书。
# 两者不一致时 `am instrument` 报
#   Permission Denial: ... does not have a signature matching the target
# 那是第一轮真机实测得到的失败形态，三个类全部 SecurityException。
#
# 所以这里卸掉重签的那份，改装测试包里我们自己的两个 APK。**不要以为构建期那道
# 「两个 APK 同签名」断言能预防它**：重签发生在上传之后，那道断言证明不到这一段。
if [ ! -f "$APP_APK" ]; then
  echo "测试包里没有应用 APK：$APP_APK"
  ls -la
  measure app_install self_installed false
  mark_all_unavailable app_apk_missing_from_test_bundle
  exit 0
fi
DF_SIGNER=$(adb shell dumpsys package "$PKG" 2>/dev/null | sed -n 's/.*signatures=\[\([^]]*\)\].*/\1/p' | head -1 | tr -d '\r\n')
echo "Device Farm 装的那份签名摘要片段：[$DF_SIGNER]"
adb uninstall "$PKG" 2>&1 | tail -2
ls -la "$APP_APK"
# `-t` 允许 testOnly；`-g` 一次性授予运行时权限，省掉判据①的对话框。
adb install -r -t -g "$APP_APK" 2>&1 | tail -5
SELF_INSTALLED=false
if adb shell pm list packages 2>/dev/null | grep -q "$PKG"; then
  SELF_INSTALLED=true
fi
measure app_install self_installed "$SELF_INSTALLED"
if [ "$SELF_INSTALLED" != "true" ]; then
  echo "自装应用 APK 失败，判据无法执行"
  mark_all_unavailable self_install_of_app_apk_failed
  exit 0
fi

p "INSTALLED PACKAGE DETAIL (targetSdk / versionName / apk path)"
adb shell dumpsys package "$PKG" 2>/dev/null | grep -e targetSdk -e versionName -e versionCode -e codePath -e firstInstallTime | head -12
TARGET_SDK=$(adb shell dumpsys package "$PKG" 2>/dev/null | sed -n 's/.*targetSdk=\([0-9]*\).*/\1/p' | head -1 | tr -d '\r\n')
echo "parsed target_sdk=$TARGET_SDK"

p "INSTALL THE INSTRUMENTED TEST PACKAGE"
# `-t` 允许安装 debug（testOnly）包；缺它报 INSTALL_FAILED_TEST_ONLY，而那句话读起来
# 像「这个包有问题」而不是「install 少了一个开关」。
if [ ! -f "$TEST_APK" ]; then
  echo "测试包不在测试包目录里：$TEST_APK"
  ls -la
  measure test_package installed false
  mark_all_unavailable androidtest_apk_missing_from_test_bundle
  exit 0
fi
ls -la "$TEST_APK"
adb install -r -t -g "$TEST_APK" 2>&1 | tail -5
TEST_INSTALLED=false
if adb shell pm list packages 2>/dev/null | grep -q "$TEST_PKG"; then
  TEST_INSTALLED=true
fi
measure test_package package "$TEST_PKG"
measure test_package installed "$TEST_INSTALLED"
if [ "$TEST_INSTALLED" != "true" ]; then
  echo "测试包安装失败"
  adb shell pm list packages 2>/dev/null | grep -i yunjian
  mark_all_unavailable androidtest_apk_install_failed
  exit 0
fi

p "INSTRUMENTATION REGISTERED ON DEVICE"
adb shell pm list instrumentation 2>/dev/null | grep -i yunjian
INSTRUMENTATION_OK=false
if adb shell pm list instrumentation 2>/dev/null | grep -q "$TEST_PKG"; then
  INSTRUMENTATION_OK=true
fi
measure test_package instrumentation_registered "$INSTRUMENTATION_OK"
if [ "$INSTRUMENTATION_OK" != "true" ]; then
  echo "设备上没有注册这个 instrumentation，am instrument 会找不到它"
  adb shell pm list instrumentation 2>/dev/null | head -20
  mark_all_unavailable instrumentation_not_registered
  exit 0
fi

p "GRANT RECORD_AUDIO BEFORE INSTRUMENTING"
# 运行时权限在这里先给一次。测试类内部还会用 UiAutomation 再授一次——两条路都留着是
# 刻意的：`pm grant` 在部分机型上对已安装包静默失败，而那种失败只表现为判据①拿不到 PCM。
adb shell pm grant "$PKG" android.permission.RECORD_AUDIO 2>&1 | tail -2
adb shell dumpsys package "$PKG" 2>/dev/null | grep -e RECORD_AUDIO -e MODIFY_AUDIO_SETTINGS | head -6

p "CHINESE IME PRECONDITIONS"
adb shell ime enable com.google.android.inputmethod.latin/com.android.inputmethod.latin.LatinIME 2>&1 | tail -1
adb shell ime set com.google.android.inputmethod.latin/com.android.inputmethod.latin.LatinIME 2>&1 | tail -1
DEFAULT_IME=$(adb shell settings get secure default_input_method 2>/dev/null | tr -d '\r\n')
ZH_SUBTYPES=$(adb shell dumpsys input_method 2>/dev/null | grep -c "mSubtypeLocale=zh")
LOCALE=$(adb shell getprop persist.sys.locale 2>/dev/null | tr -d '\r\n')
echo "default_input_method=$DEFAULT_IME zh_subtypes=$ZH_SUBTYPES persist.sys.locale=$LOCALE"
measure chinese_ime default_ime "$DEFAULT_IME"
measure chinese_ime zh_subtype_count "$ZH_SUBTYPES"
measure chinese_ime runtime_locale "$LOCALE"

p "RUN THE THREE INSTRUMENTED CRITERION TESTS"
# 三个类分三次 `am instrument`，刻意不合成一次：判据②要下载 223 MiB 并解压 633 MiB，
# 它超时或崩溃时不该把判据①③的测量一起带走。`-e class` 精确指定，避免把将来新增的
# 测试类意外拉进这一轮。
#
# `|| true` 是必需的：instrument 的退出码反映的是**测试是否通过**，而我们要的是它
# **写下的测量值**。一个测不到 RMS 的判据①会以非零退出，但它写下的
# `MEASURE-UNAVAILABLE rms` 恰恰是我们要收的东西——让它中断脚本会连判据③一起丢掉。
adb logcat -c 2>/dev/null || true
for CLASS in SpikeMicrophoneTest SpikeCorpusTest SpikeImeTest; do
  printf '\n----- am instrument %s -----\n' "$CLASS"
  # 判据②要下载并解压，`-w` 会一直等到它结束；上面的 Rust 侧自带 600 秒预算，
  # 所以这里不再另加超时，否则两个超时哪个先到会变成不可预期的事。
  adb shell am instrument -w -r \
    -e class "$CLASS_PREFIX.$CLASS" \
    -e debug false \
    "$RUNNER" 2>&1 | tail -40 || true
done

p "COLLECT MEASUREMENTS WRITTEN BY THE DEVICE"
# 三条通道按可靠性排序取第一个拿到的：应用私有文件最全，外部缓存最好取，logcat 兜底。
# 判据②那一轮会刷掉大量日志，logcat 环形缓冲很可能已经把早期的测量行冲掉了，
# 所以 logcat 只是兜底而不是依据——这一点写在这里，避免下次有人以为三者等价。
MEASURE_FILE="$DEVICEFARM_LOG_DIR/device-measurements.txt"
: > "$MEASURE_FILE"
if adb shell "run-as $PKG cat files/yunjian-measure.log" 2>/dev/null | grep -q 'YUNJIAN-MEASURE'; then
  echo "source=app_private_files"
  adb shell "run-as $PKG cat files/yunjian-measure.log" 2>/dev/null | tr -d '\r' >> "$MEASURE_FILE"
elif adb shell "cat /sdcard/Android/data/$PKG/cache/yunjian-measure.log" 2>/dev/null | grep -q 'YUNJIAN-MEASURE'; then
  echo "source=external_cache"
  adb shell "cat /sdcard/Android/data/$PKG/cache/yunjian-measure.log" 2>/dev/null | tr -d '\r' >> "$MEASURE_FILE"
else
  echo "source=logcat_fallback"
  adb logcat -d -s YunjianSpike:I 2>/dev/null | tr -d '\r' >> "$MEASURE_FILE"
fi
wc -l "$MEASURE_FILE"

p "DEVICE MEASUREMENTS (verbatim; this is what xtask parses)"
# 原样打印。宿主侧 `xtask` 解析的就是这段输出，所以不能在这里过滤或改写任何一行。
grep -e 'YUNJIAN-MEASURE' -e '^#' "$MEASURE_FILE" || echo "设备端没写下任何测量行"

p "APP UNDER TEST: DOES IT LAUNCH (evidence, not a criterion)"
# 主界面仍然照测。它挂掉不影响三条判据（测试类不依赖它），但这是那个已定位、
# 刻意未修的启动缺陷的持续证据。
adb logcat -c 2>/dev/null || true
adb shell monkey -p "$PKG" -c android.intent.category.LAUNCHER 1 2>&1 | tail -3
sleep 12
PID=$(adb shell pidof "$PKG" 2>/dev/null | tr -d '\r\n')
LAUNCHED=false
if [ -n "$PID" ]; then
  LAUNCHED=true
fi
measure app_launch process_alive "$LAUNCHED"
measure app_launch pid "${PID:-none}"
CRASHED=false
if adb logcat -d 2>/dev/null | grep -q -e "FATAL EXCEPTION" -e "beginning of crash" -e "android context was not initialized"; then
  CRASHED=true
fi
measure app_launch crashed "$CRASHED"
adb logcat -d 2>/dev/null | grep -i -e "FATAL EXCEPTION" -e "beginning of crash" -e "AndroidRuntime" -e "yunjian" -e "ndk-context" -e "Could not connect" | head -30

p "SCREENSHOT (evidence, not a verdict)"
adb shell screencap -p /sdcard/yunjian-launch.png 2>/dev/null && \
  adb pull /sdcard/yunjian-launch.png "$DEVICEFARM_LOG_DIR/yunjian-launch.png" 2>&1 | tail -1

p "MEASUREMENT COMPLETE"
