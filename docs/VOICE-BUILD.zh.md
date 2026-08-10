# 云笺语音构建与链接方案（Wave 0 定论）

本文是 todo 44 的裁决记录。todos 52、53、54、55、68、69 读这份文件而不是重新决策。

结论先行：

- **机制**：第三方绑定 `sherpa-rs`（MIT）+ 其 `sherpa-rs-sys`（MIT），由后者的
  `download-binaries` 特性按 `dist.json` 拉取 k2-fsa 官方预编译产物。不自行 vendoring
  C++ 源码构建。
- **链接方式**：**动态**。桌面三平台的静态归档实测不可用（见「静态链接为何不可行」）。
- **可执行文件体积增量**：11 920 字节；**随包动态库 19.86 MiB**（见「体积实测」）。
- **许可**：`sherpa-rs` 与 `sherpa-rs-sys` 均为 MIT，sherpa-onnx 自身 Apache-2.0，
  onnxruntime MIT——**但预编译产物里静态包含 GPL-3.0 的 espeak-ng**。这一条改变了
  产品的许可格局，需要人工决策，见「许可清单」与「必须由人拍板的一件事」。
- **五平台结论**：Linux / Windows / macOS **构建加冒烟全部通过**（后两者在 CI 宿主机上）；
  Android **构建通过**；iOS **仅 `cargo check` 通过**，`cargo build` 被上游
  `sherpa-rs` 的 `cdylib` 目标阻塞。两个移动端的设备冒烟按计划留给 todo 68 的真机
  harness。逐目标证据见「五平台结论」。

## 目录

