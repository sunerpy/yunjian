简体中文

# 平台要求

云笺的语音功能（朗读与语音默写）有系统版本底线，**低于底线的系统拿不到语音，但拿到的是
完整的打字练习产品，不是一个坏掉的应用**。词典检索、三种打字默写模式、FSRS 复习、MCP
服务器全部与语音无关，也全部不受这张表约束。

这份文档由 `cargo test -p yunjian-voice --test platform_config` 钉住：表里的每个版本号
都必须与 `crates/yunjian-voice/src/platform.rs` 的 `FLOORS` 逐字一致，改一处漏一处会让测试失败。

## 目录

- [五平台最低版本](#五平台最低版本)
- [每条底线的来源](#每条底线的来源)
- [麦克风授权链](#麦克风授权链)
- [低于底线时的行为](#低于底线时的行为)
- [验证状态](#验证状态)

## 五平台最低版本

| 平台        | 最低版本        | 底线来自              |
| ----------- | --------------- | --------------------- |
| **Linux**   | glibc 2.31      | 发布构建宿主          |
| **Windows** | Windows 10 1809 | WebView2 常青分发     |
| **macOS**   | 14.2            | `cpal` CoreAudio 后端 |
| **Android** | 8.0（API 26）   | `cpal` AAudio 后端    |
| **iOS**     | 14.0            | Tauri v2 部署目标     |

## 每条底线的来源

### macOS 14.2 —— 已确认为真

方案研究报的「`cpal` 要求 macOS 14.2」**成立**，但成因与「上游文档这么写」不同，是实读代码
得到的：

`cpal` 的 macOS 后端有一个 loopback（录制系统输出）路径，`src/host/coreaudio/macos/loopback.rs`
里无条件引用了 `AudioHardwareCreateProcessTap`、`AudioHardwareDestroyProcessTap` 与
`CATapDescription` —— 这三个 API 都是 **macOS 14.2 才引入**的。它们经
`objc2-core-audio` 绑定，而那个 crate 用的是普通的
`#[link(name = "CoreAudio", kind = "framework")] extern "C" {}`，**没有做弱链接**。
所以即使我们从不启用 loopback，符号也在二进制里，在 14.2 之前的系统上加载即失败。

两个候选 `cpal` 版本（`rodio 0.22.2` 内部的 0.17.3，与方案曾直接固定的 0.18.1）**都有
这条路径**，因此换版本回避不了。

`bundle.macOS.minimumSystemVersion` 因此显式写成 `14.2`，覆盖 Tauri 默认的 `10.13`：
留着默认值不会让构建失败，只会让 10.13–14.1 的用户装上一个启动即崩的应用。

### Android API 26 —— 高于 Tauri 文档写的 24

`cpal` 的 Android 后端通过 `ndk` crate 的 **`api-level-26`** 特性绑定 AAudio
（`[target.'cfg(target_os = "android")'.dependencies.ndk] features = ["audio", "api-level-26"]`，
0.17.3 与 0.18.1 逐字相同），而 Tauri 文档写的 Android 最低是 7.0 / **API 24**。

**冲突已按方案要求解决：`minSdk` 提到 26，并作为产品要求记录在此。** 落地在三处，且有
测试断言三处一致：

- `crates/yunjian-voice/mobile/android/build.gradle.kts` 的 `minSdk = 26`
- `crates/yunjian-voice/mobile/tauri.audio.conf.json` 的 `bundle.android.minSdkVersion = 26`
- CI 里链接器写死 `aarch64-linux-android26-clang`

留在 24 的后果不是编译失败，而是**在 API 24/25 设备上运行期崩溃**——那两个级别没有 AAudio
的符号。这也是为什么它必须是配置层的硬约束而不是文档里的一句提醒。

### iOS 14.0 —— 底线由 Tauri 决定，不是音频栈

`AVAudioSession` 采集路径本身在更早的 iOS 就有。14.0 来自 Tauri v2 的
`IPHONEOS_DEPLOYMENT_TARGET` 默认值，`bundle.iOS.minimumSystemVersion` 显式写出以免上游改默认值。

插件内部对 iOS 17 做了分支：`AVAudioApplication.requestRecordPermission` 是 17.0 才有的，
更早的系统走已废弃的 `AVAudioSession.sharedInstance().recordPermission`。两条路径都在，
因此 14.0–16.x 不受影响。

### Windows 10 1809 —— 底线由 WebView2 决定

WASAPI 采集本身从 Vista 就有。1809 来自 Tauri v2 依赖的 WebView2 常青运行时——它不再支持
更早的 Windows 10 版本。桌面外壳没有 WebView2 就跑不起来，所以音频底线在这里不是决定因素。

### Linux glibc 2.31

ALSA 后端需要 `libasound2`（构建期需要 `libasound2-dev`）。2.31 是 Ubuntu 20.04 的 glibc，
也是发布矩阵里最旧的构建宿主；动态链接的产物不能在比构建宿主更旧的 glibc 上运行。

## 麦克风授权链

**在 Rust 里采集绕开了 WebView，但没有绕开操作系统。** 每个平台都要一次授权，而各平台把
授权放在完全不同的层：

| 平台    | 授权点                                         | 谁能发起                      |
| ------- | ---------------------------------------------- | ----------------------------- |
| Linux   | 无系统级麦克风门                               | 进程自己                      |
| Windows | 「设置 → 隐私 → 麦克风」，首次 WASAPI 采集触发 | 进程自己                      |
| macOS   | TCC 弹窗，由**已签名**进程首次触达输入设备触发 | 进程自己（但需 entitlements） |
| Android | 运行时权限对话框                               | **只有 Android framework**    |
| iOS     | 录音授权 + `AVAudioSession` 激活               | 需要原生插件，`cpal` 不做     |

### Android 需要两条权限，不是一条

```
android.permission.RECORD_AUDIO
android.permission.MODIFY_AUDIO_SETTINGS
```

- `RECORD_AUDIO` 是 dangerous 级，必须运行时申请，用户可拒。
- `MODIFY_AUDIO_SETTINGS` 是 normal 级，**声明即在安装时授予、无需运行时申请**；但缺声明时
  WebView 侧的 `getUserMedia` 直接失败（`tauri#10846`），原生采集在部分机型上也拿不到可用的
  输入路由，因为音频模式与路由切换归它管。

Kotlin 插件把两者分成两个 alias 上报，并额外上报
`shouldShowRequestPermissionRationale` 取反后的「永久拒绝」判定——用户勾了「不再询问」之后
`requestPermissions` 会**立即以拒绝回调返回且不弹窗**，此时 UI 必须引导去系统设置而不是
再点一次按钮。

### macOS 需要三件套，缺一不可

1. `Info.plist` 的 `NSMicrophoneUsageDescription`——缺它时进程在触达输入设备的瞬间被系统
   **直接终止**，不是返回错误。
2. **独立的** `Entitlements.plist`，含两个键：
   ```
   com.apple.security.device.microphone
   com.apple.security.device.audio-input
   ```
3. `bundle.macOS.entitlements` 指向那个文件。

**这套配置只在已签名并公证的构建里才会因缺失而失败**（`tauri#8314`）：`tauri dev` 与本地
未签名构建全绿。这就是为什么它必须由配置层的静态断言加 CI 上的 `codesign -d --entitlements -`
一起兜，而不能靠开发期观察。

### iOS 必须先激活 `AVAudioSession`

实读 `cpal 0.18.1` 的 `src/host/coreaudio/ios/mod.rs` 确认：它会取
`AVAudioSession.sharedInstance()` 并设 `setPreferredIOBufferDuration`，但**从不调用
`setCategory` 或 `setActive`**。而它判断设备有无输入的依据是 `inputNumberOfChannels()`，
那个值在会话未激活、类别仍是默认 `.soloAmbient` 时是 **0**——于是 `cpal` 会报告「没有输入
设备」，表现为一个既不报错也录不到东西的通路。

Swift 插件因此按 `.playAndRecord` + `.measurement` 激活会话：`.playAndRecord` 是因为朗读
示范与语音默写在同一个界面里交替，`.record` 会掐掉输出路由；`.measurement` 关掉系统的
自动增益与降噪，那些处理对识别有害。

## 低于底线时的行为

**每一条失败路径都退回打字练习，并给出解释性消息。** 这由类型系统而不是约定保证：
`VoiceError::degrade_reason()` 是穷尽匹配，新增一个错误变体不给它降级原因就编译不过。

| 情形           | 降级原因                 | 用户看到的                     |
| -------------- | ------------------------ | ------------------------------ |
| 未编译语音能力 | `FeatureDisabled`        | 已切换到打字练习，评分内核相同 |
| 系统版本过低   | `SystemTooOld`           | 给出所需版本号                 |
| 授权被拒       | `PermissionDenied`       | 给出该平台的设置路径           |
| 被管理策略禁用 | `PermissionRestricted`   | 说明需要管理员放开             |
| 尚未授权       | `PermissionUndetermined` | 说明点开始时会弹窗             |
| 无输入设备     | `NoInputDevice`          | 提示接麦克风或换设备           |
| 采集失败       | `CaptureFailed`          | 提示可能被其他程序独占         |

每条消息都要回答两件事：为什么不能录、怎么恢复。有测试断言七个原因 × 五个平台的三十五
条消息都提到「打字练习」，避免只说「麦克风不可用」把用户留在原地。

## 验证状态

诚实标注：本 spike 的开发机是 Linux，无 macOS/Android/iOS 宿主机、无物理设备、无签名身份。
下表分两栏：**本机**是开发机上的实测结论，**CI** 是 `audio-permissions.yml` 在有对应宿主机处
拿到的真实结论。凡是两栏都没有「通过」的项，一律给出阻塞原因与所需条件。

| 项目                                   | 本机                 | CI           | 证据 / 阻塞原因                                                                                                                                                                     |
| -------------------------------------- | -------------------- | ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Linux 采集 1 秒 16 kHz 单声道 RMS 非零 | **通过**             | **通过**     | 本机 PulseAudio null sink + `module-sine`，RMS 0.353550；CI 同路径，setup 先用 `parec` 验到 `peak=16384` 再跑测试                                                                   |
| macOS 采集 1 秒 16 kHz 单声道 RMS 非零 | 未执行               | **通过**     | 无 macOS 宿主机。CI 在 `macos-14` 上装 BlackHole 2ch 并重启 `coreaudiod`，采到 RMS 0.353544；**设备原生 2 声道，实际走了降混路径**，不是恰好格式就对                                |
| macOS entitlements 进入签名产物        | 未执行               | **通过**     | 无签名身份。CI 用 ad-hoc 签名 + `codesign -d --entitlements -` 读回两个键；并有反向验证：不带 `--entitlements` 签名后 `audio-input` 确实读不到                                      |
| macOS 公证构建的运行期行为             | 未执行               | **未执行**   | 需付费 Apple Developer 账号与公证凭据。`tauri#8314` 描述的失败只在这一层显现，属发布流程（todo 71/74）                                                                              |
| macOS TCC 授权弹窗                     | 未执行               | **未执行**   | runner 无 GUI 会话，命令行进程不触发 TCC 对话框。需签名产物 + 真机，属 todo 68                                                                                                      |
| Windows 采集                           | 未执行               | **部分通过** | runner 无麦克风也无免交互虚拟输入驱动（枚举结果为 0 个设备）。已验证的是降级路径：报 `NoInputDevice` 并给出带设置路径的中文解释，不 panic 不挂住。真实采集需有声卡的 Windows 宿主机 |
| Android 构建采集路径                   | 未执行               | **通过**     | 缺 NDK。CI 用 r26d，链接器解析为 `aarch64-linux-android26-clang`（有断言钉住不是 24）                                                                                               |
| Android 运行时权限对话框               | 未执行               | **未执行**   | 需真机或模拟器。CI 只构建；真机验收留给 todo 68                                                                                                                                     |
| iOS 构建采集路径                       | 未执行               | **通过**     | 缺 Xcode SDK。CI 在 `macos-14` 上 `cargo build` 成功——本工作流不带 `voice`，因此不受 `sherpa-rs` 的 cdylib 限制影响                                                                 |
| iOS 真机采集与授权                     | 未执行               | **未执行**   | 需 macOS + Xcode + 已配置签名的设备，属 todo 68                                                                                                                                     |
| macOS 14.2 底线                        | **已确认（读代码）** | —            | `loopback.rs` 无条件引用 14.2 API 且 `objc2-core-audio` 未做弱链接。**未**在 14.1 及更早系统实测运行期失败——无该版本宿主机                                                          |
| Android API 26 底线                    | **已确认（读清单）** | —            | `ndk` 的 `api-level-26` 特性，cpal 0.17.3 与 0.18.1 逐字相同                                                                                                                        |
| iOS `AVAudioSession` 激活为必需        | **已确认（读代码）** | —            | cpal 的 iOS 后端 `setActive`/`setCategory` 出现次数为 0                                                                                                                             |
| 权限被拒时降级到打字练习               | **通过**             | **通过**     | 五平台 × 七种原因全部给出带设置路径的解释；反向验证过（改成不降级则 harness 报 5 处问题、单元测试同步失败）                                                                         |

## 相关文档

- [语音构建](VOICE-BUILD.zh.md)——五平台原生依赖构建、链接方式、GPL-3.0 影响
- [架构](ARCHITECTURE.zh.md)——分层与移动端逃生通道
