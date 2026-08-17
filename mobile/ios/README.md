# iOS 产品工程

binding verdict 是 **`uniffi_native`**（`crates/yunjian-mobile/src/lib.rs` 的 `BINDING_VERDICT`，
2026-08-16 由判据②的真机实测确定：语料物化在物理 Pixel 8 上 109.849 秒，超过预声明的 60 秒）。
因此本目录是一个真实的 SwiftUI 工程，**唯一允许的 Rust 入口**是
`crates/yunjian-mobile/bindings/generated/` 下 UniFFI 生成的 Swift 与它的 C 头 + modulemap。

## 未经 Xcode 编译验证（先读这一节）

**本机是 Linux，没有 macOS 与 Xcode，所以本目录下的 Swift 与构建脚本从未经过编译或运行。**
这不是「大概没问题」，下面逐条列出哪些事实已被验证、哪些没有：

| 事项                                        | 状态                                                                |
| ------------------------------------------- | ------------------------------------------------------------------- |
| 文件齐备、两个测试 target 就位              | **已验证**（`xtask` 结构门禁，删任一必备文件即红）                  |
| 界面标识与 Android 侧逐字相同               | **已验证**（两份 `TestTags` 逐条比对，任一侧多/少/改值即红）        |
| 十条判据的每个 required 键 iOS 侧都报得出来 | **已验证**（扫 harness 源码里的键字面量，缺一个即红）               |
| 十个 XCUITest 方法带 `test` 前缀            | **已验证**（缺前缀时一个测试都不会跑而 run 显示成功，这条专门守它） |
| 调用的绑定符号在生成的 Swift 里真的存在     | **已验证**（逐个在 `YunjianMobile.swift` 里核对，不凭记忆写名字）   |
| 采集结束停用 `AVAudioSession`               | **已验证**（激活与 `defer` 停用配对计数，删掉或挪出 `defer` 即红）  |
| 一轮时长按采样率换算且与 Android 相同       | **已验证**（帧数不许写字面量；两侧毫秒数比对，差过 1 ms 即红）      |
| 用到的 AVFoundation API 名与签名            | **已核对**（逐个查 Apple 官方文档，非编译验证）                     |
| **Swift 能否编译通过**                      | **未验证** —— 无 Swift 编译器                                       |
| **`xcodegen generate` 能否产出可用工程**    | **未验证** —— 无 xcodegen 与 Xcode                                  |
| **`build-xcframework.sh` 能否跑到底**       | **未验证** —— 它第一步就要求 macOS（`uname -s != Darwin` 即退 2）   |
| **`aarch64-apple-ios` 能否链接**            | **未验证** —— 且已知 `sherpa-rs` 0.6.8 在 iOS 上另有上游阻塞        |
| **界面在真机上的实际表现**                  | **未验证** —— 需物理 iPhone + 签名身份                              |
| **十条断言的 verdict**                      | 全部 `NOT EXECUTED`，见 `docs/reports/mobile-qa-2026-08-17.json`    |

判据是可复核的：`cargo test -p xtask ios_project` 跑的就是上表「已验证」那几行。
**上表「未验证」的行，不要在没有 macOS 的地方声称它们通过了。**

## 与 Android 工程的结构对应

十条断言两个平台共用同一套判据（`xtask/src/acceptance/mobile/full_criteria.rs`），所以两侧的
结构必须对得上，否则同名断言会量不同的东西。

| 职责                | Android                                         | iOS                                             |
| ------------------- | ----------------------------------------------- | ----------------------------------------------- |
| 进程入口 + 触发物化 | `YunjianApplication.kt` + `MainActivity.kt`     | `Yunjian/YunjianApp.swift`                      |
| 全部界面            | `YunjianApp.kt`（Composable）                   | `Yunjian/ContentView.swift`（SwiftUI View）     |
| 界面状态与行为      | `MainViewModel.kt`                              | `Yunjian/MainViewModel.swift`                   |
| 唯一 Rust 入口      | `YunjianRepository.kt`                          | `Yunjian/YunjianRepository.swift`               |
| 麦克风采集          | `AudioRecord`（在 ViewModel 内）                | `Yunjian/VoiceCapture.swift`（`AVAudioEngine`） |
| 界面标识            | `TestTags.kt`                                   | `Yunjian/TestTags.swift`（取值逐字相同）        |
| 测量值回传          | `AcceptanceReport.kt`                           | `Shared/AcceptanceReport.swift`                 |
| 十条断言            | `app/src/androidTest/.../FullAcceptanceTest.kt` | `YunjianUITests/FullAcceptanceUITests.swift`    |
| 容器内事实          | 同上（instrumentation 就在应用进程里）          | `YunjianAppTests/ContainerFactsTests.swift`     |