- [为什么选预编译而不是源码构建](#为什么选预编译而不是源码构建)
- [静态链接为何不可行](#静态链接为何不可行)
- [rpath：不设它发布产物就起不来](#rpath不设它发布产物就起不来)
- [体积实测](#体积实测)
- [许可清单](#许可清单)
- [必须由人拍板的一件事](#必须由人拍板的一件事)
- [五平台结论](#五平台结论)
- [按平台前置条件](#按平台前置条件)
- [冒烟模型](#冒烟模型)
- [已知限制](#已知限制)
- [复现命令](#复现命令)

## 为什么选预编译而不是源码构建

三条候选路线里：

| 路线                                        | 判定   | 理由                                                                                                            |
| ------------------------------------------- | ------ | --------------------------------------------------------------------------------------------------------------- |
| 官方 Rust API（仓库内 `rust-api-examples`） | 否     | 无 crates.io 包，只能 vendoring 源码文件，等于自己维护一份绑定                                                  |
| 自行 `build.rs` + cmake 编译 C++            | 否     | 需要在五个平台各备一套 cmake + C++17 工具链；Android/iOS 还要 NDK/Xcode SDK。收益仅是控制力，而控制力此处用不上 |
| `sherpa-rs` + 预编译产物                    | **是** | 许可已核实为 MIT；`dist.json` 覆盖本项目全部五个目标；不需要宿主机装 cmake                                      |

`sherpa-rs` 的许可在方案研究阶段记为 UNVERIFIED，本次已核实：

- `crates/sherpa-rs/Cargo.toml` -> `license = "MIT"`
- `crates/sherpa-rs-sys/Cargo.toml` -> `license = "MIT"`
- 仓库根 `LICENSE` -> `MIT License, Copyright (c) 2024 thewh1teagle`

三处一致，符合「只用 MIT 或 Apache-2.0」的既有约束，可以使用。

版本**精确锁定** `=0.6.8`：`sherpa-rs-sys` 把 sherpa-onnx 的版本号写在自己的
`dist.json` 里，放宽 crate 版本等于放宽原生产物版本。实测已发布的 0.6.8 拉取的是
**sherpa-onnx v1.12.9**，而该仓库 `main` 分支的 `dist.json` 已指向 v1.12.15——
即 crates.io 上的版本与 GitHub `main` 不同步，只有精确锁定才能让原生产物可复现。

非默认特性 `sys` 必须开启，否则拿不到 `sherpa_rs_sys` 的重导出，也就无法调用
`SherpaOnnxGetVersionStr`——那是唯一不需要模型即可证明原生库已链接的入口。

## 静态链接为何不可行

`sherpa-rs` 有 `static` 特性，实测在 Linux 上走不通，两道坎：

1. 必须额外设 `RUSTFLAGS="-C relocation-model=dynamic-no-pic"`，否则 `sherpa-rs-sys`
   的 build script 直接 panic 要求这个变量。
2. 设了之后链接失败：`undefined symbol: SherpaOnnxGetVersionStr`。原因是
   `sherpa-onnx-v1.12.9-linux-x64-static.tar.bz2` **根本没有 `lib/` 目录**，
   只有 `bin/`（一堆预编译可执行文件）和 `include/`。归档里不含任何 `.a`，
   `sherpa-rs-sys` 因此发不出任何链接指令。

这是上游打包缺陷，不是配置问题。因此桌面平台一律动态链接。

iOS 例外：`dist.json` 对 iOS 只提供静态归档（`libonnxruntime.a` +
`libsherpa-onnx.a`），Android 只提供动态 `.so`（放 `jniLibs`，符合 Android 惯例）。

## rpath：不设它发布产物就起不来

动态链接下有一个会让 CI 全绿而发布产物启动失败的陷阱，已实测复现：

```
$ ./target/release/yunjian
./target/release/yunjian: error while loading shared libraries:
libsherpa-onnx-c-api.so: cannot open shared object file: No such file or directory
$ echo $?
127
```

`.so` 就在同一目录里。`cargo test` 会自动注入 `LD_LIBRARY_PATH`，所以
`cargo test --features voice` 全部通过；`cargo build --release` 出来的二进制却跑不起来。

修法是在**二进制所属的包**里发 rpath——`crates/yunjian-cli/build.rs`：

| 平台            | 链接参数                                        |
| --------------- | ----------------------------------------------- |
| Linux / Android | `-Wl,-rpath,$ORIGIN`                            |
| macOS / iOS     | `-Wl,-rpath,@loader_path`                       |
| Windows         | 无需设置，PE 加载器在可执行文件所在目录搜索 DLL |

必须放在 `yunjian-cli` 而不是 `yunjian-voice`：`cargo:rustc-link-arg` 只作用于发出它的
那个包自己的产物，不会从 rlib 依赖传递到最终链接步骤。

修复后：

```
$ ldd target/release/yunjian | grep -iE 'onnx|sherpa'
	libsherpa-onnx-c-api.so => .../target/release/libsherpa-onnx-c-api.so
	libonnxruntime.so => .../target/release/libonnxruntime.so
$ ./target/release/yunjian; echo $?
0
```

## 体积实测

`x86_64-unknown-linux-gnu`，`--release`，同一次测量：

| 项                                | 字节                        |
| --------------------------------- | --------------------------- |
| `yunjian` 不带 `--features voice` | 438 952                     |
| `yunjian` 带 `--features voice`   | 454 344                     |
| **可执行文件增量**                | **15 392（约 15.0 KiB）**   |
| `libonnxruntime.so`               | 15 534 400                  |
| `libsherpa-onnx-c-api.so`         | 5 216 616                   |
| `libsherpa-onnx-cxx-api.so`       | 74 176                      |
| **随包动态库合计**                | **20 825 192（19.86 MiB）** |
| **安装包净增量**                  | **20 840 584（19.87 MiB）** |

可执行文件本身几乎不变，代价全在三个动态库上——这正是动态链接的形态。安装包预算按
**每平台 +19.9 MiB** 计（模型权重不在此列，那是 todo 45/53 的账）。

`libsherpa-onnx-cxx-api.so`（74 KiB）本项目并不调用，是 `sherpa-rs-sys` 的 build script
把 `lib/` 下所有 `.so` 一并复制过来的。若要抠这 74 KiB，需在打包阶段剔除。

## 许可清单

「链接进最终产物的每一件东西」逐项：

| 产物                                                  | 许可        | 核实方式                                                    | 是否随包分发                                    |
| ----------------------------------------------------- | ----------- | ----------------------------------------------------------- | ----------------------------------------------- |
| `sherpa-rs` 0.6.8                                     | MIT         | `Cargo.toml` `license` 字段 + 仓库 `LICENSE`                | 编译进二进制                                    |
| `sherpa-rs-sys` 0.6.8                                 | MIT         | `Cargo.toml` `license` 字段                                 | 编译进二进制                                    |
| sherpa-onnx v1.12.9 本体                              | Apache-2.0  | 上游仓库 `LICENSE`                                          | 是（`libsherpa-onnx-c-api.so`）                 |
| onnxruntime（bundled，1.17 系）                       | MIT         | microsoft/onnxruntime `LICENSE`                             | 是（`libonnxruntime.so`）                       |
| **espeak-ng（fork `csukuangfj/espeak-ng`）**          | **GPL-3.0** | fork 仓库 `COPYING`                                         | **是，静态包含在 `libsherpa-onnx-c-api.so` 内** |
| Whisper tiny 权重（仅冒烟用）                         | Apache-2.0  | HF `openai/whisper-tiny` cardData                           | 否，不随仓库也不随包                            |
| Kitten nano v0.2 fp16 权重（仅冒烟用）                | Apache-2.0  | HF `KittenML/kitten-tts-nano-0.2` cardData + 包内 `LICENSE` | 否                                              |
| `crates/yunjian-voice/tests/fixtures/bundled-16k.wav` | 本项目自有  | 由 Apache-2.0 的 Kitten nano 合成，非第三方录音             | 是（随仓库，114 KiB）                           |

**espeak-ng 这一行是本次 spike 最重要的发现**，且是可验证的事实而非推测：

```
$ nm -D libsherpa-onnx-c-api.so | grep -c espeak
50
$ nm -D libsherpa-onnx-c-api.so | grep -E ' T espeak' | head -3
00000000002674f0 T espeak_Cancel
00000000002673a0 T espeak_Char
0000000000269a80 T espeak_CompileDictionary
```

sherpa-onnx 的 `CMakeLists.txt` 里 `SHERPA_ONNX_ENABLE_TTS` 默认 `ON`，而
`cmake/espeak-ng-for-piper.cmake` 会把 `csukuangfj/espeak-ng`（GPL-3.0）编进来。
因此**任何开启 TTS 的预编译 sherpa-onnx 产物都含 GPL-3.0 代码**。
它是否在中文合成路径上被实际调用无关紧要——分发的二进制里存在即触发义务。

## 必须由人拍板的一件事

上一节的后果：**分发一份开启语音的云笺，等于分发一件 GPL-3.0 的结合作品。**

MIT 单向兼容 GPL-3.0，所以这不是许可冲突，但它确实要求带语音的分发物整体按
GPL-3.0 条款提供（源码可得、不得附加限制）。三条路，建议第一条：

1. **（建议）把 `voice` 特性开关直接当作许可边界。** 默认构建不含原生库，是纯 MIT；
   开启语音的分发物标注 GPL-3.0。本项目本来就在 GitHub 上完整开源，合规成本几近于零。
   这条路需要 README 与 LICENSE 说明，以及发布流程为两种产物打不同标签。
2. 源码构建 sherpa-onnx 并 `SHERPA_ONNX_ENABLE_TTS=OFF`，只留 ASR，TTS 另找非 GPL 引擎。
   代价高，且方案已排除的引擎（`edge-tts` 等）不能回头用，目前没有现成替代。
3. 放弃离线朗读。会砍掉一项核心功能。

这条不属于本 todo 可自行决定的范围，已同时记入 `problems.md` 等待人工确认。
在拍板之前，语音仍然是默认关闭的可选特性，词典与默写产品线不受影响。

## 五平台结论

**诚实标注**：本次执行环境是 Linux，无 Windows/macOS 宿主机，无 Android NDK，
无 Xcode SDK，无物理设备。凡未真正构建成功的目标一律记为「未执行」并写明阻塞原因，
不做模拟、不填假结果。

| 目标                                                                                     | 结论 | 证据 / 阻塞原因 |
| ---------------------------------------------------------------------------------------- | ---- | --------------- |
| 开发环境是 Linux，无 Windows/macOS 宿主机、无 Android NDK、无 Xcode SDK、无物理设备，    |
| 所以四个非 Linux 目标在本机一律「未执行」。**但本 PR 的 `voice-build` 工作流真的在 CI 上 |
| 跑起来了**，于是拿到了真实结论。下表按最终状态记录，并保留本机结论作为对照。             |

| 目标                       | 结论                                                | 证据                                                                                                                                                                                                                                                             |
| -------------------------- | --------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `x86_64-unknown-linux-gnu` | **通过（本机 + CI）**                               | 构建；`cargo test -p yunjian-voice --features voice smoke` 两条用例通过（一次真实识别 + 一次真实合成，RMS 均高于静音阈值）；发布二进制 `ldd` 解析到两个原生库并以退出码 0 运行                                                                                   |
| `x86_64-pc-windows-msvc`   | **通过（CI，`windows-latest`）**                    | 本机仅因缺 `link.exe` 失败；CI 上构建 + 冒烟 + 产物启动全部通过                                                                                                                                                                                                  |
| `aarch64-apple-darwin`     | **通过（CI，`macos-14`）**                          | 本机因缺 macOS SDK 无法为 Darwin 三元组跑 `bindgen`；CI 上构建 + 冒烟 + 产物启动全部通过                                                                                                                                                                         |
| `aarch64-linux-android`    | **构建通过（CI）；设备冒烟未执行**                  | 需 `ANDROID_NDK_HOME` 指向 NDK r26+，且**必须**通过 `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android=--sysroot=<NDK>/toolchains/llvm/prebuilt/linux-x86_64/sysroot` 传 sysroot，否则 clang 回落宿主机 `/usr/include`。设备上的识别与合成走 todo 68 的真机 harness |
| `aarch64-apple-ios`        | **仅 `cargo check` 通过；`cargo build` 被上游阻塞** | 见下方「iOS 的上游阻塞」。设备冒烟同样留给 todo 68                                                                                                                                                                                                               |

工作流在 run `31366975021` 上**六个作业全绿**，含终结汇总作业。Windows 与 macOS 的
冒烟日志可见 `smoke_recognition_of_bundled_wav_yields_text ... ok` 与
`smoke_synthesis_writes_non_silent_wav ... ok`，即两个平台都真实跑过一次识别与一次合成。
两个移动端作业把每 ABI 的原生库体积写进 job summary。

### iOS 的上游阻塞

`cargo build --target aarch64-apple-ios` 失败，且**不是本项目的配置问题**：
`sherpa-rs` 声明 `crate-type = ["cdylib", "rlib"]`，而 cargo 会为依赖构建**每一种**
声明的 lib 类型；它那个 `cdylib` 在 iOS 上链接失败——
`Undefined symbols for architecture arm64` / `ld: symbol(s) not found`，
尝试产出的是 `libsherpa_rs-*.dylib`（`-dynamiclib`）。iOS 侧只有静态归档
（`libonnxruntime.a` + `libsherpa-onnx.a`），一个独立 dylib 在这里本就不该被构建。

`bindgen` 与原生产物获取在 iOS 上**都成功了**，所以问题只在那个多余的 `cdylib`。
`cargo check` 通过，证明编译通路成立。彻底解决需要 fork `sherpa-rs` 去掉 `cdylib`
（或推动上游），是 **todo 69** 必须处理的事；已记入 `problems.md`。

## 按平台前置条件

| 平台    | 前置条件                                                                                                                                                    |
| ------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 全平台  | libclang（`bindgen` 生成 C API 绑定）；首次构建需能访问 `github.com/k2-fsa/sherpa-onnx/releases` 下载原生产物（约 20-80 MiB，缓存在 `~/.cache/sherpa-rs/`） |
| Linux   | `libclang-dev`（Debian/Ubuntu）。实测本机 libclang 在 `/usr/lib/llvm-18/lib`，不在默认搜索路径上，需 `export LIBCLANG_PATH=/usr/lib/llvm-18/lib`            |
| Windows | Visual Studio 2017+ 或 Build Tools for Visual Studio，**必须勾选 Visual C++**（`link.exe`）；LLVM 并设 `LIBCLANG_PATH`                                      |
| macOS   | Xcode 命令行工具（`xcode-select --install`），自带 libclang                                                                                                 |
| Android | `ANDROID_NDK_HOME` 指向 NDK r26+；产物是 `jniLibs/arm64-v8a/` 下的 `.so`，按 Android 惯例随 APK 分发                                                        |
| iOS     | macOS 宿主机 + Xcode；iOS 走静态归档（`.a`），随 app 包静态链接                                                                                             |

不需要 cmake：走预编译路线，cmake 只有在放弃预编译改源码构建时才需要（>= 3.13 + C++17）。

## 冒烟模型

冒烟只为证明通路成立，选的是两个体积最小且许可已核实的模型。**权重不进 git**
（`.gitignore` 已排除 `/models/cache/`），模型的正式核许可与按需下载分别是 todo 45 和 53。

```bash
mkdir -p models/cache && cd models/cache
curl -sSL -O https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-whisper-tiny.tar.bz2
curl -sSL -O https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kitten-nano-en-v0_2-fp16.tar.bz2
tar xf sherpa-onnx-whisper-tiny.tar.bz2 && tar xf kitten-nano-en-v0_2-fp16.tar.bz2
```

解包后分别约 245 MiB 与 42 MiB。目录位置可用 `YUNJIAN_MODEL_DIR` 覆盖。

冒烟用例刻意**在模型缺失时失败而不是跳过**：跳过会让「没跑」冒充「通过」，
在一个专门用来证明通路成立的 spike 里那是最坏的结果。

## 已知限制

- **前置检查只在 Linux 上探测 libclang。** `clang-sys` 在 macOS 走 `xcode-select`、
  在 Windows 扫注册表与若干安装目录，那套发现逻辑无法在 `build.rs` 里如实复制。
  最初照 Linux 路径一概探测，结果 macOS / Windows / iOS 三个 CI 作业被**误判**为
  缺前置条件——本来能成功的构建被自己的守卫拦下。假阴性的守卫比没有守卫更糟，
  因此这两个平台交给 `bindgen` 自己报错（它的提示已指名 `LIBCLANG_PATH`）。
- **下游 `build.rs` 不能保证抢在依赖的 `build.rs` 之前发声。**
  `crates/yunjian-voice/build.rs` 的前置检查会给出指向本文的可读报错（消息内容与格式
  已实测验证）。cargo 并行执行 build script，所以我们的检查**能够**与
  `sherpa-rs-sys` 同时触发——实测 `aarch64-linux-android` 就是两条报错一起出现，
  其中包含我们那条缺 `ANDROID_NDK_HOME` 的提示。但顺序无保证：
  `aarch64-apple-darwin` / `aarch64-apple-ios` 只看到 `sherpa-rs-sys` 的 `bindgen` 报错。
  对 `dist.json` 未收录的目标，`sherpa-rs-sys` 给的是
  `Target <triple> not found. try to disable download-feature`——尚可读，但不指向本文。
  要保证每次都先给出可读诊断，需要在 cargo 之前插一道 preflight（Makefile 或 CI 步骤），
  留给 todo 4/6。
- **`sherpa_rs::read_audio_file` 硬断言 16 kHz**，读不了它自己 TTS 输出的 24 kHz 音频。
  任何回读路径都得绕开它；本 crate 的测试改用 `hound`。
- **静态归档缺 `lib/`**（见上文），Linux 静态链接不可行。
- **CI 上必须给 `Swatinem/rust-cache` 设 `cache-targets: false`。** 它会保留
  `sherpa-rs-sys` 的 build script 指纹却裁掉那个脚本复制出来的 `.so`，于是脚本不再重跑、
  原生库也不再出现，链接报 `unable to find library -lonnxruntime`。第一轮干净缓存不会
  暴露这个问题，第二次运行才炸。原生产物另用一层 `actions/cache` 兜住
  （`~/.cache/sherpa-rs` / `~/Library/Caches/sherpa-rs` / `~/AppData/Local/sherpa-rs`），
  这样重跑 build script 也不必重新下载几十 MB。
- **`llvm-config` 在 `ubuntu-latest` 上不存在**（`libclang-dev` 只装 `llvm-config-18`）。
  用它取 `--libdir` 会把 `LIBCLANG_PATH` 写成空串，`bindgen` 随后报
  `Unable to find libclang ... (invalid: [])`。直接 `find /usr/lib/llvm-* -name 'libclang.so*'`。
- espeak-ng 的 GPL-3.0 问题无法通过「不调用它」规避，见「许可清单」。

## 复现命令

```bash
export LIBCLANG_PATH=/usr/lib/llvm-18/lib          # Linux，按实际路径调整

# 无语音：必须成功，且不链接 onnxruntime
cargo build --workspace --release
! ldd target/release/yunjian | grep -i onnx

# 带语音：构建 + 运行 + 冒烟
cargo build --workspace --release --features yunjian-cli/voice
ldd target/release/yunjian | grep -iE 'onnx|sherpa'
./target/release/yunjian
cargo test -p yunjian-voice --features voice smoke

# 重新生成随包 WAV（需要 TTS 模型）
cargo test -p yunjian-voice --features voice -- --ignored regenerate
```
