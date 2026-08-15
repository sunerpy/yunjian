#!/bin/sh
# yunjian Device Farm capability probe - read-only device interrogation.
p() { printf '\n===== %s =====\n' "$1"; }

p "DEVICE IDENTITY"
adb shell getprop ro.product.manufacturer 2>&1
adb shell getprop ro.product.model 2>&1
adb shell getprop ro.build.version.release 2>&1
adb shell getprop ro.build.version.sdk 2>&1
adb shell getprop ro.build.fingerprint 2>&1

p "MICROPHONE HARDWARE FEATURE"
adb shell pm list features 2>&1 | grep -i -e microphone -e audio

p "AUDIO INPUT DEVICES (dumpsys audio)"
adb shell dumpsys audio 2>&1 | grep -i -e "mic" -e "input device" -e "Muted" | head -40

p "AUDIO FLINGER INPUT THREADS"
adb shell dumpsys media.audio_flinger 2>&1 | grep -i -e "Input thread" -e "AudioStreamIn" -e "sample rate" | head -30

p "SHELL UID RECORD_AUDIO APPOP"
adb shell appops get 2000 RECORD_AUDIO 2>&1
adb shell cmd appops get 2000 android:record_audio 2>&1

p "LOCALE (expect the run configuration override)"
adb shell getprop persist.sys.locale 2>&1
adb shell getprop ro.product.locale 2>&1
adb shell settings get system system_locales 2>&1

p "INPUT METHODS - ALL (enabled and disabled)"
adb shell ime list -a -s 2>&1

p "INPUT METHODS - FULL DETAIL"
adb shell ime list -a 2>&1 | head -60

p "CURRENT DEFAULT IME"
adb shell settings get secure default_input_method 2>&1
adb shell settings get secure enabled_input_methods 2>&1

p "GBOARD / IME PACKAGES PRESENT"
adb shell pm list packages 2>&1 | grep -i -e inputmethod -e keyboard -e latin -e pinyin -e sogou -e baidu

p "GBOARD CHINESE SUPPORT (subtype dump)"
adb shell dumpsys input_method 2>&1 | grep -i -e "zh" -e "pinyin" -e "subtype" | head -40

p "FREE STORAGE (corpus materialization headroom)"
adb shell df -h /data /sdcard 2>&1

p "CPU / MEMORY (mid-range classification)"
adb shell getprop ro.soc.model 2>&1
adb shell cat /proc/meminfo 2>&1 | head -3

p "NETWORK EGRESS (production corpus download path)"
adb shell ping -c 2 -W 3 8.8.8.8 2>&1 | tail -3

p "HOST TOOLING"
adb version 2>&1 | head -2
echo "DEVICEFARM_DEVICE_NAME=$DEVICEFARM_DEVICE_NAME"
echo "DEVICEFARM_DEVICE_OS_VERSION=$DEVICEFARM_DEVICE_OS_VERSION"
echo "DEVICEFARM_DEVICE_PLATFORM_NAME=$DEVICEFARM_DEVICE_PLATFORM_NAME"
echo "DEVICEFARM_DEVICE_UDID=$DEVICEFARM_DEVICE_UDID"

p "PROBE COMPLETE"
