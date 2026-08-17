- 2026-08-16：UniFFI 的普通移动绑定应与 GPL 语音实现拆成 `uniffi` / `native-voice` 两个 feature，避免不使用 ASR 的客户端静默携带语音栈。
- 2026-08-16：UniFFI callback 不能把一次轮询超时当成终态；必须持续拉取直到核心 operation 报告唯一终态或句柄关闭。
- 2026-08-16：Android 系统钥匙串之前必须由 Kotlin 使用 `applicationContext` 初始化 JNI 全局引用与 `ndk-context`，且该顺序需要在 Rust 构造入口再次检查。
- 2026-08-16：生成的 Kotlin/Swift 源码、C header、modulemap、生成脚本和结构测试应一起版本化，才能让“binding 已落地”成为可执行事实而非状态常量声明。

## [2026-08-17] iOS 产品工程如何与 Android 结构对等（在无法编译验证的前提下），以及报告溯源怎么做到结构性正确

### 一、iOS 工程：结构对等的做法，与本机不可验证的部分

`mobile/ios/` 此前只有 README。现在有一份 SwiftUI 产品工程，**逐个职责与 Android 侧对齐**：

| 职责 | Android | iOS |
|---|---|---|
| 进程入口 + 触发物化 | `YunjianApplication.kt` + `MainActivity.kt` | `Yunjian/YunjianApp.swift` |
| 全部界面 | `YunjianApp.kt` | `Yunjian/ContentView.swift` |
| 界面状态与行为 | `MainViewModel.kt` | `Yunjian/MainViewModel.swift` |
| 唯一 Rust 入口 | `YunjianRepository.kt` | `Yunjian/YunjianRepository.swift` |
| 麦克风采集 | `AudioRecord` | `Yunjian/VoiceCapture.swift`（`AVAudioEngine`）|
| 界面标识 | `TestTags.kt` | `Yunjian/TestTags.swift`（**取值逐字相同**）|
| 测量值回传 | `AcceptanceReport.kt` | `Shared/AcceptanceReport.swift` |
| 十条断言 | `app/src/androidTest/.../FullAcceptanceTest.kt` | `YunjianUITests/FullAcceptanceUITests.swift` |
| 容器内事实 | 同上（instrumentation 在应用进程里）| `YunjianAppTests/ContainerFactsTests.swift` |

**只有最后一行两侧不同，而那是平台差异不是结构分叉**：Android 的 instrumentation 跑在应用进程里，
一个 target 既能驱动界面又能读 `filesDir`、调 Rust 门面；XCUITest 是**另一个进程**，读不到应用容器。
所以 iOS 拆两个 target，两者写同一套 `YUNJIAN-FULL` 行，宿主侧按断言 id 归并**键的并集**。
这条差异直接决定了 `corpus_present` / `atomic_install` / `native_voice_enabled` 只能由进程内 target 报。

**三处刻意的机制差异**（判据相同、机制不同，不是放宽）：
- 没有 `YunjianAndroid.initialize` 的对应物 —— 它存在的唯一理由是 `ndk-context` 需要 JVM 侧 Context；
  iOS 把生成的 Swift 直接链进可执行文件，**不需要 JNI 引导**。裁决选 uniffi_native 时记过
  「Tauri 外壳没有为 ndk-context 提供初始化时机」，iOS 侧连这个问题都不存在。
- 页签选中值 `@SceneStorage` ←→ Compose `rememberSaveable`：`@State` 在场景回收后重建退回初值，
  那正是 `background_return_preserves_layout` 要检的东西。
- 键盘不遮挡靠 `ScrollView` + `.scrollDismissesKeyboard`，Android 靠自己消费 ime 插入值。
  判据 `input_bottom_screen_px > 0` 两侧一样。

**本机不可验证的部分（如实列出，不假装）**：Swift 能否编译、`xcodegen generate` 能否产出可用工程、
`build-xcframework.sh` 能否跑到底（它第一步 `uname -s != Darwin` 即退 2）、`aarch64-apple-ios` 能否链接
（`sherpa-rs` 0.6.8 另有上游阻塞）、界面在真机上的实际表现。十条 verdict 全部 `NOT EXECUTED`。
清单落在 `mobile/ios/README.md` 顶部，与「已验证」那几行并列，并标明验证手段。

**已验证的是能在文本层判定、且一旦漂移会让真机验收白跑的五件事**（`xtask ios_project`）：
文件齐备；唯一 Rust 入口是生成物（产品源码不得出现 `@_silgen_name` / `@_cdecl`）；
两侧标识逐字相同（双向，任一侧多出一个也红）；十条判据的**每个** required 键 iOS 侧都报得出来；
十个方法带 `test` 前缀。

**最值得守的是最后一条**：XCTest 只发现 `test` 前缀的方法。写成 `t01_`（照 Android 的命名习惯）
**一个测试都不会跑，而 run 会显示成功**——这与「随包表有行 ≠ 行里有赏析」同型，是「看起来做了但没做」
的又一个面。我自己第一版就写成了 `t01_`，靠这条断言逮到。

### 二、`commit_sha` 怎么做到结构性正确

**只手改数字会把一个可发现的缺口变成更难发现的伪证据。** 机制是三层：

1. **生成期 clean 守卫**（`provenance::require_clean`）：写报告前 `git status --porcelain -- <被测路径>`
   非空即拒。`git rev-parse HEAD` 报的是**提交**里的字节，真机上跑的是**工作树**里的字节；
   两者不一致时那个 sha 指向的根本不是被测代码——2026-08-17 那次绑错就是这个一般形态。
