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
- 2026-08-17：一次性 Android JNI spike 可保留在仓库但通过自己的 `[workspace]` 与 `Cargo.lock` 脱离产品 workspace；CodeBuild 必须用 `--locked --manifest-path`，并显式 `--target-dir` 回到根 `target/`，才能同时守住 10 成员契约与既有 APK 注入路径。
- 2026-08-17：workspace 成员门禁应解析根 `Cargo.toml` 的 `[workspace].members` 并逐项比较冻结清单；临时注入第 11 项可证明守卫真会变红，避免注释文本或模糊计数造成假绿。
- 2026-08-17：隔离 worktree 的 `--all-features` 语音实测要显式把 `YUNJIAN_MODEL_DIR` 指向已有缓存根；未设置时 5 个 streaming 测试因模型缺失失败不代表代码回归，设置后必须重跑并达到 0 failed。

## [2026-08-17] 我第六次否掉自己：todo 41 不是「需用户提供 endpoint」，本机就能跑

**我此前反复把它报成待用户决策**（「本机无开放权重推理运行时，`generation_executed=false`，
需用户提供 endpoint」），并据此让 F1 记 todo 41/75 两条 FAIL、F4 记阻塞 1。**这个判断是错的。**

**实际情况**（我逐步实测）：

1. `xtask/src/pregenerate.rs:271` 的 `Generator::connect` 走的是 **Ollama `base_url`** ——
   `DEFAULT_MODEL = "deepseek-r1:7b"`、`DEFAULT_MODEL_LICENSE = "MIT"`、`DEFAULT_PROVIDER = "ollama"`
2. **不需要 GPU**。本机 61 GiB 内存，只要生成 **16 条**
3. `ollama` 可以**免 root 免 systemd** 装：官方 `install.sh` 要 root，但 GitHub release
   的 tarball 解开就能跑（`./bin/ollama serve` + `OLLAMA_MODELS` 指向 `/config`）
4. **实测冒烟：39 秒出 66 字中文赏析**，质量可用
   （「展现了边塞的壮丽景象与历史厚重感，通过描绘明月与关河的对比…」）

**下载地址的坑**：`https://ollama.com/download/ollama-linux-amd64.tgz` 返回 **404**
（size=9 字节），真实资产在 GitHub releases 且是 **`.tar.zst`** 不是 `.tgz`：
`ollama-linux-amd64.tar.zst`（1355 MiB）。**我第一次 tar 报 `Child returned status 2`
读起来像归档损坏，实际是下载到了 9 字节的 404 页面。**
判据：`ls -lh` 看体积，或 `file` 看类型，不要直接怀疑归档格式。

**为什么这个误判值得记**：我把「本机没有 X」当成了「需要用户提供 X」，
而中间少了一步——**「X 能不能装上」**。这与「能不能软件模拟」是同一条判据
（音频那次一条 `pactl load-module module-sine` 即解；托盘那次缺的不是硬件
而是总线上的一个名字）。**「本机没有」不等于「本机不能有」。**

**这是「验证而非采信」第十三次生效，第六次否掉的是我自己的判断。**

## [2026-08-17] rpath 与「build script 刻意不做别的事」不冲突；F3 的两条缺口只有一条是真的

### 一、rpath 为什么可以进 build script

`crates/yunjian-app/build.rs` 的文件头写着「刻意不做别的事……build script 的失败诊断远比
一条测试断言难读」。补 rpath 看起来违反它，实际不违反，判据是**那条原则拒绝的是什么**：
它拒绝的是「能用一条断言完成、却被塞进 build script」的**校验**工作（图标校验、权限扩充）。
rpath 不属于这一类，两条理由缺一不可：

1. **没有「用断言替代」这个选项。** 链接参数只能由 build script 经 `cargo:rustc-link-arg-*`
   交给 cargo，且必须由**二进制所属的包**发（`cargo:rustc-link-arg` 不从 rlib 依赖传递到
   最终链接步骤）。`yunjian-voice` 替它发是无效的。
2. **这段代码不产生需要诊断的失败。** 只读两个环境变量再打印一行，没有 I/O、没有校验、
   没有可 panic 的路径——「诊断难读」这个代价根本不会发生。

所以判据不是「build script 里能不能放东西」，而是「这件事有没有别的落点」＋「它会不会制造
难读的失败」。这个区别写进了 build.rs 的文件头，否则下一个人会以为原则被违反了。

**顺带测得一件 cli 注释没写的事**：`libonnxruntime.so` 也必须与二进制同目录。
`libsherpa-onnx-c-api.so` 自己带 `RPATH=$ORIGIN`（预编译产物自带），所以只要与它同目录即可
被解析；漏拷时报 `libonnxruntime.so: cannot open shared object file`，与缺 rpath 的报错**同形
但主体不同**，容易读成 rpath 没生效。

