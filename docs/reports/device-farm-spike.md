# AWS Device Farm 移动端真机验收可行性实测

日期 2026-08-15 · 基线 `main@7e8109a` · 区域 `us-west-2`（Device Farm 唯一可用区）

## 结论摘要

**Device Farm 解开的是「没有物理设备」这一条阻塞，解不开「没有可安装的移动产物」。**
todo 68 的四条判据里，**三条在 Device Farm 上可测，一条与 Device Farm 无关**；
但四条全部仍需一个真实 APK，而 `mobile/android` 目前只有 README。

| 判据                             | Device Farm 能否测      | 依据                                                                                                |
| -------------------------------- | ----------------------- | --------------------------------------------------------------------------------------------------- |
| ① 麦克风 16 kHz 单声道非静音 PCM | **可测（有重要限定）**  | 真机麦克风硬件存在、三条静音开关全为 false；但**无音频注入**，非零 RMS 只能来自机架环境噪声         |
| ② 语料物化走生产下载路径         | **可测**                | `/data` 剩 104 GB、公网可达（19–31 ms）、gzip 211 MiB 语料余量充足                                  |
| ③ 中文 IME（targetSdk 35）       | **可测**                | 设备为 Android 15 / SDK 35；Gboard 的 147 个 subtype 里有且仅有 `zh_CN`；run 的 locale 覆盖实测生效 |
| ④ iOS TestFlight 提交            | **与 Device Farm 无关** | 需 App Store Connect 凭据与付费开发者账号；用户已决定不发商店                                       |

**Android 链路可行，iOS 链路不可行**：Device Farm 会用通配 provisioning profile 重签名，
因此不必注册设备 UDID，但**产出 `.ipa` 仍需 macOS 与 Xcode**。

## 一、Device Farm 的能力边界（真机实测，非文档推测）

两次探针 run 均 `COMPLETED / PASSED`，跑在 **Google Pixel 8 / Android 15 / SDK 35**
（`google/shiba/shiba:15/AP3A.241005.015/12366759:user/release-keys`，UDID `43160DLJH0009A`）。
合计 **0.42 + 0.5 ≈ 0.9 device-minutes**，走免费额度，**增量费用 $0**。

### 麦克风：硬件在、未静音，但喂不进已知音频

```
feature:android.hardware.microphone            ← 硬件特性存在
feature:android.hardware.audio.low_latency
mic mute FromSwitch=false FromRestrictions=false FromApi=false from system=false
Available input devices (5)；Input MixPorts (9)：primary input / voip_tx / fast input / hifi_input ...
audio_flinger 输入线程 Sample rate: 48000 Hz ×7
appops uid 2000 RECORD_AUDIO → Default mode: allow
```

**但设备上没有任何命令行采集工具**：

```
tinycap -> ABSENT     tinyplay -> ABSENT     tinymix -> ABSENT     arecord -> ABSENT
/system/bin/tiny*: No such file or directory
```

**这两件事合起来决定了判据①的性质**：PCM 必须由我们自己的 APK 通过 `AudioRecord` 采集，
宿主侧 `adb` 拿不到。而且 **Device Farm 没有音频注入能力**——`ScheduleRun` 的
`configuration` 只有 `locale` / `location` / `radios` / `extraDataPackageArn` /
`auxiliaryApps`，没有任何音频字段；官方 FAQ 详述了摄像头可用却从不提音频输入；
竞品把「audio injection」当作差异化功能单独宣传。

**由此得出一条必须写清的区分**：

- 判据①（`rms > 0`）**可以 PASS**——机架环境噪声就是非零信号。
- todo 71 的 `voice_recitation_round_succeeds_end_to_end`（ASR 要识别出一段**已知**背诵）
  **不可能在 Device Farm 上 PASS**——喂不进受控语料。**判据①通过不蕴含语音断言通过。**

### 中文 IME：Gboard 有且仅有一个 `zh_CN` subtype