2. **报告里写被测源码的内容摘要**（`tested_sources`）：10 条被守（Android 产品源码与 instrumentation、
   工程与依赖声明、UniFFI 生成物与移动门面）+ 1 条只记录（`Cargo.lock`）。
   「这份报告描述的是哪份代码」因此不依赖任何人的记忆。
3. **一条 git-free 断言**（`the_committed_mobile_report_still_describes_the_current_sources`）：
   每次 `cargo test` 重算摘要与报告比对，被测源码一变即红，并在判词里写明
   「不要改报告里的摘要让这条变绿」。

**判据刻意不是 git object id 也不是「sha 是 HEAD 祖先」**：`actions/checkout` 默认浅克隆拿不到历史对象，
一条「报告描述的是不是当前代码」的断言**不该因为克隆深度而失效或通过**
（`crates/yunjian-mobile/tests/architecture.rs` 早记过同一条）。git 只在**生成**报告时用一次，
那时人在真机旁边、一定有完整仓库。

**`Cargo.lock` 只记录不设门禁**，理由是它会被工作区里任何无关 crate 的依赖调整改掉。
把它纳入门禁会让**别处的正当改动**把这份真机报告判红，而那种红没有信息量，只会训练人忽略它。

**沿用另一平台那一节的条件从两项加到三项**（版本 + sha + **摘要**）：同一个 commit 上工作树可以不同
（曾经生成报告的那次就带着未提交改动），只对 sha 会把「另一份代码上量到的结果」并进来。

**注入验证六次全部变红**：删 `ContentView.swift`、`test_` 前缀去掉、iOS 标识取值分叉、
改 Android 被测源码、把报告 sha 改成短 sha、被测路径 dirty 时生成报告。

### 三、`mobile-size.json` 之前错在哪，现在的真实值

错在**描述了没有被走的那条分支**：`binding_verdict=undetermined`，两条 `build_commands` 是
`cargo tauri android/ios build …`（tauri_mobile 分支），而实际交付的是原生 Kotlin 壳。
且 `executed_pass=0` 配 `all_pass=false` —— **一份零执行的报告读起来像「测了但没全过」**。

修法不是改 JSON，是让配置与校验按分支分叉：`validate_build_commands` 在 `uniffi_native` 下要求命令
指向 `mobile/android` gradle 与 `mobile/ios` xcodegen/xcodebuild、拒绝 `cargo tauri`，
**并断言另一条分支的生成工程在树上不存在**——「哪条分支被选中」因此在文件系统上可判定，
而不是靠读注释。`tauri_mobile` 分支的旧判据原样保留。

真实测量值（本机构建、逐个核对 ABI）：

| 产物 | 字节 | ABI |
|---|---|---|
| `yunjian-arm64-v8a-release.apk` | 42 699 280 | arm64-v8a |
| `yunjian-armeabi-v7a-release.apk` | 30 656 370 | armeabi-v7a |
| `yunjian-x86_64-release.apk` | 46 852 187 | x86_64 |
| `yunjian-release.aab` | 86 503 972 | 三 ABI |

7 条断言现为 **4 PASS / 1 FAIL / 2 NOT EXECUTED**（此前 0 / 0 / 7）：
`android_aab`、`apk_ceiling`（三个都 < 80 MiB）、`packaged_assets`（扫 4 个 ZIP 共 115/115/115/153 条目，
无 corpus .db、无语音模型）、`instrumented_device_smoke`（Pixel 8 真机三步）PASS；
`ios_archive`、`artifact_sizes_measured` NOT EXECUTED 并带 macOS 判据。

### 四、顺带逮到的两件事

**1. 分发 ABI 集与「移动端默认开 voice」两个决定冲突（新发现，需用户裁量）。**
`i686-linux-android` 不在上游 sherpa-onnx 的预编译清单里（`crates/yunjian-voice/build.rs` 的
`PREBUILT_TARGETS`），所以带 voice 的 **x86 APK 在本机构建不出来**，`android_per_abi_apks` 判 FAIL。
三条出路都需要用户定：从声明的 ABI 集里去掉 x86 / 为 i686 源码编译 sherpa-onnx（cmake+C++17）/
x86 那一档不带 voice（那会让四个 APK 不是同一个产品）。
**我没有改那条四 ABI 判据**——事后放宽等于把门禁谈掉；判词里写全了成因与三条出路。
清单不写第二份副本：一条测试解析 `build.rs` 的 `PREBUILT_TARGETS` 比对，上游哪天补上 i686 它会红。

**2. `zip` 的 `default-features = false` 让产物扫描器误报「产物坏了」。**
`ZipArchive::by_index` 会为条目建解压器，于是 deflate 条目报
`Compression method not supported`——而真实 release APK 里恰好一部分是 deflate
（arm64 那份实测 70 条 stored、45 条 deflate）。**那条报错读起来像 APK 损坏**，实际是扫描器要了
它不需要的能力：这个守卫只问「包里有没有 corpus .db 或语音模型」，只需要**条目名**。
改用 `file_names()`（只读中央目录）即解，不必动 workspace 的 feature。

**「先把已知前提做完」第四次生效**：本机没有 gradle（`mise install gradle@8.13` 一条即得），
Android SDK/NDK/cargo-ndk/debug keystore 全在位。于是「Android 无法在本机构建」这个前提
根本不成立，重跑真机验收与真实产物测量都做到了。**报环境限制之前先把能装的装上。**

**注入验证里我自己又栽一次边界错**：解析 `PREBUILT_TARGETS` 时先 `split_once(']')`，
而第一个 `]` 落在类型标注 `&[&str]` 上，于是解析出空清单——**空清单里当然没有 i686，
这条测试会永远绿**。一条假门禁比没有门禁更糟。先切 `= &[` 才对。
