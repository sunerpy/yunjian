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