```
$ adb shell ime list -a -s
com.google.android.inputmethod.latin/com.android.inputmethod.latin.LatinIME
com.google.android.tts/...VoiceInputMethodService

subtype 总数 147，其中 mSubtypeLocale=zh* 命中 1 条：zh_CN
默认 IME = com.google.android.inputmethod.latin/com.android.inputmethod.latin.LatinIME
```

`schedule-run --configuration '{"locale":"zh_CN"}'` 实测生效：设备
`persist.sys.locale` 变为 `zh-CN`（`ro.product.locale` 仍是 `en-US`，说明是运行时覆盖）。
`run.locale` 回传 `zh_CN`。

第一次探针把 subtype dump 截到 #17 就得出「无中文」的错误印象——**subtype 列表按 locale
字母序，`zh_CN` 在倒数第二位**。这正是「不要用 `head` 截断后下结论」的又一例。

### 语料物化的物理余量

```
/dev/block/dm-57  110G  5.4G used  104G avail  6%  /data/user/0
Tensor G3；MemTotal 7 781 000 kB
ping 8.8.8.8 → 2/2 received，rtt 19.160/25.537/31.914 ms
```

判据②要求「中端 Android」。Pixel 8 是旗舰；`Xiaomi Redmi Note 13`（Android 15 / 128 GB）
在池中且 `HIGHLY_AVAILABLE`，是更贴合判据的目标机。

## 二、前置产物从哪来

**链条是「先能构建，才能上 Device Farm」。**

### Android：可行

`.aws/buildspec-android-spike.yml` 描述了在 CodeBuild（`us-east-2`，
`yunjian-runner` 已用 `aws/codebuild/standard:7.0` + `BUILD_GENERAL1_LARGE`）安装
cmdline-tools、`platforms;android-35`、`build-tools;35.0.0`、`ndk;26.3.11579264`，
再 `cargo tauri android build --target aarch64 --apk`。

**该 buildspec 未执行验证**，三个待确认点已写在文件头部：镜像内 Android SDK 实际版本与
路径、NDK 约 2.5 GiB 能否落在磁盘配额内、以及 `tauri android init` 会生成**产品外壳**
`gen/android` 而 todo 69 的 binding 守卫要求 verdict 定了才建外壳——所以冒烟目标只建单 ABI。

成本量级：CodeBuild LARGE Linux 约 $0.02/min，首次构建（NDK 下载 + 全量 Rust 编译）
估 20–40 min ≈ **$0.4–0.8/次**，缓存后显著下降。

### iOS：不可行

必须 macOS。账号内有三个 `MAC_ARM` / `BUILD_GENERAL1_LARGE` 预留容量 fleet
（`test-mac-us-east-2`、`test-mac-us-west-2`、`workkit-macos-fleet`，`baseCapacity` 均为 1），
所以**算力不是阻塞**。真正的阻塞有两条：

1. `cargo tauri ios build` 需要签名身份。Device Farm 会重签名，所以**不需要**注册 UDID，
   理论上可用 `CODE_SIGNING_ALLOWED=NO` 构建后手工打包 `Payload/*.app` 成 `.ipa`；
   **此路径未验证**。
2. 方案 finding B4 记录的 `tauri#15066`：对 Apple iOS 26 SDK 的链接失败。池中 iOS 设备
   已到 `26.3.1`。这条是否已解除**未验证**。

因此 iOS 侧的判据①③与 todo 71 的 iOS 十条**仍是 NOT EXECUTED**，可执行条件为：
「在 `MAC_ARM` fleet 上产出 `.ipa`，且确认 `tauri#15066` 已解除」。

### iOS TestFlight（判据④）应标为「用户决定不做」

它需要 App Store Connect 凭据与付费 Apple Developer Program，与 Device Farm 无关。
用户已决定不发商店。**这一条属于「用户决定不做」，不属于「缺条件未执行」**——两者
在报告里的含义不同，不应混写。

## 三、门禁 verdict 的依赖关系

