#!/usr/bin/env bash
# 构建 `yunjian-mobile` 的 iOS 静态库并打成 xcframework。
#
# ## 这是 Android 侧 `cargoNdkBuild` 的对应物
#
# Android：`cargo ndk` 交叉编译出 `libyunjian_mobile.so` 与它的两个 `NEEDED` 依赖，摆进
# `jniLibs/<abi>/`。iOS：编译出 `libyunjian_mobile.a`，与 UniFFI 的 C 头 + modulemap 一起
# 打成 `YunjianMobileFFI.xcframework`，由 Xcode 链接。
#
# 两侧同一条约定：**缺工具链时失败并指名缺什么**，而不是产出一个装上去才发现找不到符号的包。
#
# ## 为什么用 `cargo rustc --crate-type staticlib` 而不是 `cargo build`
#
# `crates/yunjian-mobile/Cargo.toml` 的 `[lib] crate-type` 是 `["staticlib", "cdylib", "rlib"]`。
# `cargo build` 会把三种都产一遍，其中 cdylib 那条在 iOS 上另有已记录的上游阻塞
# （见 `.omo/notepads/yunjian/decisions.md` 与 `mobile/device-farm.toml` 的 `ios_full`）。
# iOS 应用需要的只有静态库——显式只要它，可以少踩一条与本目标无关的路。
#
# ## 语音默认打开
#
# 与 Android 的 `-Pyunjian.voice`（默认 true）一致：todo 69 原文
# 「Voice ships on mobile in both branches」。要一份 MIT 构建用 `YUNJIAN_VOICE=0`，
# 那时 `startAsr` 返回「当前原生库未启用 native-voice」而不是静默降级。
#
# ## 未在本机验证
#
# 本脚本需要 macOS + Xcode 才能跑到底（`xcodebuild -create-xcframework`、
# `aarch64-apple-ios` 的链接都要 Apple 工具链）。仓库所在主机是 Linux，因此**本脚本从未被
# 完整执行过**；它的每一步都写明了依据，但不要当成已验证过的产物。见 `mobile/ios/README.md`。
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IOS_DIR="$(dirname "$HERE")"
REPO_ROOT="$(cd "$IOS_DIR/../.." && pwd)"
BUILD_DIR="$IOS_DIR/build"
BINDINGS="$REPO_ROOT/crates/yunjian-mobile/bindings/generated"

PROFILE="${YUNJIAN_PROFILE:-release}"
WITH_VOICE="${YUNJIAN_VOICE:-1}"
# 设备 + 模拟器两片。真机验收只需 `aarch64-apple-ios`；带上模拟器片是为了让开发机能跑起来，
# 与 Android 的 `-Pyunjian.abis` 默认只出 arm64 而发布出四个 ABI 同一个取舍。
TARGETS="${YUNJIAN_IOS_TARGETS:-aarch64-apple-ios aarch64-apple-ios-sim}"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "本脚本需要 macOS：xcodebuild -create-xcframework 与 iOS 链接都只在 Apple 工具链上可用" >&2
  echo "当前系统：$(uname -s)" >&2
  exit 2
fi

for tool in cargo rustup xcodebuild; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "缺少 $tool；无法构建 iOS 原生库" >&2
    exit 2
  }
done

FEATURES="uniffi"
if [ "$WITH_VOICE" = "1" ]; then
  FEATURES="uniffi,native-voice"
fi
echo "== features=$FEATURES profile=$PROFILE targets=$TARGETS =="

rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR/include"

# UniFFI 的 C 头与 modulemap 直接取生成物，不复制成第二份维护：Swift 侧
# `#if canImport(YunjianMobileFFI)` 找的就是这个模块名。
cp "$BINDINGS/YunjianMobileFFI.h" "$BUILD_DIR/include/"
# xcframework 的头目录要求 modulemap 叫 `module.modulemap`；内容原样取生成的那份。
cp "$BINDINGS/YunjianMobileFFI.modulemap" "$BUILD_DIR/include/module.modulemap"

LIBRARY_ARGS=()
for target in $TARGETS; do
  rustup target list --installed | grep -qx "$target" || {
    echo "未安装 Rust target $target；执行 rustup target add $target" >&2
    exit 2
  }
  echo "-- cargo rustc --target $target"
  (
    cd "$REPO_ROOT"
    cargo rustc \
      -p yunjian-mobile \
      --lib \
      --crate-type staticlib \
      --features "$FEATURES" \
      --profile "$PROFILE" \
      --target "$target"
  )
  archive="$REPO_ROOT/target/$target/$PROFILE/libyunjian_mobile.a"
  [ -f "$archive" ] || {
    echo "构建成功但没找到 $archive；请核对 crate-type 与 profile" >&2
    exit 1
  }
  slice="$BUILD_DIR/$target"
  mkdir -p "$slice"
  cp "$archive" "$slice/libyunjian_mobile.a"
  LIBRARY_ARGS+=(-library "$slice/libyunjian_mobile.a" -headers "$BUILD_DIR/include")
done

xcodebuild -create-xcframework \
  "${LIBRARY_ARGS[@]}" \
  -output "$BUILD_DIR/YunjianMobileFFI.xcframework"

echo "== 产出 $BUILD_DIR/YunjianMobileFFI.xcframework =="