**只有最后一行是两侧不同的**：Android 的 instrumentation 跑在应用进程里，一个 target 既能驱动
界面又能读 `filesDir`、调 Rust 门面；XCUITest 是另一个进程，读不到应用容器。所以 iOS 拆成两个
target，两者写同一套 `YUNJIAN-FULL` 行，宿主侧按断言 id 归并键的并集。

三处刻意的机制差异（不是结构分叉）：

- **没有 `YunjianAndroid.initialize` 的对应物。** 它存在的唯一理由是 `ndk-context` 需要 JVM 侧的
  Context；iOS 把生成的 Swift 直接链进可执行文件，不需要 JNI 引导。
- **页签选中值用 `@SceneStorage`**，对应 Compose 的 `rememberSaveable`：`@State` 在场景被回收后
  重建时退回初值，那正是「后台返回时页面不空白」这条断言要检的东西。
- **键盘不遮挡靠 `ScrollView` + `.scrollDismissesKeyboard`**，而 Android 靠自己消费 ime 插入值
  （`imePadding()`）。判据 `input_bottom_screen_px > 0` 两侧相同，机制不同。

## 怎么构建（在 macOS 上）

```sh
brew install xcodegen
cd mobile/ios
xcodegen generate          # 由 project.yml 产出 Yunjian.xcodeproj（刻意不进版本库）
open Yunjian.xcodeproj     # 或用 xcodebuild
```

`Yunjian` target 的 preBuild 脚本会跑 `scripts/build-xcframework.sh`：交叉编译
`yunjian-mobile` 的静态库，与 UniFFI 的 C 头 + modulemap 一起打成
`build/YunjianMobileFFI.xcframework`。缺 Rust target 或缺 Xcode 时**失败并指名缺什么**，
而不是产出一个跑起来找不到符号的应用（与 Android 的 `cargoNdkBuild` 同一约定）。

两个可调项，与 Android 的 gradle 属性一一对应：

| 环境变量              | 默认                                      | 含义                                     |
| --------------------- | ----------------------------------------- | ---------------------------------------- |
| `YUNJIAN_VOICE`       | `1`                                       | 是否编译 `native-voice`；`0` 即 MIT 构建 |
| `YUNJIAN_IOS_TARGETS` | `aarch64-apple-ios aarch64-apple-ios-sim` | 要出哪几片                               |

**`Yunjian.xcodeproj` 不进版本库。** 与 Android 不签入 `jniLibs/*.so` 同一个道理：生成物会与
声明悄悄漂移，而漂移的表现在运行期而不是构建期。工程结构的唯一真相是 `project.yml`。

## 许可边界

与 Android 相同：默认构建带 `native-voice`，而预编译的 sherpa-onnx 静态含 GPL-3.0 的 espeak-ng，
所以**默认构建整体按 GPL-3.0 条款提供**，`packaging/licenses/` 作为 resource 打进 bundle。
要一份 MIT 构建用 `YUNJIAN_VOICE=0`，那时 `startAsr` 返回「当前原生库未启用 native-voice」
而不是静默降级。完整论述见仓库根 [`LICENSES.md`](../../LICENSES.md)。

## 真机验收

十条断言在 `YunjianUITests/` 与 `YunjianAppTests/`，判据在
`xtask/src/acceptance/mobile/full_criteria.rs`。**设备侧只报测量值，PASS/FAIL 由宿主侧判定**
——让被测物自己判等于把门禁搬进被测物内部。

```sh
cd mobile/ios && xcodegen generate
xcodebuild build-for-testing -scheme Yunjian -destination 'generic/platform=iOS'
# 上传 Device Farm（ios_full 段）→ 回收测量与截图 → 落盘报告
cargo run -p xtask -- acceptance --platform ios --set full
```

回传日志放 `docs/reports/mobile-qa-ios-measurements.log`（路径按平台分开：同名会让
「Android 跑过、iOS 没跑」变成「iOS 读到了 Android 的测量值」），截图放 `docs/reports/mobile-qa/`。
缺回传时十条保持 `NOT EXECUTED`——远端池可达证明的是「有真机」，不是「真机上跑过云笺」。

真实 archive 必须在装有 Xcode 26 / iOS 26 SDK、Distribution 证书与 provisioning profile 的
macOS 上产生，再上传 TestFlight；**Linux 不能替代这一步**，见
`mobile/device-farm.toml` 的 `ios_full.blocked_reason`。
