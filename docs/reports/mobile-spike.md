# 移动端可行性实测

> [!WARNING]
> **verdict: `undetermined`。** `NOT EXECUTED` 不是产品失败，也不是通过；
> 在四项真机判据全部得到 PASS/FAIL 之前，移动端框架选型保持 `undetermined`。

## 本次运行

| 项             | 值                                           |
| -------------- | -------------------------------------------- |
| 请求平台       | `ios`                                        |
| 日期           | `2026-08-15`                                 |
| 提交           | `ec6cbc9eb4135b6ebbefd7f51dac530fc83a0cf6`   |
| 宿主 OS        | `Ubuntu 24.04.4 LTS / Linux 6.17.0-1019-aws` |
| 前置探测       | `xcrun devicectl list devices`               |
| 工具可用       | `false`                                      |
| 工具退出码     | `None`                                       |
| 已识别物理设备 | `0`                                          |

### 前置探测原始输出

```text
stdout:

stderr:
No such file or directory (os error 2)
```

## 四项预声明判据

### `microphone_capture` · 经 todo 46 权限插件在物理 Android 与物理 iOS 设备上采集麦克风 PCM

- **verdict**: `NOT EXECUTED`
- **threshold**: Android 与 iOS 均满足 sample_rate_hz == 16000、channel_count == 1、rms > 0
- **driver**: Android: adb + instrumented test APK；iOS: xcrun devicectl + XCUITest bundle
- **detail**: NOT EXECUTED：本轮没有满足 `microphone_capture` 所需的完整物理设备、测试载体与凭据；没有用模拟器或宿主机数据顶替
- **可执行条件**: 同时备妥已授权 USB 调试的物理 Android、已注册到签名身份的物理 iOS、两端已安装的 instrumented 测试包，并授予麦克风权限
- **measurement**:

```json
{
  "channel_count": null,
  "device_model": null,
  "os_build": null,
  "permission_plugin": null,
  "rms": null,
  "sample_rate_hz": null
}
```

### `corpus_materialization` · 在中端物理 Android 上走生产下载路径校验并原子解压发布语料

- **verdict**: `NOT EXECUTED`
- **threshold**: 下载 .db.gz、SHA-256 校验成功、原子落入 app storage，duration_seconds < 60 且 crashed == false
- **driver**: adb + instrumented test APK，调用与生产 corpus fetch 相同的下载、校验、解压和原子替换路径
- **detail**: NOT EXECUTED：本轮没有满足 `corpus_materialization` 所需的完整物理设备、测试载体与凭据；没有用模拟器或宿主机数据顶替
- **可执行条件**: 连接一台已授权 USB 调试的中端物理 Android，安装调用生产语料物化路径的 instrumented test APK，并提供可下载的 .db.gz 发布工件与 SHA-256
- **measurement**:

```json
{
  "artifact_bytes": null,
  "atomic_install": null,
  "crashed": null,
  "device_model": null,
  "duration_seconds": null,
  "os_build": null,
  "production_path": null,
  "sha256_verified": null
}
```

### `chinese_ime` · 在 targetSdk 35 的物理 Android 上用中文输入法向检索框输入中文

- **verdict**: `NOT EXECUTED`
- **threshold**: target_sdk == 35、中文提交成功、keyboard_overlap_px == 0，输入框始终可见且 visualViewport 正常更新
- **driver**: adb + targetSdk 35 instrumented test APK；物理键盘输入法交互由设备端测试记录 viewport 与控件边界
- **detail**: NOT EXECUTED：本轮没有满足 `chinese_ime` 所需的完整物理设备、测试载体与凭据；没有用模拟器或宿主机数据顶替
- **可执行条件**: 连接已授权的物理 Android，安装 targetSdk 35 测试 APK，启用可输入中文的软键盘，并由设备端 instrumentation 记录输入文本、键盘遮挡和 visualViewport
- **measurement**:

```json
{
  "device_model": null,
  "entered_text": null,
  "input_visible": null,
  "keyboard_overlap_px": null,
  "os_build": null,
  "target_sdk": null,
  "visual_viewport_updated": null
}
```

### `ios_testflight_submission` · 用 Xcode 26 / iOS 26 SDK 完成一次真实 archive、链接与 TestFlight 上传

- **verdict**: `NOT EXECUTED`
- **threshold**: xcode_major >= 26、ios_sdk_major >= 26、archive_link_succeeded == true、upload_succeeded == true 且 testflight_build_id 非空
- **driver**: xcrun devicectl + XCUITest bundle + xcodebuild archive + App Store Connect upload
- **detail**: NOT EXECUTED：本轮没有满足 `ios_testflight_submission` 所需的完整物理设备、测试载体与凭据；没有用模拟器或宿主机数据顶替
- **可执行条件**: 在安装 Xcode 26 与 iOS 26 SDK 的 macOS 上连接已注册到签名身份的物理 iOS 设备，配置 Distribution 证书、provisioning profile 与 App Store Connect 上传凭据
- **measurement**:

```json
{
  "archive_link_succeeded": null,
  "device_model": null,
  "ios_sdk_version": null,
  "os_build": null,
  "testflight_build_id": null,
  "upload_succeeded": null,
  "xcode_version": null
}
```

## 机械选型规则

1. 四项全为 `PASS` → `tauri_mobile`；
2. 任一项为 `FAIL` → `uniffi_native`；
3. 没有 `FAIL` 但存在 `NOT EXECUTED` → `undetermined`。

第三态防止把缺设备误写成产品失败，也防止在没有证据时推进移动 shell。