方案要求四条**全 PASS** 才选 `tauri_mobile`，**任一 FAIL** 选 `uniffi_native`。
若 Device Farm 只跑通 Android 而 iOS 不行，verdict **应当仍是 `undetermined`**，
理由是：

- 判据①的阈值明写「Android 与 iOS **均**满足」，iOS 未测就不是 PASS；
- 「未执行」不等于「失败」。把 NOT EXECUTED 当 FAIL 会让 `uniffi_native` 被一个
  **没测过的**结论选中，那和把 PASS 编出来一样是伪造，只是方向相反；
- 现有一致性校验已经实现了这条：`selection_verdict` 只在四项全 PASS 时给
  `tauri_mobile`，出现 FAIL 才给 `uniffi_native`，其余一律 `undetermined`。

## 四、本轮的接线与其诚实边界

`xtask/src/acceptance/device_farm.rs` 让 `--platform android|ios --set spike|full`
在 `mobile/device-farm.toml` 存在时改用 Device Farm 远端驱动，前置探测由一次
`aws devicefarm get-device-pool` 同时证明凭据、区域与设备池三件事。

实跑结果（`main@7e8109a` + 本分支）：

```
远端真机驱动 AWS Device Farm；android 远端真机池可达，但缺产物
  target/mobile/yunjian-spike.apk、target/mobile/spike-tests.zip、.aws/devicefarm/spike-android.yml
preflight.available = true      exit_code = 0
criteria verdicts = [NOT EXECUTED, NOT EXECUTED, NOT EXECUTED, NOT EXECUTED]
verdict = undetermined
```

**`preflight.available` 从 `false` 变成 `true`，而四条判据一条都没动。** 这是刻意的：
远端池可达证明的是「云上有真机」，不是「真机上装过云笺」。报告同时打印四条确切的
`aws devicefarm` 调度命令，让「未执行」带上可照抄的配方。

## 五、完整前置清单

1. 验证 `.aws/buildspec-android-spike.yml`，产出 `target/mobile/yunjian-spike.apk`
2. 实现 APK 内的三个 instrumented 测试（`SpikeMicrophoneTest` / `SpikeCorpusTest` / `SpikeImeTest`），
   把测量值写 logcat 供宿主侧收集
3. 把 Android 设备池换成 `Xiaomi Redmi Note 13`（Android 15）以贴合判据②的「中端」
4. 建 `MAC_ARM` CodeBuild 项目并验证 `tauri#15066`；不成立则 iOS 侧维持 NOT EXECUTED
5. 判据④按「用户决定不做」标注，不计入缺条件

## 六、账号内的真实资源

| 资源             | 值                                                                                                  |
| ---------------- | --------------------------------------------------------------------------------------------------- |
| Device Farm 项目 | `arn:aws:devicefarm:us-west-2:891377171033:project:9b17cc74-307c-4fdb-b97c-90dc308a8a62`            |
| Android 设备池   | `.../devicepool:9b17cc74-.../7c385981-1356-4e2f-a3c6-ff2a406c4ea9`（静态，锁 Pixel 8 / Android 15） |
| 免费额度         | 1000 device-minutes，剩 **957.45**                                                                  |
| 计费             | METERED $0.17/device-minute；不限量 $250/slot/月                                                    |
| 并发             | 默认 5；`maxJobTimeoutMinutes` 150                                                                  |
| 已知硬限制       | **不接受 `.aab`**（APK 上限 4 GB）；`appArn` 实测**必填**（CLI 标 optional 但服务端拒绝）           |

---

# 追加：Android 产物已产出并真机执行（2026-08-15 第二轮）

基线 `ci/android-spike-build`（`main@eb4d0b1` + 本分支）· CodeBuild `us-east-2` · Device Farm `us-west-2`

## 结论摘要

**buildspec 已真跑通，两个产物已产出并装进物理 Pixel 8；但四条判据仍全部 `NOT EXECUTED`，
verdict 仍是 `undetermined`。** 这一轮解开的是「没有可安装的移动产物」这条阻塞，
解不开「判据要的测量值必须来自应用内部」。

