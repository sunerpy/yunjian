# 移动端可行性实测

> [!WARNING]
> **verdict: `uniffi_native`。** `NOT EXECUTED` 既不是产品失败也不是通过；只有实测 FAIL 才选 `uniffi_native`，只有四条全 PASS 才选 `tauri_mobile`。
>
> 存在实测 FAIL，机械规则据此选择 uniffi_native。FAIL 是决定性的：不需要知道判据④（iOS TestFlight，用户已决定不做）的结果也能定选型，因此④的范围问题不影响本次结论。要改变它只能靠让那条 FAIL 在真机上变成 PASS，而不是重新解释阈值——阈值在执行前已声明，事后放宽等于把门禁谈掉。

## 本次运行

| 项             | 值                                                                                                                                                                                                     |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 请求平台       | `android`                                                                                                                                                                                              |
| 日期           | `2026-08-18`                                                                                                                                                                                           |
| 提交           | `cf274e869872dca20bc324c96b4944397a4ed581`                                                                                                                                                             |
| 宿主 OS        | `Ubuntu 24.04.4 LTS / Linux 6.17.0-1019-aws`                                                                                                                                                           |
| 前置探测       | `aws devicefarm get-device-pool --arn arn:aws:devicefarm:us-west-2:891377171033:devicepool:9b17cc74-307c-4fdb-b97c-90dc308a8a62/7c385981-1356-4e2f-a3c6-ff2a406c4ea9 --region us-west-2 --output json` |
| 工具可用       | `true`                                                                                                                                                                                                 |
| 工具退出码     | `Some(0)`                                                                                                                                                                                              |
| 已识别物理设备 | `0`                                                                                                                                                                                                    |

### 前置探测原始输出

```text
stdout:
{
    "devicePool": {
        "arn": "arn:aws:devicefarm:us-west-2:891377171033:devicepool:9b17cc74-307c-4fdb-b97c-90dc308a8a62/7c385981-1356-4e2f-a3c6-ff2a406c4ea9",
        "name": "yunjian-probe-pixel8-a15",
        "type": "PRIVATE",
        "rules": [
            {
                "attribute": "ARN",
                "operator": "IN",
                "value": "[\"arn:aws:devicefarm:us-west-2::device:DE5BD47FF3BD42C3A14BF7A6EFB1BFE7\"]"
            }
        ]
    }
}
stderr:

```

## 四项预声明判据

### `microphone_capture` · 经 todo 46 声明的权限路径在物理 Android 与物理 iOS 设备上采集麦克风 PCM

- **verdict**: `PASS`
- **threshold**: sample_rate_hz == 16000、channel_count == 1、rms > 0，且 permission_plugin == record_audio_granted+modify_audio_settings_granted（todo 46 声明的两条权限在包内齐备且录音权限运行期已授予）
- **driver**: Android: adb + instrumented test APK（SpikeMicrophoneTest）；iOS: xcrun devicectl + XCUITest bundle
- **detail**: PASS：必需测量值齐备且满足预声明阈值
- **可执行条件**: 同时备妥已授权 USB 调试的物理 Android、已注册到签名身份的物理 iOS、两端已安装的 instrumented 测试包，并授予麦克风权限
- **measurement**:

```json
{
  "channel_count": 1,
  "device_model": "Pixel 8",
  "os_build": "15/35",
  "permission_plugin": "record_audio_granted+modify_audio_settings_granted",
  "rms": 0.3098704535241539,
  "sample_rate_hz": 16000
}
```

### `corpus_materialization` · 在中端物理 Android 上走生产下载路径校验并原子解压发布语料

- **verdict**: `FAIL`
- **threshold**: 下载 .db.gz、SHA-256 校验成功、原子落入 app storage，duration_seconds < 60 且 crashed == false
- **driver**: adb + instrumented test APK，调用与生产 corpus fetch 相同的下载、校验、解压和原子替换路径
- **detail**: FAIL：测量值齐备但未达阈值——物化耗时 109.849 秒，超过 60 秒阈值
- **可执行条件**: 连接一台已授权 USB 调试的中端物理 Android，安装调用生产语料物化路径的 instrumented test APK，并提供可下载的 .db.gz 发布工件与 SHA-256
- **measurement**:

