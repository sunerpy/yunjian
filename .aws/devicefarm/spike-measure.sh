#!/bin/sh
# yunjian Device Farm 判据测量 —— 宿主侧（adb）可测部分。
#
# 这个脚本刻意**只测它真能测到的东西**，并对测不到的项打印显式的
# `MEASURE-UNAVAILABLE` 行，而不是留空让读者以为漏跑了。
#
# 为什么有些项宿主侧测不到（PR #99 已在真机上实测确认，不是推测）：
#   - 设备上没有 tinycap / tinyplay / tinymix / arecord，`/system/bin/tiny*` 不存在，
#     所以 PCM 采集只能由 APK 内的 `AudioRecord` 完成，adb 拿不到样本。
#   - Device Farm 的 `ScheduleRun.configuration` 只有 locale / location / radios /
#     extraDataPackageArn / auxiliaryApps，**没有音频注入**，因此即便能采到，
#     信号也只能是机架环境噪声。
#   - 语料物化要走生产下载路径，那条路径在应用进程内，adb 无法代它执行。
#   - 键盘遮挡与 visualViewport 是 WebView 内的量，必须由应用侧上报。
#
# 每一行以 `YUNJIAN-MEASURE ` 开头的输出都是**机器可读测量值**，供宿主侧
# `xtask acceptance --platform android --set spike` 解析成 verdict。
# 格式：`YUNJIAN-MEASURE <criterion_id> <key>=<value>`。

PKG=top.onethinker.yunjian

p() { printf '\n===== %s =====\n' "$1"; }
measure() { printf 'YUNJIAN-MEASURE %s %s=%s\n' "$1" "$2" "$3"; }
unavailable() { printf 'YUNJIAN-MEASURE-UNAVAILABLE %s %s reason=%s\n' "$1" "$2" "$3"; }

