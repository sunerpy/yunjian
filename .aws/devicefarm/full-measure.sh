#!/bin/sh
# 云笺 todo 71 真机验收 —— 设备侧编排。
#
# ## 与 spike 那套的关系
#
# `spike-measure.sh` 量的是**三条框架选型判据**（麦克风能不能采、语料能不能物化、
# 中文 IME 能不能用），跑的是 `mobile/android/spike/androidTest` 里三个不依赖主界面的类。
# 本脚本量的是**十条产品验收断言**，跑的是 `mobile/android/app/src/androidTest` 里的
# `FullAcceptanceTest`，而那个类**必须**拉起真界面。
#
# 可复用的是编排：公共设备重签的应对、测试包安装的 `-t` 开关、测量文件的三通道回收。
# 这几件都是真机上实测得来的，照抄比重新踩一遍便宜。
#
# ## 仍然不在设备侧下结论
#
# 本脚本只搬测量值。PASS / FAIL / NOT EXECUTED 由宿主侧
# `xtask acceptance --platform android --set full` 判定，因为判词、阈值与报告 schema
# 都在那里。让设备侧下结论等于把门禁搬进被测物内部。

PKG=top.onethinker.yunjian
TEST_PKG="$PKG.test"
RUNNER="$TEST_PKG/androidx.test.runner.AndroidJUnitRunner"
TEST_APK=yunjian-androidTest.apk
APP_APK=yunjian.apk
TEST_CLASS=top.onethinker.yunjian.FullAcceptanceTest
MEASURE_FILE=yunjian-acceptance.log

# 十条断言的 id。顺序与 `xtask` 的 `FULL_DECLARED` 一致。
ASSERTIONS="install_and_launch corpus_first_run_materialization two_char_search_returns_results reading_view_citations_and_ai_appreciation typed_recitation_scores_correctly voice_recitation_round_succeeds_end_to_end voice_permission_denied_degrades chinese_ime_prefilled_field_visible background_return_preserves_layout app_exits_cleanly"

p() { printf '\n===== %s =====\n' "$1"; }
measure() { printf 'YUNJIAN-FULL %s %s=%s\n' "$1" "$2" "$3"; }

# 某一步导致**全部十条**都测不到时逐条标注。集中在这里，因为「装不上应用」与
# 「instrument 挂了」要标注的是同一组断言，重复几遍容易漏项。
mark_all_unavailable() {
  REASON="$1"
  for A in $ASSERTIONS; do
    printf 'YUNJIAN-FULL %s harness_unavailable=%s\n' "$A" "$REASON"
  done
}

