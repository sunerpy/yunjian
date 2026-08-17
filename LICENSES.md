# 第三方许可与署名

简体中文 · [English](docs/readme/LICENSES.md)

本文逐条列出云笺随包或按需下载的每一件第三方资产、它的许可、以及署名要求怎么被满足。
**每一条许可与 revision 都取自仓库内的真实清单文件**（[`models.toml`](models.toml)、
[`corpus/sources.toml`](corpus/sources.toml)、根 [`Cargo.toml`](Cargo.toml)），不凭记忆写。

云笺自身的代码按 [MIT](LICENSE) 授权（`Copyright (c) 2026 sunerpy`）。

## 目录

- [两种分发产物，两种许可](#两种分发产物两种许可)
- [语音模型权重（按需下载）](#语音模型权重按需下载)
- [夹带在模型发布包里的第三方产物](#夹带在模型发布包里的第三方产物)
- [语音原生依赖（`voice` 特性）](#语音原生依赖voice-特性)
- [语料数据源（随包）](#语料数据源随包)
- [Rust 依赖](#rust-依赖)
- [前端依赖](#前端依赖)
- [不随包、也不下载的东西](#不随包也不下载的东西)
- [署名义务怎么被满足](#署名义务怎么被满足)

## 两种分发产物，两种许可

**这一节是本文最重要的一条，不是脚注。**

| 产物                                         | cargo 特性                         | 整体许可    | 依据                                                                          |
| -------------------------------------------- | ---------------------------------- | ----------- | ----------------------------------------------------------------------------- |
| 默认构建（词典 + 打字默写 + MCP + 桌面外壳） | `voice` **关闭**（`default = []`） | **MIT**     | 实测不链接任何 onnxruntime：`ldd target/release/yunjian \| grep -i onnx` 为空 |
| 语音构建（含离线朗读与语音练习）             | `voice` **开启**                   | **GPL-3.0** | 预编译 sherpa-onnx 静态含 GPL-3.0 的 espeak-ng（见下）                        |

**依据是可验证的事实，不是推测。** 预编译的 `libsherpa-onnx-c-api.so` 里有 **50 个 `espeak_*`
导出符号**（`nm -D --defined-only libsherpa-onnx-c-api.so | grep -c espeak` == 50，
**大小写敏感**——用 `-i` 会因 `OfflineSpeakerDiarization` 里的 `neSpeak` 多算 14 个）。
sherpa-onnx 的 `SHERPA_ONNX_ENABLE_TTS` 默认 `ON`，会经 `cmake/espeak-ng-for-piper.cmake` 把
`csukuangfj/espeak-ng` 编进来，而那个 fork 的 `COPYING` 是 **GNU GPL Version 3**。

许可链其余部分是干净的（`sherpa-rs` / `sherpa-rs-sys` MIT、sherpa-onnx Apache-2.0、
onnxruntime MIT），但**分发的二进制里存在即触发义务**——它在中文合成路径上是否被实际调用无关紧要。

MIT 单向兼容 GPL-3.0，**所以这不是许可冲突**；但一份开启语音的云笺是一件结合作品，
整体须按 GPL-3.0 条款提供（源码可得、不得附加限制）。项目本身在 GitHub 上完整开源，
合规成本几近于零。

### 义务怎么随产物走

**源码可得只满足一半，声明义务要求许可原文随分发物走。** 因此两类产物各自带上许可：

| 产物                       | 随带什么                                                                                                | 落点                      |
| -------------------------- | ------------------------------------------------------------------------------------------------------- | ------------------------- |
| 命令行归档（GPL-3.0）      | [`packaging/licenses/`](packaging/licenses/) 整目录：MIT 原文、GPL-3.0 原文、`NOTICE.md` 署名与源码去处 | 归档内 `licenses/`        |
| 桌面安装包（MIT）          | 仓库根 [`LICENSE`](LICENSE)                                                                             | 安装包内 `LICENSE`        |
| Android APK/AAB（GPL-3.0） | [`packaging/licenses/`](packaging/licenses/) 整目录，同命令行归档                                       | APK 内 `assets/licenses/` |

### 移动端为什么落在 GPL-3.0 一侧

todo 69 明写 **`Voice ships on mobile in both branches`**：移动产品要与桌面同等能力，
所以 Android 构建默认开 `native-voice`（Gradle 的 `yunjian.voice`，默认 `true`）。

**边界与桌面同源，且已在 Android 产物上逐项核实**：`aarch64-linux-android` 的
`libsherpa-onnx-c-api.so` 同样有 **50 个 `espeak_*` 导出符号**
（`llvm-nm -D --defined-only ... | grep -c espeak` == 50），且它是
`libyunjian_mobile.so` 的 `NEEDED` 依赖之一，与 `libonnxruntime.so` 一起打进
APK 的 `lib/<abi>/`。**分发物里存在即触发义务**，与桌面那条判据一字不差。

要产出一份 MIT 的移动构建，用 `-Pyunjian.voice=false`：那时 `.so` 不链接 sherpa-onnx，
`startAsr` 返回「当前原生库未启用 native-voice」而不是静默降级，语音入口据此显示
具体原因。**这条路径存在，但不是默认**——默认交付完整能力。

**GPL-3.0 原文取自 `csukuangfj/espeak-ng` 的 `COPYING` 原样副本**（SHA-256
`8ceb4b9e…`），即真正约束我们所再分发的那份代码的文件，不是随便一份抄本。

这件事由 `cargo test -p yunjian-cli --test distribution_licenses` 分两层守住：
本仓测试保证载荷本身完整正确（三份文件齐全、MIT 副本与根 `LICENSE` 逐字节相同、
GPL 摘要不变、目录里不许有多余文件——工作流是整目录拷贝），发布工作流则在**打包之后
解开归档逐个核对**。后者是对真实产物字节的判断，`tar.gz` 与 `zip` 两条独立打包路径各有一份。

**桌面侧有一个 tauri-bundler 的坑值得记**：`bundle.licenseFile` 只喂给 dmg / msi / nsis
的许可页，**deb 与 AppImage 根本不读它**（`tauri-bundler` 的 `linux/debian.rs` 里
`license` 只出现在两行 SPDX 文件头注释里）。deb 唯一会复制的是 `bundle.resources`，
落到 `/usr/lib/<productName>/`，AppImage 从 deb 载荷组装因而也跟着有。
两个字段都配上，Linux 两种安装包才真的带着许可。

## 语音模型权重（按需下载）

**安装包里不含任何权重。** 权重按需下载并逐字节校验 SHA-256；仓库里只有身份与许可记录，
由 `cargo run -p xtask -- verify-models` 强制校验。

**只接受 MIT 与 Apache-2.0，没有例外。** `license` 字段本身不被信任——门禁会打开随仓证据文件
核对 SPDX 标记，字段与证据不符即失败。

| 发布包                                                       | 类型 | 角色       | 许可       | 证据形态           | 原始权重（署名对象）                                  | 压缩包字节数 |
| ------------------------------------------------------------ | ---- | ---------- | ---------- | ------------------ | ----------------------------------------------------- | ------------ |
| `sherpa-onnx-whisper-tiny`                                   | asr  | production | MIT        | `upstream_license` | OpenAI Whisper tiny                                   | 116,204,861  |
| `sherpa-onnx-whisper-base`                                   | asr  | production | MIT        | `upstream_license` | OpenAI Whisper base                                   | 207,557,382  |
| `sherpa-onnx-whisper-small`                                  | asr  | production | MIT        | `upstream_license` | OpenAI Whisper small                                  | 639,387,718  |
| `sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20` | asr  | production | Apache-2.0 | `model_card`       | `pfluo/k2fsa-zipformer-chinese-english-mixed`         | 511,274,346  |
| `vits-melo-tts-zh_en`                                        | tts  | production | MIT        | `package_license`  | MyShell.ai MeloTTS（`Copyright (c) 2024 MyShell.ai`） | 167,006,755  |
| `kokoro-multi-lang-v1_0`                                     | tts  | production | Apache-2.0 | `package_license`  | `hexgrad/Kokoro-82M`                                  | 349,418,188  |
| `kitten-nano-en-v0_2-fp16`                                   | tts  | **smoke**  | Apache-2.0 | `package_license`  | `KittenML/kitten-tts-nano-0.2`                        | 26,586,708   |

**锁定的许可证据与其摘要**（`license_rev` 是 40 位 commit SHA——分支名会移动，等于没锁）：

| 发布包                                  | 随仓证据副本                                                                         | 副本 SHA-256                                                       | `license_rev`                              |
| --------------------------------------- | ------------------------------------------------------------------------------------ | ------------------------------------------------------------------ | ------------------------------------------ |
| Whisper tiny / base / small（共用一份） | `models/licenses/openai-whisper.LICENSE`                                             | `b5d65a59060e68c4ff940e1eddfa6f94b2d68fdf58ed7f4dd57721c997e35e9d` | `5f86d1d86363843179951550570367b37c5d6f78` |
| streaming-zipformer-bilingual-zh-en     | `models/licenses/sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20.CARD.md` | `b6f9458f4208ae821beaaf11dc983486916a4089a2f3677b40f9ff06ec4e6440` | `98590b7ed6443e77b714204da2757d75e1a642f4` |
| （其上游模型卡）                        | `models/licenses/k2fsa-zipformer-chinese-english-mixed.CARD.md`                      | —                                                                  | `6eb615ae77ecac05c5628d5c8ed7037c14a338d5` |
| vits-melo-tts-zh_en                     | `models/licenses/vits-melo-tts-zh_en.LICENSE`                                        | `88a50e5a02bbc2a5c2f084dc19da751aa97b1690f5fda76cd8005c8634d1ca70` | `a0d5c6a264c0ef92d70d8661d8cc502d79627cd6` |
| kokoro-multi-lang-v1_0                  | `models/licenses/kokoro-multi-lang-v1_0.LICENSE`                                     | `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30` | `7e9b67b79bfdcbd2b4bc144370345fcceac3cb0c` |
| kitten-nano-en-v0_2-fp16                | `models/licenses/kitten-nano-en-v0_2-fp16.LICENSE`                                   | `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30` | `7d14a97d38072576ddca7a673ab5bb49c43bb169` |

**三条必须说明的限定：**

- **Whisper 转换包自己不带 LICENSE**，也未在模型卡声明 SPDX。证据形态因此是
  `upstream_license`：OpenAI 的 `openai/whisper` LICENSE 明写权重按 MIT 发布，ONNX 导出是该权重的
  格式转换，MIT 条款随之传递。**这条链写在清单的 note 里，不靠读者自己推。**
- **`kitten-nano-en-v0_2-fp16` 是英文冒烟模型**，只用于证明构建与链接通路成立，**不进中文产品
  路径、不随产品下发**。它留在清单里只因为「产品用到的每个权重都要有许可记录」这条规则不设例外。
- **`vits-melo-tts-zh_en` 与 `kokoro-multi-lang-v1_0` 的包内 LICENSE 与 HuggingFace 锁定 revision
  的字节完全一致**（实测 `cmp` 通过），所以它们的证据形态是最强的 `package_license`。

## 夹带在模型发布包里的第三方产物

发布包里夹带的第三方产物**逐项声明**（`models.toml` 的 `[[model.bundled]]`），存在 GPL 类条目时
`distribution_impact` 必填——**分发影响不能只活在某个人的记忆里**。

| 宿主发布包                 | 夹带路径         | 产物                                                    | 许可        | 分发影响                                                                                                                                                                        |
| -------------------------- | ---------------- | ------------------------------------------------------- | ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `kokoro-multi-lang-v1_0`   | `espeak-ng-data` | `csukuangfj/espeak-ng` 的发音词典数据（**355 个文件**） | **GPL-3.0** | 只是数据、不是被链接的代码，但随包分发它等于分发 GPL-3.0 材料。与「预编译 sherpa-onnx 静态含 GPL-3.0 espeak-ng」是同一个许可格局，结论一致：**语音构建整体按 GPL-3.0 条款分发** |
| `kitten-nano-en-v0_2-fp16` | `espeak-ng-data` | 同上                                                    | **GPL-3.0** | 它是冒烟模型，不随产品下发，因此不影响发布产物的许可结论                                                                                                                        |

**`vits-melo-tts-zh_en` 不夹带 `espeak-ng-data`**——中文读音走包内 `dict/`（jieba）与
`lexicon.txt`，因此它是唯一一把不牵扯 GPL-3.0 数据的中文音色。

**如实记录一条未做的验证：** 中文合成走 `lexicon-zh.txt`、不经 espeak，所以纯中文用途下 kokoro
的那份数据**理论上**可以不下发；**这一点尚未实测确认**，因此当前不据此做任何分发承诺，
上表的结论按「随包即触发」记。

## 语音原生依赖（`voice` 特性）

| 组件                                   | 许可        | 说明                                                                                                                      |
| -------------------------------------- | ----------- | ------------------------------------------------------------------------------------------------------------------------- |
| `sherpa-rs`（`=0.6.8`）                | MIT         | 第三方 Rust 绑定，已核实                                                                                                  |
| `sherpa-rs-sys`                        | MIT         | 按 `dist.json` 下载官方预编译产物；`dist.json` 把 sherpa-onnx 版本写死在 crate 版本里，所以放宽版本号等于放宽原生产物版本 |
| k2-fsa/sherpa-onnx 预编译产物          | Apache-2.0  | **但静态含下一行**                                                                                                        |
| `csukuangfj/espeak-ng`（编入上一行）   | **GPL-3.0** | 50 个 `espeak_*` 导出符号，见上文                                                                                         |
| onnxruntime（含在 sherpa-onnx 产物里） | MIT         | —                                                                                                                         |

## 语料数据源（随包）

**随包语料只包含公有领域原文，以及由 MIT 许可的上游整理产物。** 逐资产判定在
[`corpus/sources.toml`](corpus/sources.toml)，由 `xtask verify-sources` 强制校验。

| 来源                                                                                  | 锁定 revision                              | 许可 | 随仓 LICENSE 副本                                        | 副本 SHA-256                                                       | 提供什么                                                |
| ------------------------------------------------------------------------------------- | ------------------------------------------ | ---- | -------------------------------------------------------- | ------------------------------------------------------------------ | ------------------------------------------------------- |
| [`chinese-poetry/chinese-poetry`](https://github.com/chinese-poetry/chinese-poetry)   | `b8594f81a89752241442f2ce267d6f66f96704ee` | MIT  | `corpus/licenses/chinese-poetry.LICENSE`                 | `c195319aeaa3ffcbe16aa5d26eec19eae5a42f84337dd2b3dc3c9d5ccbbd6507` | 唐诗、宋词、元曲、楚辞、诗经、五代诗词、平仄（strains） |
| [`Werneror/Poetry`](https://github.com/Werneror/Poetry)                               | `4cfe49c06858e00d15f84d192fe5294295f79689` | MIT  | `corpus/licenses/Werneror-Poetry.LICENSE`                | `3c2630eb84efab60868d5195aa656b954f77d3cc1127dc886601e21cfd9fb63b` | 历代诗词（13 个分桶 CSV）                               |
| [`charlesix59/chinese_word_rhyme`](https://github.com/charlesix59/chinese_word_rhyme) | `ff0e9c13fb037c43e0eaa5dc929c0fe4fa2ffb18` | MIT  | `corpus/licenses/charlesix59-chinese_word_rhyme.LICENSE` | `e1464036d0f0ca738de9ebcb697b8faaf6dc2eafd193dc98555f23b409e87599` | 平水韵、词林正韵、逐字平仄                              |

**判定粒度是单个资产而不是仓库**——一份仓库级 MIT LICENSE 只能授予该仓库自身整理工作的权利，
盖不住它抓取或转录来的内容。清单里共 **68 条 asset 判定**：42 条 `public_domain`、
5 条 `permissive`、21 条 `unverified`。`shippable = false` 的资产一律不进入分发产物，
**`unverified` 且 `shippable = true` 是硬失败**。

**云笺自建的数据文件**（`data/` 下）不是第三方资产，随代码按 MIT 授权，但它们的**依据**来自
公有领域韵书与公有领域语料，逐行注明定位符与所据版本：

| 文件                       | 行数  | 依据类型                                                                                       |
| -------------------------- | ----- | ---------------------------------------------------------------------------------------------- |
| `data/poyin.tsv`           | 89    | 《平水韵》《词林正韵》到声部一级，逐行含定位符与所据版本                                       |
| `data/polyphone_index.tsv` | 1815  | 韵书中同字兼收多个声部或韵部                                                                   |
| `data/reading_roster.tsv`  | 22    | 选本 + 逐首依据                                                                                |
| `data/citune_rhythm.tsv`   | **2** | **`corpus_modal`：《全宋词》实测众数句式，不是公有领域词谱权威**（见[语音](docs/VOICE.zh.md)） |

历代集评种子（`corpus/commentary/sources/`）487 条覆盖 398 首诗，取自 10 部**前现代**诗话，
逐条以维基文库校录本的固定修订号为据——前现代诗话已过保护期，与现代赏析是两个法律类别。

## Rust 依赖

工作区的第三方 crate 全部来自 crates.io，许可以各 crate 自身声明为准（`cargo metadata` 可查，
`cargo-audit` 在 CI 里另有一个独立作业）。**几条与许可或合规直接相关的选型写在这里，
因为它们是刻意的决定而不是默认值：**

| crate                                  | 相关说明                                                                                                              |
| -------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `rusqlite`（`bundled`）                | `bundled` 不可移除：系统 SQLite 可能早于 FTS5 trigram 分词器（3.34.0）                                                |
| `keyring-core` + 五个逐平台 store      | **刻意不依赖 `keyring` 门面 crate**——它自己的文档写明应用不该链接它，且会把五平台的 store 一次性拖进依赖树            |
| `genai`（`0.6`，`rustls-tls`）         | 刻意不引入 `rig-core`（agent 框架）与 `llm-chain`（2023 年起停更）；写死 rustls 免掉 OpenSSL 这个 C 依赖              |
| `ferrous-opencc`                       | 只出现在 `yunjian-corpus`，**运行期不带转换字典**；刻意不用 `opencc-rust` 与带默认特性的 `zhconv`                     |
| `pinyin`（`default-features = false`） | **正确性约束而非瘦身**：`with_tone` 的声调符号是多字节非 ASCII，会让下游按字节比对的近音判据静默失效                  |
| `inputx-phonetic-edit`（`1`）          | crates.io 仅发布 `1.4.0`，其声明的 `rust-version = 1.95` 决定工作区最低 Rust 版本                                     |
| `tauri` / `tauri-build`                | **刻意不引入 `tauri-plugin-log`**：日志走 `yunjian_core::init_logger`，三个入口共用同一份级别解析、脱敏与滚动文件布局 |
| `sha2`                                 | 清单里的字段名是 `license_sha256`，必须真的是 SHA-256——换成 blake3 就无法与 `sha256sum` 及 GitHub 侧摘要互相核对      |

**最低 Rust 版本已对齐：** crates.io 元数据确认 `inputx-phonetic-edit` 只有 `1.4.0` 一个版本，
没有可保留 Rust 1.88 的旧版本可降级。根 `Cargo.toml` 因此声明 `rust-version = "1.95"`，
README 同步写明 Rust 1.95+；CI 除 stable 全量门禁外，还用精确 Rust 1.95 执行
`cargo check --workspace --all-features --locked`，防止真实下限再次静默漂移。

## 前端依赖

桌面外壳的前端在 `app/`，依赖声明在 `app/package.json`，许可以各 npm 包自身声明为准。
其中 React 与 Vite 生态均为 MIT 或 Apache-2.0。**前端不引入任何模型权重或语料。**

一条已实测的教训：**npm 包版本号同样属于「禁止凭记忆写」的标识符**——首版 `package.json`
按记忆写的 `@types/react-dom@18.3.9` 不存在（真实是 18.3.7），`npm install` 报 `ETARGET`。

## 不随包、也不下载的东西

被拒绝的资产**没有读取路径**，不只是清单上写着「不用」：

- **被拒的 17 个数据源**逐条附理由记在 [`corpus/DENYLIST.md`](corpus/DENYLIST.md)，
  `verify-sources` 会对 source 的 `name` / `url` 做子串匹配，命中即构建失败；并额外断言
  `REQUIRED_DENYLIST` 的 14 个标识符必须都在清单里，**删条目直接构建失败**。
  完整清单与逐条理由见[语料与索引](docs/CORPUS.zh.md)。
- **被拒的语音模型**记在 [`models/DENYLIST.md`](models/DENYLIST.md)，含
  `matcha-icefall-zh-baker`（训练数据非商用）、整个 `vits-zh-hf-*` 系列（11 个包无任何许可声明）、
  `aishell3`、`edge-tts`（不是可离线分发的权重，而是对未公开端点的调用）、`MCGA`
  （CC BY-NC-SA-4.0）、`SenseVoice` / `sense-voice` / `paraformer`（FunASR 协议，既非 MIT 也非
  Apache-2.0）、四批无声明的流式包，以及 `vosk-model`（**许可干净但按能力排除**）。
- **被扣留的资产在代码里没有读取路径**：`yunjian_corpus::rhyme` 只接受 `SHIPPED_ASSETS` 白名单内
  的路径，传入被扣留资产得到的是错误而不是数据。中华新韵（2005 年现代出版物）与抓自
  `sou-yun.cn` 的词谱都在此列。

## 署名义务怎么被满足

**署名副本与证据副本是两份用途不同、内容逐字节相同的东西，两者都不许手工编辑，
也不许被格式化工具改写**（见 `.gitattributes` 与 `.oxfmtignore`）：

- **`models/licenses/` 是证据**：由 `verify-models` 按 `license_sha256` 逐字节校验，
  用来证明「清单里写的 SPDX 与上游锁定 revision 的实际内容一致」。
- **[`licenses/`](licenses/) 是署名**：给分发件用，每个登记的模型都有一份它的许可原文。
  文件名是发布包名，扩展名跟着 `license_evidence` 走（`.LICENSE` 是许可全文，
  `.CARD.md` 是模型卡）。

两者一致由 `cargo test -p yunjian-voice models` 的
`licenses_directory_holds_a_file_for_every_manifest_entry` 断言钉住——不一致就意味着
「分发时给出的许可原文」与「经过校验的那一份」是两份东西。

**语料侧同理**：`corpus/licenses/` 保存三个上游的 LICENSE 副本，`verify-sources` 的
`vendored_licenses_match_recorded_hashes` 等用例校验「随仓字节 == 记录摘要」。

一条已实测的陷阱值得记下：**Windows runner 的 `core.autocrlf=true` 会让任何逐字节 SHA-256 门禁
必然误报**（LF 检出后变 CRLF，摘要必然不符）。修法是仓库根 `.gitattributes` 里
`corpus/licenses/** -text`——`-text` 比 `eol=lf` 更强，git 完全不做行尾转换，
对象库字节 == 工作树字节。**删掉那条会让多个门禁一起变红。**

## 相关文档

- [语料与索引](docs/CORPUS.zh.md)——来源、逐条排除理由、身份模型、集评准入
- [语音](docs/VOICE.zh.md)——模型与许可、破读词表、不评判读音标准
- [语音构建](docs/VOICE-BUILD.zh.md)——五平台原生依赖构建与 GPL-3.0 影响
- [AI 赏析](docs/AI.zh.md)——为什么随包数据集只用开放权重模型
