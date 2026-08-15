# 移动端分发与体积报告

> [!WARNING]
> **`all_pass = false`；binding verdict 为 `undetermined`。**
> Android/iOS 构建、真实体积与设备 smoke 没有产物或物理证据时一律是 `NOT EXECUTED`，不是 PASS。

- 日期：`2026-08-15`
- 提交：`f19fcb7ec070c3a9627909f0c8abd19087336059`
- APK 上限：`83886080` bytes（80 MiB / split APK）
- universal APK 默认分发：`false`

## 构建命令

- `android`: `cargo tauri android build --ci --split-per-abi --apk --aab --target aarch64 armv7 i686 x86_64`
- `ios`: `cargo tauri ios build --ci --export-method app-store-connect`

## 工具探测

| 工具                                                                 | 可用    | 依据                                                                            |
| -------------------------------------------------------------------- | ------- | ------------------------------------------------------------------------------- |
| `adb`                                                                | `false` | No such file or directory (os error 2)                                          |
| `gradle`                                                             | `false` | No such file or directory (os error 2)                                          |
| `sdkmanager`                                                         | `false` | exit=Some(1); stdout=; stderr=mise ERROR No version is set for shim: sdkmanager |
| Set a global default version with one of the following:              |
| mise use -g android-sdk@1.0                                          |
| mise ERROR Version: 2026.7.7 linux-x64 (2026-07-15)                  |
| mise ERROR Run with --verbose or MISE_VERBOSE=1 for more information |
| `tauri_cli`                                                          | `true`  | exit=Some(0); stdout=tauri-cli 2.11.4; stderr=                                  |
| `xcrun`                                                              | `false` | No such file or directory (os error 2)                                          |

## 逐条裁决

| 断言                        | 裁决             | 依据 / 可执行条件                                                                                                                                                                                                 |
| --------------------------- | ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `android_per_abi_apks`      | **NOT EXECUTED** | 没有真实 APK，未执行四 ABI 完整性判断<br>**可执行条件**：提供完整 Android SDK/NDK 与已选 binding，执行带 --split-per-abi --apk 的构建命令                                                                         |
| `android_aab`               | **NOT EXECUTED** | 没有真实构建产物<br>**可执行条件**：在有完整 Android SDK/NDK 与已选 binding 的 runner 上执行 distribution.toml 的 Android 命令                                                                                    |
| `ios_archive`               | **NOT EXECUTED** | 没有真实构建产物<br>**可执行条件**：在安装 Xcode 26 / iOS 26 SDK 且配置 Distribution 证书与 provisioning profile 的 macOS 上执行 distribution.toml 的 iOS 命令                                                    |
| `artifact_sizes_measured`   | **NOT EXECUTED** | 产物集不完整：APK 0 / 4，AAB 0 / 1，iOS archive 0 / 1；未用估算补空值<br>**可执行条件**：在 Android 与 macOS 签名 runner 上完成构建，把全部产物下载到同一目录后重跑                                               |
| `apk_ceiling`               | **NOT EXECUTED** | 没有 APK，无法测量上限<br>**可执行条件**：构建四个 split release APK 后重跑；上限固定为 80 MiB/个                                                                                                                 |
| `packaged_assets`           | **NOT EXECUTED** | 没有 APK/AAB/IPA，未执行 ZIP 内容扫描<br>**可执行条件**：提供真实 APK/AAB/IPA；守卫会读取 ZIP 中央目录并拒绝 corpus .db 与语音模型                                                                                |
| `instrumented_device_smoke` | **NOT EXECUTED** | 没有物理设备 instrumented smoke 观测；未用 jsdom、模拟器或宿主结果顶替<br>**可执行条件**：连接已授权物理设备，运行 instrumented test 完成两字检索、打字背诵一轮、语音会话启动与停止，并通过 --smoke-json 提交观测 |

## 实测产物

没有真实产物；不记录估算体积。

## 商店提交

1. Play 首次上传必须手工在 Play Console 建立应用与 internal testing track，上传 AAB、完成内容与数据安全声明；后续自动化才能复用该应用身份。
2. APK 只作为逐 ABI 测试/旁载产物，默认发布不生成 universal APK；Play 分发使用 AAB。
3. iOS 在有 Xcode 26 / iOS 26 SDK 与签名身份的 macOS 上 archive，再通过 App Store Connect / TestFlight 上传。
4. corpus 与语音模型均按需下载并校验，不进入商店初始包；ZIP 资产守卫把这条架构约束变成失败条件。