同时**发现一个只有真机才暴露的产品缺陷**：云笺在物理 Android 上启动后立刻
`SIGABRT` 崩溃。它此前不可能被发现——桌面测试、模拟器与 `cargo check` 都不会触发。

## 一、三个「未确认点」的实测答案

| 待确认点                      | 实测答案                                                                                                                                                                                                                                                                                                                                                         |
| ----------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `standard:7.0` 的 Android SDK | **完全不带**。`ANDROID_HOME` / `ANDROID_SDK_ROOT` / `ANDROID_NDK_HOME` 三个变量全未设置，`find / -maxdepth 6 -name sdkmanager` 零命中。必须自装，自装后 `sdkmanager` 报 `12.0`                                                                                                                                                                                   |
| NDK 磁盘占用                  | `ndk;26.3.11579264`（即 **r26d**）**2.1 GiB**；含 platforms/build-tools/platform-tools/cmdline-tools 的整个 SDK **2.5 GiB**；四组件一次装完 **22 秒**。容器 300 GiB、装前剩 129 GiB、装后剩 125 GiB——**磁盘不是瓶颈，富余两个数量级**                                                                                                                            |
| `tauri android init` 与守卫   | **不冲突，且已用「跑那道守卫」证明。** init 后 `git status --porcelain` **全空**；`crates/yunjian-mobile/Cargo.toml` 逐字节未变；`cargo test -p yunjian-mobile --test architecture` **5 passed / 0 failed**，含 `undetermined_verdict_builds_neither_binding_branch`。init 生成的 `crates/yunjian-app/gen/android/` 已在 `.gitignore` 内，与守卫读取的文件无交集 |

`init` 还主动放弃改写 `crates/yunjian-app/Cargo.toml`，原话是
`` `tauri` dependency has workspace inheritance enabled. The features array won't be
automatically rewritten. Expected features: [] `` —— 工作区继承挡住了它的自动改写，
而它期望的附加特性本来就是空集。

## 二、产物

| 项         | 值                                                                   |
| ---------- | -------------------------------------------------------------------- |
| APK        | `325,613,048` B（310.5 MiB），sha256 `c790c841…dad1bd`               |
| 其中原生库 | `lib/arm64-v8a/libyunjian_app.so` `317,732,384` B                    |
| 测试包     | `spike-tests.zip` `4,549` B，sha256 `895390c0…da14ab`                |
| 签名       | `CN=Android Debug`（Gradle 调试签名），SHA-256 `78e778a5…1e5b95`     |
| ABI        | `native-code: 'arm64-v8a'`，单 ABI                                   |
| minSdk     | `sdkVersion:'24'`                                                    |
| 构建耗时   | 约 18 分钟（含自装 SDK、`cargo install tauri-cli`、Gradle 首次拉取） |

APK 体积 310 MiB 全部来自 **debug 未优化的原生库**（`libyunjian_app.so` 占 97.6%）。
Device Farm 的 APK 上限是 4 GB，因此不构成阻塞；release + strip 的体积归 todo 70 度量。

## 三、真机执行结果

run `16d01317-cd38-4c8a-ba17-b17e85f4e42c`，`COMPLETED` / `PASSED`，**1.09 device-minutes**，
走免费额度，**增量费用 $0**。

> [!WARNING]
> **Device Farm 报的 `PASSED` 不代表云笺可用。** run 的 `PASSED` 只说明 testspec 里的
> 命令都退出 0，而同一轮的测量值明确写着 `app_launch crashed=true`。
> 这正是 `problems.md` 记录的「退出码 0 不等于验收通过」，本轮又遇一次。

### 新增的确定事实：云笺真的装进了物理设备

```
YUNJIAN-MEASURE app_install device_model=Pixel 8
YUNJIAN-MEASURE app_install os_build=15/35
YUNJIAN-MEASURE app_install package=top.onethinker.yunjian
YUNJIAN-MEASURE app_install installed=true
```