p "DEVICE IDENTITY"
MODEL=$(adb shell getprop ro.product.model 2>/dev/null | tr -d '\r\n')
RELEASE=$(adb shell getprop ro.build.version.release 2>/dev/null | tr -d '\r\n')
SDK=$(adb shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r\n')
FINGERPRINT=$(adb shell getprop ro.build.fingerprint 2>/dev/null | tr -d '\r\n')
echo "model=$MODEL release=$RELEASE sdk=$SDK"
echo "fingerprint=$FINGERPRINT"

p "CRITERION 0 - IS YUNJIAN ACTUALLY INSTALLED"
# 这是本轮相对 PR #99 唯一真正新增的事实：那两次探针用的是 AWS 自带的
# DefaultAndroidApplicationForDeviceFarm.apk，云笺从未装到真机上。
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
  exit 1
fi

p "INSTALLED PACKAGE DETAIL (targetSdk / versionName / apk path)"
adb shell dumpsys package "$PKG" 2>/dev/null | grep -e targetSdk -e versionName -e versionCode -e codePath -e firstInstallTime | head -12
TARGET_SDK=$(adb shell dumpsys package "$PKG" 2>/dev/null | sed -n 's/.*targetSdk=\([0-9]*\).*/\1/p' | head -1 | tr -d '\r\n')
VERSION_NAME=$(adb shell dumpsys package "$PKG" 2>/dev/null | sed -n 's/.*versionName=\([^ ]*\).*/\1/p' | head -1 | tr -d '\r\n')
echo "parsed target_sdk=$TARGET_SDK version_name=$VERSION_NAME"

p "LAUNCH THE APP AND SEE IF IT STAYS UP"
adb logcat -c 2>/dev/null || true
adb shell monkey -p "$PKG" -c android.intent.category.LAUNCHER 1 2>&1 | tail -3
sleep 12
PID=$(adb shell pidof "$PKG" 2>/dev/null | tr -d '\r\n')
echo "pid after launch: [$PID]"
LAUNCHED=false
if [ -n "$PID" ]; then
  LAUNCHED=true
fi
measure app_launch process_alive "$LAUNCHED"
measure app_launch pid "${PID:-none}"

p "FOREGROUND ACTIVITY / WINDOW"
adb shell dumpsys activity activities 2>/dev/null | grep -i -e "mResumedActivity" -e "topResumedActivity" -e "$PKG" | head -10
adb shell dumpsys window windows 2>/dev/null | grep -i -e "mCurrentFocus" -e "mFocusedApp" | head -5

p "CRASH / WEBVIEW EVIDENCE FROM LOGCAT"
# 白屏这一类故障在真机上比桌面更难看出来，所以把 WebView 与自家日志一起捞出来。
# 判据不建立在这些行上，它们是给人读的证据。
adb logcat -d 2>/dev/null | grep -i -e "FATAL EXCEPTION" -e "beginning of crash" -e "AndroidRuntime" -e "yunjian" -e "Tauri" -e "chromium" -e "WebView" -e "Could not connect" | head -40
CRASHED=false
if adb logcat -d 2>/dev/null | grep -q -e "FATAL EXCEPTION" -e "beginning of crash"; then
  CRASHED=true
fi
measure app_launch crashed "$CRASHED"

p "SCREENSHOT (evidence, not a verdict)"
# 「数字是结论，图是证据」——PR #100 的教训是我们曾反复引用一个颜色占比数字，
# 却从没打开产生那个数字的图。所以这里一定要把图留成 artifact。
adb shell screencap -p /sdcard/yunjian-launch.png 2>/dev/null && \
  adb pull /sdcard/yunjian-launch.png "$DEVICEFARM_LOG_DIR/yunjian-launch.png" 2>&1 | tail -1

p "CRITERION 3 - CHINESE IME"
adb shell ime enable com.google.android.inputmethod.latin/com.android.inputmethod.latin.LatinIME 2>&1 | tail -1
adb shell ime set com.google.android.inputmethod.latin/com.android.inputmethod.latin.LatinIME 2>&1 | tail -1
DEFAULT_IME=$(adb shell settings get secure default_input_method 2>/dev/null | tr -d '\r\n')
echo "default_input_method=$DEFAULT_IME"
ZH_SUBTYPES=$(adb shell dumpsys input_method 2>/dev/null | grep -c "mSubtypeLocale=zh")
echo "zh subtype count=$ZH_SUBTYPES"
LOCALE=$(adb shell getprop persist.sys.locale 2>/dev/null | tr -d '\r\n')
echo "persist.sys.locale=$LOCALE"
measure chinese_ime device_model "$MODEL"
measure chinese_ime os_build "$RELEASE/$SDK"
measure chinese_ime target_sdk "${TARGET_SDK:-unknown}"
measure chinese_ime default_ime "$DEFAULT_IME"
measure chinese_ime zh_subtype_count "$ZH_SUBTYPES"
measure chinese_ime runtime_locale "$LOCALE"
# 这三项是 WebView 内部的量，adb 读不到；缺一个判据就不能判 PASS。
unavailable chinese_ime entered_text needs_in_app_instrumentation
unavailable chinese_ime keyboard_overlap_px needs_in_app_instrumentation
unavailable chinese_ime input_visible needs_in_app_instrumentation
unavailable chinese_ime visual_viewport_updated needs_in_app_instrumentation

p "CRITERION 1 - MICROPHONE (hardware only; PCM needs in-app AudioRecord)"
adb shell pm list features 2>/dev/null | grep -i -e microphone -e audio.low_latency
adb shell dumpsys audio 2>/dev/null | grep -i "mic mute" | head -3
MIC_FEATURE=false
if adb shell pm list features 2>/dev/null | grep -q android.hardware.microphone; then
  MIC_FEATURE=true
fi
measure microphone_capture device_model "$MODEL"
measure microphone_capture os_build "$RELEASE/$SDK"
measure microphone_capture hardware_present "$MIC_FEATURE"
unavailable microphone_capture sample_rate_hz needs_in_app_audiorecord
unavailable microphone_capture channel_count needs_in_app_audiorecord
unavailable microphone_capture rms needs_in_app_audiorecord_and_device_farm_has_no_audio_injection
unavailable microphone_capture permission_plugin needs_in_app_instrumentation

p "CRITERION 2 - CORPUS MATERIALIZATION (headroom only; path is in-process)"
adb shell df -h /data 2>/dev/null | tail -2
adb shell ping -c 2 -W 3 8.8.8.8 2>/dev/null | tail -3
measure corpus_materialization device_model "$MODEL"
measure corpus_materialization os_build "$RELEASE/$SDK"
unavailable corpus_materialization artifact_bytes needs_in_app_production_fetch
unavailable corpus_materialization sha256_verified needs_in_app_production_fetch
unavailable corpus_materialization duration_seconds needs_in_app_production_fetch
unavailable corpus_materialization atomic_install needs_in_app_production_fetch
unavailable corpus_materialization crashed needs_in_app_production_fetch
unavailable corpus_materialization production_path needs_in_app_production_fetch

p "PROBE COMPLETE"
