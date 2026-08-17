# 移动端分发与体积报告

> [!WARNING]
> **`all_pass = false`；binding verdict 为 `uniffi_native`。**
> Android/iOS 构建、真实体积与设备 smoke 没有产物或物理证据时一律是 `NOT EXECUTED`，不是 PASS。

- 日期：`2026-08-17`
- 提交：`8a589760263cf7edf13dca3a91d7f3baed7e270b`
- APK 上限：`83886080` bytes（80 MiB / split APK）
- universal APK 默认分发：`false`

## 构建命令

- `android`: `cd mobile/android && for abi in arm64-v8a armeabi-v7a x86 x86_64; do gradle -Pyunjian.abis=$abi :app:assembleRelease; done && gradle -Pyunjian.abis=arm64-v8a,armeabi-v7a,x86,x86_64 :app:bundleRelease`
- `ios`: `cd mobile/ios && xcodegen generate && xcodebuild archive -scheme Yunjian -destination 'generic/platform=iOS' -archivePath build/Yunjian.xcarchive && xcodebuild -exportArchive -archivePath build/Yunjian.xcarchive -exportOptionsPlist ExportOptions.plist -exportPath build/export`

## 工具探测

| 工具                                                                 | 可用    | 依据                                                                            |
| -------------------------------------------------------------------- | ------- | ------------------------------------------------------------------------------- |
| `adb`                                                                | `false` | No such file or directory (os error 2)                                          |
| `gradle`                                                             | `false` | exit=Some(1); stdout=; stderr=mise ERROR No version is set for shim: gradle     |
| Set a global default version with one of the following:              |
| mise use -g gradle@8.13.0                                            |
| mise ERROR Version: 2026.7.7 linux-x64 (2026-07-15)                  |
| mise ERROR Run with --verbose or MISE_VERBOSE=1 for more information |
| `sdkmanager`                                                         | `false` | exit=Some(1); stdout=; stderr=mise ERROR No version is set for shim: sdkmanager |
| Set a global default version with one of the following:              |
| mise use -g android-sdk@1.0                                          |
| mise ERROR Version: 2026.7.7 linux-x64 (2026-07-15)                  |
| mise ERROR Run with --verbose or MISE_VERBOSE=1 for more information |
| `tauri_cli`                                                          | `true`  | exit=Some(0); stdout=tauri-cli 2.11.4; stderr=                                  |
| `xcrun`                                                              | `false` | No such file or directory (os error 2)                                          |

## 逐条裁决

| 断言                        | 裁决             | 依据 / 可执行条件                                                                                                                                                                                                                                                                                                                                                             |
| --------------------------- | ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `android_per_abi_apks`      | **FAIL**         | 要求四个单 ABI APK 且禁 universal；实际 3 个（arm64-v8a, armeabi-v7a, x86_64），universal=false；缺 x86；x86 的 i686-linux-android 是否有上游 sherpa-onnx 预编译产物见 crates/yunjian-voice/build.rs 的 PREBUILT_TARGETS，没有时带 voice 的该 ABI 产物需源码编译 sherpa-onnx（cmake >= 3.13 与 C++17），这是分发 ABI 集与「移动端默认开 voice」两个决定之间的冲突，需用户裁量 |
| `android_aab`               | **PASS**         | 产物存在：target/mobile/dist/yunjian-release.aab（86503972 bytes）                                                                                                                                                                                                                                                                                                            |
| `ios_archive`               | **NOT EXECUTED** | 没有真实构建产物<br>**可执行条件**：在安装 Xcode 26 / iOS 26 SDK 且配置 Distribution 证书与 provisioning profile 的 macOS 上执行 distribution.toml 的 iOS 命令                                                                                                                                                                                                                |
| `artifact_sizes_measured`   | **NOT EXECUTED** | 产物集不完整：APK 3 / 4，AAB 1 / 1，iOS archive 0 / 1；未用估算补空值<br>**可执行条件**：在 Android 与 macOS 签名 runner 上完成构建，把全部产物下载到同一目录后重跑                                                                                                                                                                                                           |
| `apk_ceiling`               | **PASS**         | 3 个 APK 均不超过 83886080 bytes                                                                                                                                                                                                                                                                                                                                              |
| `packaged_assets`           | **PASS**         | 扫描 4 个 ZIP 产物，未发现 corpus .db 或语音模型                                                                                                                                                                                                                                                                                                                              |
| `instrumented_device_smoke` | **PASS**         | Google Pixel 8 / 15/35：检索、打字背诵、语音 start-stop 均通过                                                                                                                                                                                                                                                                                                                |

## 实测产物

| 类型  | ABI           |     字节 | ZIP 条目 | 路径                                                 |
| ----- | ------------- | -------: | -------: | ---------------------------------------------------- |
| `Apk` | `arm64-v8a`   | 42699280 |      115 | `target/mobile/dist/yunjian-arm64-v8a-release.apk`   |
| `Apk` | `armeabi-v7a` | 30656370 |      115 | `target/mobile/dist/yunjian-armeabi-v7a-release.apk` |
| `Aab` | `—`           | 86503972 |      153 | `target/mobile/dist/yunjian-release.aab`             |
| `Apk` | `x86_64`      | 46852187 |      115 | `target/mobile/dist/yunjian-x86_64-release.apk`      |

## 商店提交

1. Play 首次上传必须手工在 Play Console 建立应用与 internal testing track，上传 AAB、完成内容与数据安全声明；后续自动化才能复用该应用身份。
2. APK 只作为逐 ABI 测试/旁载产物，默认发布不生成 universal APK；Play 分发使用 AAB。
3. iOS 在有 Xcode 26 / iOS 26 SDK 与签名身份的 macOS 上 archive，再通过 App Store Connect / TestFlight 上传。
4. corpus 与语音模型均按需下载并校验，不进入商店初始包；ZIP 资产守卫把这条架构约束变成失败条件。