PR #99 的两轮探针用的是 AWS 自带的 `DefaultAndroidApplicationForDeviceFarm.apk`，
云笺从未上过真机。**这一条是本轮相对上一轮真正新增的事实**，它对应上一轮那句
「池里有真机 ≠ 真机上装过云笺」。

### 产品缺陷：启动即 SIGABRT

```
I RustStdoutStderr: INFO yunjian_core::logger: 日志已初始化 … dir=/data/user/0/top.onethinker.yunjian/cache/yunjian/logs
I RustStdoutStderr: INFO yunjian_app: 云笺桌面端启动 app="yunjian" version="0.1.0"
I RustStdoutStderr: thread '<unnamed>' (26631) panicked at ndk-context-0.1.1/src/lib.rs:72:30:
I RustStdoutStderr: android context was not initialized
I RustStdoutStderr: attempt to unwind out of `rust` with err: Any { .. }
F libc    : Fatal signal 6 (SIGABRT), code -1 (SI_QUEUE) in tid 26631
W ActivityTaskManager: Force finishing activity top.onethinker.yunjian/.MainActivity
```

**先成立的部分不能忽略**：配置读取、日志初始化、日志目录落到 Android 应用私有 cache
（`/data/user/0/top.onethinker.yunjian/cache/yunjian/logs`）全部成功，`libyunjian_app.so`
被 `nativeloader` 正常加载，`MainActivity` 在 `+489ms` 内 `Displayed`。
崩溃发生在这些之后。

**根因已定位，不是猜的。** tombstone 的栈是
`yunjian_app::stop_unwind::<run>` ← `__start_app` ← `tao::ndk_glue::create`，
即 panic 发生在 `run()` 内部，由 tao 的 `stop_unwind` 在 FFI 边界转成 `abort()`。
而 `cargo tree -i ndk-context --target aarch64-linux-android` 给出唯一依赖链：

```
ndk-context v0.1.1
└── android-native-keyring-store v1.0.0
    └── yunjian-ai
        └── yunjian-app
```

对应 `crates/yunjian-app/src/ipc.rs` 的 `AppState::new` 第一行
`KeyStore::open(KeyStoreConfig::default())`，它由 `ipc::configure_builder` 调用，
位置正好在那两行日志之后、`.setup()` 之前——与崩溃时序完全吻合。

**这段代码本来就打算优雅降级**（`unwrap_or_else` 到 `KeyStore::session_memory`，
warn 文案是「无法打开系统密钥存储，退化到本进程内存」），**但 panic 不是 `Err`**，
所以既有的降级分支根本没机会执行。

一条排除性证据：`ndk-context` 的另一个使用者是 `cpal`，只在 `voice` 特性下进入依赖图，
本轮没开 `voice`，因此与本次崩溃无关。

三处独立证据互相印证，不是单一信号：测量值 `crashed=true`、logcat 的 tombstone 与
`Force finishing activity`、以及 13 秒后的截图停在启动器桌面
（`docs/reports/mobile-qa/device-farm-launch-2026-08-15.png`，已实际打开查看，
不是只引用它的数字）。

**修这个缺陷需要先做一个产品决定，因此本轮不改**：移动端的 API Key 该存哪里。
把 panic `catch_unwind` 掉会让密钥静默退化成「仅本进程内存」，即重启后不再保留——
这是用户可见的凭据存储行为变化，属于安全边界，不应由本任务单方面决定。
另一条路是把 `KeyStore::open` 延迟到 android context 就绪之后再惰性打开。
**两条路都不会改变任何判据的 verdict**（见下一节），所以适合单独立项。

## 四、四条判据为何仍是 NOT EXECUTED