### 二、`mobile-package` 的处置：不实现、不改配置

**F3 命中的是解释这次修复的注释，不是活的配置行。** PR #110 引入
`cargo run -p xtask -- mobile-package`，PR #114（`a9ca77e`）已把它换成 run full-acceptance-29
逐条实测过的 `gradle + cp + zip` 序列；现存唯一文本命中是第 50 行刻意留下的历史记录。
判据是 `git show 36f3181/a9ca77e -- mobile/device-farm.toml`，不是 grep 计数。

**「报告存在 ≠ 报告可复现」这条本身是对的，缺口也真的存在过**——只是它在 a9ca77e 就被
关掉了，而**关闭它的那行注释让缺口在文本层看起来仍然存在**。这是「解释一条规则的文字命中
这条规则」的第六次，且第一次是以**假红**的形式出现（前五次都是假绿）：同一个机制缺陷，
方向可以反过来。

### 三、两条断言各自的机制，以及为什么都必须剔注释

- **rpath 契约**（`xtask/tests/workspace_contract.rs`）：先从 `[workspace].members` 筛出
  「有 bin 且某个 feature 值含 `yunjian-voice/voice`」的包，要求这个集合**恰好等于**
  必需集 ∪ 免除集（免除要写理由：`xtask` 只在 cargo 下跑，`yunjian-mobile` 唯一 bin 是
  `uniffi-bindgen`、移动端加载 cdylib 不经 rpath）。**集合相等而非「必需集都合格」**是关键：
  新增一个能开 voice 的二进制包时它两边都不在，断言逼人做一次决定。再对必需集逐份 build.rs
  要求两条 `-bins` link-arg 与 `CARGO_FEATURE_VOICE` 分流，并**禁止 `-bin=<name>` 形式**
  （只作用于点名的那一个，改名或加 bin 时静默漏掉）。
- **子命令引用**（`xtask/src/main.rs` 的 tests）：扫 `*.toml`/`Makefile`/`*.yml`，按 token
  走状态机抽出被调用的子命令名，逐个问 **clap** 是否认得。**问 clap 而不是解析 `Commands`
  源码文本**：用户敲的名字由 clap 的改名规则（`CorpusBuild` → `corpus-build`）决定，源码里的
  变体名只是输入；问 clap 顺带免疫「注释里提到的变体名被当成数据」。
  **必须要求 `run` 出现过**，否则 `cargo test -p xtask corpus_package` 的测试名过滤器会被
  当成子命令，在一条完全正确的命令上变红——那种红没有信息量，只会训练人忽略它。
  实测扫描面 18 处调用 / 12 个子命令，分布在 Makefile(9)、corpus-release.yml(5)、
  mobile.yml(3)、corpus.yml(1)；零命中守卫防「写法变了而 marker 没跟上」。

**两条都必须剔注释，且注入验证证明了这一步是 load-bearing 的**：去掉 `strip_hash_comments`
后第 50 行的历史注释立刻被判成活调用（假红）；删掉 `$ORIGIN` 发射只留文件头散文时，
不剔注释的朴素 `contains` 会依然为真（假绿）。

### 四、我自己在这次里被自己的夹具逮到一次

抽出的子命令名粘着 TOML 的收尾引号（`verify-models"`），于是 `known.contains` 为假——
**一条完全正确的配置会被判成「引用了不存在的子命令」**。仓库当前恰好没有带引号的 xtask 调用
（18 处全在 Makefile 与 YAML 的裸命令里），所以真实扫描是绿的，**这个缺陷只有夹具能暴露**。
教训：写「扫真实文件」的断言时，必须另配一条**只吃合成输入**的测试来证明抽取器本身正确；
真实文件恰好不覆盖某个形态时，那个形态的 bug 会一直躺着，直到有人写出那种形态并被假红拦住。

### 五、基线数字重测，没有采信交接

交接说全量 1317，我用同一段聚合脚本 stash 掉改动后重测 `main` 得 **1318 passed / 0 failed /
9 ignored**；1318 + 6 条新测试 = 1324，算术闭合。差 1 不影响结论，但**「验证而非采信」在这里
第十三次生效**——若直接采信 1317，1324 就会对不上 6，我会去找一个不存在的问题。

### 六、磁盘纪律：不建 worktree 也是一个选项

交接强调 worktree 必须在 `/config`（`/tmp` 所在的 `/dev/root` 只剩 16 GiB，一个 `target/`
就 92 GiB，爆盘的失败形态是 `LLVM ERROR: IO failure on output stream` + `Bus error`，
**读起来像编译器崩溃**）。这次改动只涉及 3 个文件，直接在原树分支即可，省掉一份 92 GiB
`target/` 的重建——**「隔离」的成本要和收益比，3 个文件的改动不值一次全量重建。**