p "DEVICE IDENTITY"
MODEL=$(adb shell getprop ro.product.model 2>/dev/null | tr -d '\r\n')
RELEASE=$(adb shell getprop ro.build.version.release 2>/dev/null | tr -d '\r\n')
SDK=$(adb shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r\n')
FINGERPRINT=$(adb shell getprop ro.build.fingerprint 2>/dev/null | tr -d '\r\n')
HARDWARE=$(adb shell getprop ro.hardware 2>/dev/null | tr -d '\r\n')
QEMU=$(adb shell getprop ro.kernel.qemu 2>/dev/null | tr -d '\r\n')
echo "model=$MODEL release=$RELEASE sdk=$SDK hardware=$HARDWARE qemu=[$QEMU]"
echo "fingerprint=$FINGERPRINT"
# 这四项是宿主侧「是不是物理设备」的判据来源。模拟器给 goldfish / ranchu / generic。
measure device_identity model "$MODEL"
measure device_identity os_build "$RELEASE/$SDK"
measure device_identity ro_hardware "$HARDWARE"
measure device_identity ro_kernel_qemu "${QEMU:-unset}"
measure device_identity fingerprint "$FINGERPRINT"

p "REPLACE THE DEVICE-FARM-RESIGNED COPY WITH OURS"
# Device Farm 对**公共**设备一律重签（`skipAppResign` 只对私有设备有效）。重签后应用
# 带的是它自己的证书，而测试包 zip 里的 APK 不经重签，还是我们的调试证书。两者不一致时
# `am instrument` 报 `does not have a signature matching the target`——PR #103 第一轮
# 三个类全 SecurityException 就是这个。构建期那道「两个 APK 同签名」断言拦不住它：
# 重签发生在上传之后。
if [ ! -f "$APP_APK" ]; then
  echo "测试包里没有应用 APK：$APP_APK"
  ls -la
  mark_all_unavailable app_apk_missing_from_test_bundle
  exit 0
fi
adb uninstall "$PKG" 2>&1 | tail -2
# `-t` 允许 testOnly；`-g` 预授运行时权限（`voice_permission_denied_degrades`
# 会自己 `pm revoke` 再测，所以预授不会掩盖那条断言）。
adb install -r -t -g "$APP_APK" 2>&1 | tail -5
if ! adb shell pm list packages 2>/dev/null | grep -q "$PKG"; then
  echo "自装应用 APK 失败"
  mark_all_unavailable self_install_of_app_apk_failed
  exit 0
fi
measure harness app_self_installed true

if [ ! -f "$TEST_APK" ]; then
  echo "测试包里没有 androidTest APK：$TEST_APK"
  ls -la
  mark_all_unavailable androidtest_apk_missing_from_test_bundle
  exit 0
fi
adb install -r -t -g "$TEST_APK" 2>&1 | tail -5
if ! adb shell pm list packages 2>/dev/null | grep -q "$TEST_PKG"; then
  echo "装测试包失败"
  mark_all_unavailable androidtest_apk_install_failed
  exit 0
fi
measure harness test_self_installed true

if ! adb shell pm list instrumentation 2>/dev/null | grep -q "$TEST_PKG"; then
  echo "instrumentation 未注册"
  adb shell pm list instrumentation 2>/dev/null | head -10
  mark_all_unavailable instrumentation_not_registered
  exit 0
fi
measure harness instrumentation_registered true

p "TARGET SDK / VERSION"
adb shell dumpsys package "$PKG" 2>/dev/null | grep -e targetSdk -e versionName -e codePath | head -6
TARGET_SDK=$(adb shell dumpsys package "$PKG" 2>/dev/null | sed -n 's/.*targetSdk=\([0-9]*\).*/\1/p' | head -1 | tr -d '\r\n')
measure harness target_sdk "${TARGET_SDK:-unknown}"

p "ASR WEIGHTS ARE FETCHED BY THE APP ITSELF"
# **刻意不在这里推权重。** 两条外部塞文件的路都试过并失败：
#
#   - 推进内部 `files/yunjian/models/`（第九轮）：`run-as mkdir` 破坏了应用在同一棵树下
#     的写权限（探针 `app_can_create_siblings=false`），语料物化随后报
#     `database is locked` 与 `unable to open database file`。
#   - 推进外部 `/sdcard/Android/data/<pkg>/files/`（第十轮）：`adb push` 落下的文件属主是
#     `shell`，应用（另一个 uid）读不到，`isDirectory` 报 `false`——看起来像没推上去。
#
# 现在由**产品自己**那条按需下载路径取权重（`ModelCache::ensure`：下载 + SHA-256 +
# 原子解包），文件属主就是应用。这同时把「按需下载在真机上可用」一并测到了，
# 而那正是产品的真实形态（安装包不含任何权重）。
echo "权重由应用经 fetch_voice_model 下载；此处不做任何推送"

p "RUN THE TEN ASSERTIONS"
adb logcat -c 2>/dev/null || true
# 一次 `am instrument` 跑完整个类：十条断言之间有真实的先后依赖（语料没物化就搜不到
# 东西），分成十次调用会让每次都从头等一遍物化，而物化在真机上要分钟级。
# `-e class` 只给类名不给方法名，runner 按方法名字典序执行，`t01`..`t10` 前缀即执行顺序。
#
# **不加 `|| exit`**：某个方法失败时后面的测量仍要收，而收测量正是判断
# 「产品坏了」还是「这一项没测到」所需要的。
adb shell am instrument -w -r \
  -e class "$TEST_CLASS" \
  -e debug false \
  "$RUNNER" 2>&1 | tail -80

p "COLLECT DEVICE-SIDE MEASUREMENTS"
# 三条通道按可用性依次尝试。理由见 `AcceptanceReport` 的类注释：没有一条在所有配置下
# 都可靠，而漏掉测量等于把一次真实结果变成 NOT EXECUTED。
COLLECTED=false
if adb shell run-as "$PKG" cat "files/$MEASURE_FILE" 2>/dev/null | grep -q 'YUNJIAN-FULL'; then
  echo "--- via run-as ---"
  adb shell run-as "$PKG" cat "files/$MEASURE_FILE" 2>/dev/null
  COLLECTED=true
fi
if [ "$COLLECTED" != "true" ]; then
  EXT="/sdcard/Android/data/$PKG/cache/$MEASURE_FILE"
  if adb shell cat "$EXT" 2>/dev/null | grep -q 'YUNJIAN-FULL'; then
    echo "--- via external cache ---"
    adb shell cat "$EXT" 2>/dev/null
    COLLECTED=true
  fi
fi
if [ "$COLLECTED" != "true" ]; then
  echo "--- via logcat ---"
  adb logcat -d -s YunjianAcceptance:I 2>/dev/null | sed -n 's/.*\(YUNJIAN-FULL .*\)/\1/p'
fi

p "PULL SCREENSHOTS"
# 「数字是结论，图是证据」。截图落在应用外部缓存，这里搬到 Device Farm 的日志目录，
# run 结束后随 artifacts 一起下载。
SHOT_DIR="/sdcard/Android/data/$PKG/cache"
adb shell ls "$SHOT_DIR" 2>/dev/null | tr -d '\r' | while read -r NAME; do
  case "$NAME" in
    *.png)
      adb pull "$SHOT_DIR/$NAME" "$DEVICEFARM_LOG_DIR/$NAME" 2>&1 | tail -1
      SIZE=$(adb shell stat -c %s "$SHOT_DIR/$NAME" 2>/dev/null | tr -d '\r\n')
      printf 'YUNJIAN-FULL screenshots %s=%s\n' "$NAME" "${SIZE:-0}"
      ;;
  esac
done

p "CRASH LOG"
adb logcat -d -b crash 2>/dev/null | grep -i "$PKG" | head -40 || true