| 判据                        | verdict      | 已实测到                                                                          | 仍缺（设备侧原话）                                                                                                                                                                         |
| --------------------------- | ------------ | --------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `microphone_capture`        | NOT EXECUTED | `device_model`、`os_build`、麦克风硬件特性存在                                    | `sample_rate_hz` / `channel_count` `needs_in_app_audiorecord`；`rms` `needs_in_app_audiorecord_and_device_farm_has_no_audio_injection`；`permission_plugin` `needs_in_app_instrumentation` |
| `corpus_materialization`    | NOT EXECUTED | `device_model`、`os_build`、`/data` 余量与公网可达                                | 六项全部 `needs_in_app_production_fetch`                                                                                                                                                   |
| `chinese_ime`               | NOT EXECUTED | `target_sdk=36`、`zh_subtype_count=1`、`runtime_locale=zh-CN`、默认 IME 为 Gboard | `entered_text` / `keyboard_overlap_px` / `input_visible` / `visual_viewport_updated` 均 `needs_in_app_instrumentation`                                                                     |
| `ios_testflight_submission` | NOT EXECUTED | 无                                                                                | 用户已决定不发商店；与 Device Farm 无关                                                                                                                                                    |

**为什么这些测量值宿主侧拿不到，是实测而非推测**：设备上不存在
`tinycap` / `tinyplay` / `tinymix` / `arecord`（`/system/bin/tiny*: No such file or directory`），
所以 PCM 只能由应用内 `AudioRecord` 采集；语料物化要走生产下载路径，那条路径在应用进程内；
键盘遮挡与 `visualViewport` 是 WebView 内部的量。三者都必须由 APK 内的 instrumented 测试上报，
而**那三个测试类在仓库里并不存在**——上一版 testspec 直接 `am instrument` 调用
`SpikeMicrophoneTest` / `SpikeCorpusTest` / `SpikeImeTest`，若照那样调度，
失败信息会像「设备有问题」，掩盖真实原因「我们没写那三个测试」。本轮已把 testspec 改成
「测得到的如实测，测不到的显式标注缺什么」。

**顺带测出一个确定的阈值不符**：判据③要求 `target_sdk == 35`，真机上实测 `target_sdk=36`
（`nativeloader` 也独立印证 `target_sdk_version=36`）。原因是 Gradle 在构建时自行装了
`Android SDK Platform 36` 并以此为 targetSdk。这一项**目前不是 FAIL**，因为该判据的其余
必需项还没测到，判据整体停在 NOT EXECUTED；但它是一个已知的、必须处理的缺口：
要么把生成工程的 targetSdk 钉到 35，要么修订判据阈值。

**修掉启动崩溃也不会让任何判据变 PASS**：上面每一条缺失项都需要仓库里尚不存在的
应用内 instrumentation 代码，与应用是否活着无关。这是本轮把崩溃单独立项、
而不是顺手 `catch_unwind` 的第二个理由。

## 五、顶层 verdict：仍是 `undetermined`，且理由与上一轮不同

方案要求四条**全 PASS** 才选 `tauri_mobile`，**任一 FAIL** 选 `uniffi_native`。
本轮四条全为 `NOT EXECUTED`，机械导出 `undetermined`。

判据④（iOS TestFlight）**既不是 PASS 也不是 FAIL**：用户已决定不发商店。
把它当 FAIL 会让 `uniffi_native` 被一个从未执行的结论选中——那与编造 PASS 同样是伪造，
只是方向相反。把它当 PASS 则是直接伪造。所以顶层只能是 `undetermined`。

**但这里有一个必须写明的语义区别**：「因缺条件而未执行」会随条件补齐而自行解除，
而「用户决定不做」不会。判据④永远不会有测量值，因此**这道门禁按现有措辞已不可能达成**。
判据①的阈值同样写着「Android 与 iOS **均**满足」，在 iOS 侧产不出 `.ipa`（缺 macOS 与
Xcode）之前，它也不可能 PASS。也就是说四条判据里有两条依赖 iOS，而 iOS 链路未打通。

这不是 harness 能自己解决的，需要用户在两条路里选一条：

1. **修订判据集**：明确把④移出门禁、并把①的阈值拆成 Android 与 iOS 两条独立判据。
   门禁随即可由 ①(Android)②③ 判定，前提是补齐应用内 instrumentation。