```json
{
  "artifact_bytes": 223113374,
  "atomic_install": true,
  "crashed": false,
  "device_model": "Pixel 8",
  "duration_seconds": 109.849,
  "os_build": "15/35",
  "production_path": "yunjian_core::assets::AssetResolver::{discover,new}+sync",
  "sha256_verified": true
}
```

### `chinese_ime` · 在 targetSdk 35 的物理 Android 上用中文输入法向边到边窗口的检索框输入中文

- **verdict**: `PASS`
- **threshold**: target_sdk == 35、edge_to_edge == true、中文提交成功、keyboard_overlap_px == 0，输入框始终可见且 visualViewport 正常更新
- **driver**: adb + targetSdk 35 instrumented test APK（SpikeImeTest 驱动 SpikeWebViewActivity）；键盘交互由设备端测试记录 viewport 与控件边界
- **detail**: PASS：必需测量值齐备且满足预声明阈值
- **可执行条件**: 连接已授权的物理 Android，安装 targetSdk 35 测试 APK，启用可输入中文的软键盘，并由设备端 instrumentation 记录输入文本、键盘遮挡和 visualViewport
- **measurement**:

```json
{
  "device_model": "Pixel 8",
  "edge_to_edge": true,
  "entered_text": "云笺",
  "input_visible": true,
  "keyboard_overlap_px": 0,
  "os_build": "15/35",
  "target_sdk": 35,
  "visual_viewport_updated": true
}
```

### `ios_testflight_submission` · 用 Xcode 26 / iOS 26 SDK 完成一次真实 archive、链接与 TestFlight 上传

- **verdict**: `NOT EXECUTED`
- **threshold**: xcode_major >= 26、ios_sdk_major >= 26、archive_link_succeeded == true、upload_succeeded == true 且 testflight_build_id 非空
- **driver**: xcrun devicectl + XCUITest bundle + xcodebuild archive + App Store Connect upload
- **detail**: NOT EXECUTED：真机已回传部分测量值，但以下必需项仍缺：device_model（设备侧未回传）；os_build（设备侧未回传）；xcode_version（设备侧未回传）；ios_sdk_version（设备侧未回传）；archive_link_succeeded（设备侧未回传）；upload_succeeded（设备侧未回传）；testflight_build_id（设备侧未回传）。缺项不记 FAIL——未执行不是产品失败
- **可执行条件**: 用户已决定不做商店提交，本判据因此在范围外，不会有测量值。若日后恢复：在安装 Xcode 26 与 iOS 26 SDK 的 macOS 上连接已注册到签名身份的物理 iOS 设备，配置 Distribution 证书、provisioning profile 与 App Store Connect 上传凭据
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

## 本次顶层裁决的语义

存在实测 FAIL，机械规则据此选择 uniffi_native。FAIL 是决定性的：不需要知道判据④（iOS TestFlight，用户已决定不做）的结果也能定选型，因此④的范围问题不影响本次结论。要改变它只能靠让那条 FAIL 在真机上变成 PASS，而不是重新解释阈值——阈值在执行前已声明，事后放宽等于把门禁谈掉。

## 预声明阈值的修订记录

- 判据①的 `permission_plugin`：由「todo 46 的 Tauri 插件类参与采集」修订为「该插件的可观测契约成立」，即两条权限在已安装包内声明齐备且 `RECORD_AUDIO` 运行期已授予。理由：那个类是 Tauri 插件，只有被 Rust 侧 `tauri::plugin` 注册才存在于进程里，而这份注册属于 todo 69 的 `tauri_mobile` 分支——正是本门禁要决定的事，依赖它会形成循环。采集参数与 RMS 三项阈值一字未改。
- 判据③的 `target_sdk == 35`：**未修订**。PR #102 真机实测 36 是构建缺陷而非阈值错误（tauri 模板取 compileSdk 默认值，AGP 又自动下载缺失平台）；修正方式是在应用模块把 `targetSdk` 钉在 35，而不是放宽判据。
- 判据③新增必需项 `edge_to_edge`：非边到边窗口里系统会替应用避让键盘，遮挡天然为 0，那样的 PASS 证明不了产品自己处理了 ime 插入——而判据引用的正是 edge-to-edge 与 visualViewport 两个长期缺陷。