2. **接受门禁长期 `undetermined`**，用别的依据决定移动端框架。

**在用户做出选择之前，harness 保持 `undetermined` 是正确行为，不是缺陷。**

## 六、本轮为让链路跑通而修的东西

| 症状（实测原话）                                                               | 真因                                                                        |
| ------------------------------------------------------------------------------ | --------------------------------------------------------------------------- |
| `could not find menu in tauri` + `the item is gated here`                      | `tauri::menu` / `tauri::tray` 是 `#[cfg(desktop)]`，移动端不存在            |
| `expected UpdateTarget, found ()`                                              | `UpdateTarget::current()` 只覆盖三个桌面目标，Android 上没有任何分支返回    |
| `Library artifact not found … libyunjian_app.so`                               | `[lib]` 缺 `crate-type`，Tauri 移动端要 `cdylib`                            |
| `error: no such command: tauri`                                                | npm 装的是独立可执行文件，不是 cargo 子命令                                 |
| `Cannot find module '<crate 目录>/tauri'`（Gradle `:app:rustBuildArm64Debug`） | `android init` 把「当初怎么调用 CLI」烧进生成工程；npm 全局 shim 之后找不到 |

最后一条最值得记：**Rust 那一步在它之前已经成功**（`Finished dev profile in 1m09s`，
`libyunjian_app.so` 已 symlink 进 `jniLibs`），失败点与真因隔着一个阶段。
所以 `.aws/buildspec-android-spike.yml` 里 `cargo install tauri-cli` 那 4 分钟编译是
**刻意选慢**，注释已写明不要「优化」回 npm。

另一条被实测推翻的假设：`tauri android build` 会向 Gradle 注入
`CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=…/aarch64-linux-android24-clang`，
**把 buildspec 里那份 android26 导出顶掉**。对本轮无害（未开 `voice`，依赖图里没有 cpal，
用不到 AAudio），但 todo 70 打开 `voice` 后必须改 minSdk 让 Tauri 自己选对链接器，
在 buildspec 里导出变量是无效的。

## 七、更新后的完整前置清单

1. ~~验证 `.aws/buildspec-android-spike.yml`，产出 `target/mobile/yunjian-spike.apk`~~ **已完成**
2. **修启动崩溃**（先定移动端凭据存储策略），见第三节
3. 实现 APK 内的 instrumented 测试并上报测量值；`androidTest` 源集与
   `assembleAndroidTest` 目前都不存在
4. 把生成工程的 targetSdk 钉到 35，或修订判据③的阈值
5. 由用户裁决判据集是否修订（第五节两条路）
6. 把 Android 设备池换成 `Xiaomi Redmi Note 13`（Android 15）以贴合判据②的「中端」
7. 建 `MAC_ARM` CodeBuild 项目并验证 `tauri#15066`；不成立则 iOS 侧维持 NOT EXECUTED

## 八、本轮成本

| 项           | 值                                                                                                                                                                                |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| CodeBuild    | 7 次构建（1 次侦察 + 6 次冒烟迭代），累计约 62 分钟 `BUILD_GENERAL1_LARGE` on-demand，约 **$1.2**                                                                                 |
| Device Farm  | 1 轮 run，**1.09 device-minutes**，走免费额度，**$0**                                                                                                                             |
| 免费额度余量 | 约 **956.4** device-minutes                                                                                                                                                       |
| 临时 IAM     | `yunjian-runner` 上加了 `yunjian-runner-devicefarm-upload` 内联策略，只含 `devicefarm:CreateUpload` / `GetUpload`，作用域限本项目 ARN；如不再需要可一条 `delete-role-policy` 撤销 |

产物**不经 S3 也不经开发机**：APK 310 MiB 在构建机上直接 `create-upload` + 预签名 PUT
进 Device Farm，避免搬两遍 620 MiB。构建**刻意不做 `schedule-run`**——调度会真花设备分钟数，
那一步由人在看过签名与 ABI 输出后手动发起，对应 IAM 也只给了上传两个动作。
