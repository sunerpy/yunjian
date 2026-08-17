简体中文 · [English](readme/VOICE.md)

# 语音

**云笺不评判读音标准。** 这不是保守措辞，而是一条实测结论加一条产品裁决，本文逐条说明它的依据、
它换来了什么、以及当前真正具备的能力边界在哪里。

## 目录

- [不评判读音标准（本文最重要的一条）](#不评判读音标准本文最重要的一条)
- [为什么：文言 ASR 的 CER 实测 77.01%](#为什么文言-asr-的-cer-实测-7701)
- [v1 反馈契约：跟读，不是机器评分](#v1-反馈契约跟读不是机器评分)
- [模型与许可](#模型与许可)
- [许可格局：开启语音的分发件整体按 GPL-3.0](#许可格局开启语音的分发件整体按-gpl-30)
- [破读词表与它的公有领域取材规则](#破读词表与它的公有领域取材规则)
- [词谱句式表：覆盖闭合，但依据类型不是词谱](#词谱句式表覆盖闭合但依据类型不是词谱)
- [crate 结构与特性边界](#crate-结构与特性边界)
- [降级链：每一个失败点都回到打字练习](#降级链每一个失败点都回到打字练习)
- [尚未具备的部分（如实记录）](#尚未具备的部分如实记录)

## 不评判读音标准（本文最重要的一条）

**云笺不评判读音标准。** 产品不产出、界面不显示、文档不声称、MCP 输出也不包含任何形式的
声韵分、调型分、发音准确率或逐字读音评级。

这条声明由**类型边界**而不是文档纪律强制。全工作区搜索没有 `pronunciation_score`、
`phoneme_score`、`tone_score`、`声韵分`、`调型分` 这类字段、类型或函数；真正的机制是：

- `VoicePracticeFeedback` 的定义原文就是「不含字准、漏字列表或自动评级的语音练习反馈」，
  字段**只有三个**：`spoke: bool`、`pause_count: usize`、`relative_rhythm: RelativeRhythm`。
  测试 `voice_feedback_exposes_only_activity_pauses_and_relative_rhythm` 只访问这三个。
- `source_guards_keep_voice_derived_text_out_of_typed_scoring` 扫描分发源码，**禁止**六种把语音
  派生类型偷渡进打字评分的写法：`impl From<BiasedHyp> for TypedAttempt`、
  `impl Into<TypedAttempt> for BiasedHyp`、`impl Deref for BiasedHyp`、
  `impl AsRef<TypedAttempt> for BiasedHyp`、`impl From<VoicePracticeFeedback> for TypedScore`、
  `impl From<TypedScore> for VoicePracticeFeedback`；并禁止 ASR / 应用路径调用
  `TypedAttempt::new`。
- 两条 **compile-fail** 用例把它钉在编译期：`voice_feedback_cannot_be_graded.rs` 试图
  `grade_typed(feedback, ...)`（参数要求 `&TypedScore`，编译不过）；
  `voice_feedback_cannot_be_a_typed_score.rs` 试图 `feedback.into()`，期望错误明确是
  `TypedScore: From<VoicePracticeFeedback>` 未实现。
- `verdict_has_exactly_two_possible_values` 断言 `scoring_mode` 只能取两个值，并直接写
  `assert_ne!(v, "full", "字准永远不得升为正式评分")`。

**打字路径不受这条约束影响**——它是确定性比对，字准照常作为分数。两条路径由类型隔离，不是由
一个布尔开关隔离，正是为了让「语音结果不能进入打字评分」这件事编译期就成立。

## 为什么：文言 ASR 的 CER 实测 77.01%

依据是 [`docs/reports/asr-cer.json`](reports/asr-cer.json) 的实测数据，不是估算：

| 项             | 值                                                                        |
| -------------- | ------------------------------------------------------------------------- |
| 总 CER         | **0.7701488252278123（77.01%）**                                          |
| 测量语句数     | 1800（计划 1800，全部测到）                                               |
| 诗数 / 体裁数  | 50 / 8                                                                    |
| 阈值           | 0.1（10%）                                                                |
| TTS 模型       | `vits-melo-tts-zh_en`、`kokoro-multi-lang-v1_0`                           |
| ASR 模型       | `sherpa-onnx-whisper-tiny` / `-base` / `-small`                           |
| 信道条件       | `clean`、`narrowband-8k`、`pink-20db`、`pink-10db`、`slow-110`、`fast-90` |
| 是否用真人录音 | **否**（`human_recordings_used: false`）                                  |
| `scoring_mode` | `guided_practice`                                                         |

**而 77.01% 是乐观上界，不是估计值。** 报告自己写明了这一点：合成音只有单一说话人、没有口音、
没有吞音、没有真实房间的混响与远场衰减，韵律也比人朗读规整；增强逼近的是**信道与语速**，
逼近不了说话人差异。真人只会更差。

**乐观上界的用途是单向的**：它足以**证伪**（上界都超阈值 7.7 倍，真人只会更差），
不足以**证成**（上界达标不代表真人达标）。

为什么会这么差是可解释的，不是测量错误：通用普通话 ASR 的语言模型与文言正面冲突——词汇语法
完全不同、破读一律读错、生僻字密集。

**这对语音评分意味着什么：**原方案的语音评分建立在「ASR 转写 → 与原文对齐 → 算完整度」之上。
CER 77% 时这条链的**对齐输入是噪声**，因此不只是字准不能报，**连完整度都不可靠**。所以早期
把 verdict 定为 `completeness_only` 本身是错的——它假设开放转写还能支撑完整度，而那从未被测量过。
该 verdict 已撤销。

之所以只能这样测：唯一公开的中文朗诵语料 `MCGA` 是 CC BY-NC-SA-4.0 且只放了 test split，
NC 条款排除本项目（见 [`corpus/DENYLIST.md`](../corpus/DENYLIST.md)）。真人 CER 因此**未测**，
`docs/reports/asr-cer-human.md` 是留给日后贡献的位置，**明确不作为门禁**。

## v1 反馈契约：跟读，不是机器评分

v1 的语音交互是**跟读**：逐句 TTS 示范 → 用户复诵 → 反馈只含「是否开口、停顿、相对节奏」→
结束后**由用户自选** FSRS 等级。

这不是砍掉语音——语音交互完整存在，只是把「机器自动评分」换成「示范 + 节奏反馈 + 用户自评」。

`VoicePracticeFeedback` 的三个字段就是反馈的全部：

| 字段              | 类型             | 含义               |
| ----------------- | ---------------- | ------------------ |
| `spoke`           | `bool`           | 是否检测到用户开口 |
| `pause_count`     | `usize`          | 检测到的停顿次数   |
| `relative_rhythm` | `RelativeRhythm` | 相对于示范音的节奏 |

`RelativeRhythm` 只有三个变体，刻意没有数值：`Slower`（比示范音更慢）、`Similar`（大致相近）、
`Faster`（更快）。

FSRS 等级由用户直接选择，取值是 `FsrsGrade` 的四个变体 `Again` / `Hard` / `Good` / `Easy`
（常量 `ALL` 声明为「所有可由用户直接选择的等级」）。**语音路径上不存在自动 FSRS 评级**，
由测试 `schedule_source_has_no_voice_automatic_grading_path` 强制——它检查调度器的生产代码
不含 `VoicePracticeFeedback`、`RelativeRhythm`、`spoke` 或 `pause_count`。

会话编排有一条**不可省略**的约束：**播放与录音必须不重叠**（播一行 → 停播 → 录复诵）。
否则系统会完美识别自己的扬声器输出，得到一个虚假的 100% 覆盖。

`scoring_mode` 的取值域只有两个，**永远不含 `"full"`，也不再含 `"completeness_only"`**：

- `guided_practice`——v1 契约，即上述跟读形态；
- `coverage_advisory`——**仅在**一个独立的关键词检出（KWS）spike 通过事先冻结的门槛后才允许开放，
  且即使开放也只能显示「检测到 3/4 句」这类粗粒度信息，**不恢复逐字准确率、漏读字符列表或自动
  FSRS 评级**。

两条 honesty gate 守住这个取值域：`verdict_has_exactly_two_possible_values` 判构造出的 verdict，
`the_shipped_report_declares_a_legal_scoring_mode` 判落盘报告。

## 模型与许可

**不随包分发任何模型权重。** 权重按需下载并本地校验；仓库里只有身份与许可记录
（[`models.toml`](../models.toml)，由 `cargo run -p xtask -- verify-models` 强制校验）。

判定粒度是**单个发布包**，不是模型族——一个上游 repo 的声明只覆盖它自己的产物，覆盖不了它转换或
再分发的东西。这与语料用的是同一条规则。**`license` 字段本身不被信任**：门禁会打开随仓证据文件
核对 SPDX 标记，字段与证据不符即失败。

| 发布包                                                       | 类型 | 角色       | 许可       | 证据形态           | 原始权重                                      | 压缩包大小    |
| ------------------------------------------------------------ | ---- | ---------- | ---------- | ------------------ | --------------------------------------------- | ------------- |
| `sherpa-onnx-whisper-tiny`                                   | asr  | production | MIT        | `upstream_license` | OpenAI Whisper tiny                           | 116,204,861 B |
| `sherpa-onnx-whisper-base`                                   | asr  | production | MIT        | `upstream_license` | OpenAI Whisper base                           | 207,557,382 B |
| `sherpa-onnx-whisper-small`                                  | asr  | production | MIT        | `upstream_license` | OpenAI Whisper small                          | 639,387,718 B |
| `sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20` | asr  | production | Apache-2.0 | `model_card`       | `pfluo/k2fsa-zipformer-chinese-english-mixed` | 511,274,346 B |
| `vits-melo-tts-zh_en`                                        | tts  | production | MIT        | `package_license`  | MyShell.ai MeloTTS                            | 167,006,755 B |
| `kokoro-multi-lang-v1_0`                                     | tts  | production | Apache-2.0 | `package_license`  | `hexgrad/Kokoro-82M`                          | 349,418,188 B |
| `kitten-nano-en-v0_2-fp16`                                   | tts  | **smoke**  | Apache-2.0 | `package_license`  | `KittenML/kitten-tts-nano-0.2`                | 26,586,708 B  |

逐项摘要、锁定的 `license_rev`（40 位 commit SHA，分支名会移动等于没锁）与随仓证据副本的
SHA-256 全在 `models.toml` 里；分发用的署名副本在 [`licenses/`](../licenses/)，
由 `licenses_directory_holds_a_file_for_every_manifest_entry` 断言与证据副本逐字节一致。
完整的第三方许可清单见 [`LICENSES.md`](../LICENSES.md)。

**三条必须知道的限定：**

- **Whisper 转换包自己不带 LICENSE。** 证据是 `upstream_license`：OpenAI 的 `openai/whisper`
  LICENSE 明写权重按 MIT 发布，ONNX 导出是格式转换，MIT 条款随之传递。这条链写在 note 里，
  不靠读者自己推。
- **`kitten-nano-en-v0_2-fp16` 是英文冒烟模型，不进中文产品路径。** 它留在清单里只因为
  「产品用到的每个权重都要有许可记录」这条规则不设例外。
- **流式识别许可通了，实现没通。** `sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20`
  是整个流式候选集里唯一许可链两端都在的包，但 `sherpa-rs` 0.6.8 只封装了 offline 识别器；
  它的 CER 是 **NOT MEASURED**。

**核实推翻了研究阶段的一条结论。** 原结论是「ASR 侧的许可整体健康」，实测为假：六个候选里只有
两个族的许可能确认为 MIT 或 Apache-2.0。FunASR 系（SenseVoice / Paraformer）走阿里自家的
_FunASR Model Open Source License Agreement v1.1_，**既不是 MIT 也不是 Apache-2.0**；多数流式
zipformer 转换包在上游没有任何许可声明。逐条判定与证据在
[`models/DENYLIST.md`](../models/DENYLIST.md)，被拒的包含 `matcha-icefall-zh-baker`（训练数据
非商用）、整个 `vits-zh-hf-*` 系列（11 个包，无任何许可声明）、`aishell3`、`edge-tts`（不是可
离线分发的权重，而是对未公开端点的调用）、`MCGA`、`SenseVoice` / `sense-voice` / `paraformer`、
四批无声明的流式包，以及 `vosk-model`（**许可干净但按能力排除**：实测 CER 23.54 太差，
列在清单里正是为防它日后被当作「许可干净就能用」重新引入）。

## 许可格局：开启语音的分发件整体按 GPL-3.0

**这是可验证的事实，不是推测。** 预编译的 `libsherpa-onnx-c-api.so` 里有 **50 个 `espeak_*`
导出符号**（`nm -D --defined-only ... | grep -c espeak` == 50，**大小写敏感**；用 `-i` 会因
`OfflineSpeakerDiarization` 里的 `neSpeak` 多算 14 个）。sherpa-onnx 的
`SHERPA_ONNX_ENABLE_TTS` 默认 `ON`，会把 `csukuangfj/espeak-ng` 编进来，而那个 fork 的
`COPYING` 是 **GNU GPL Version 3**。

许可链其余部分是干净的：`sherpa-rs` / `sherpa-rs-sys` MIT（已核实）、sherpa-onnx Apache-2.0、
onnxruntime MIT。但**分发的二进制里存在即触发义务**，它在中文合成路径上是否被实际调用无关紧要。

**当前落地的处置：把 `voice` 特性开关直接当作许可边界。**

- `voice` cargo 特性**默认关闭**（`default = []`）。默认构建是纯 MIT，实测不链接任何 onnxruntime
  （`ldd target/release/yunjian | grep -i onnx` 为空）。
- **开启 `voice` 的分发件整体须按 GPL-3.0 条款提供**（源码可得、不得附加限制）。MIT 单向兼容
  GPL-3.0，所以这不是许可冲突。
- 因此发布产物分两种：默认构建标 MIT，语音构建标 GPL-3.0。

**两个 TTS 包还各自夹带 GPL-3.0 的 espeak-ng 发音词典数据**（`models.toml` 的
`[[model.bundled]]` 逐项声明）：`kokoro-multi-lang-v1_0` 夹带 355 个文件，
`kitten-nano-en-v0_2-fp16` 同样夹带。`vits-melo-tts-zh_en` **不夹带**——中文读音走包内 `dict/`
与 `lexicon.txt`，因此它是唯一一把不牵扯 GPL-3.0 数据的中文音色。

**如实记录一条未做的验证：** 中文合成走 `lexicon-zh.txt`、不经 espeak，所以纯中文用途下
kokoro 的那份数据**理论上**可以不下发；**这一点尚未实测确认**，因此当前不据此做任何分发承诺。

## 破读词表与它的公有领域取材规则

破读（同一个字在诗词里读古音而非今音）由**项目自建**的 [`data/poyin.tsv`](../data/poyin.tsv)
承担，**不引用任何第三方现代注音资料**。

表头是 `字 / context / pinyin / 依据 / confidence`，当前 **89 个数据行**。列语义：

- `context`——句片段，`*` 表示不限语境；
- `pinyin`——带调拼音，`-` 表示只登记处置；
- `依据`——**必须同时含定位符与所据版本**；
- `confidence`——取 `rhyme_attested`、`tone_split`、`engine_default`。

三个黄金用例（也是最常被举出的三个破读）：

| 字  | context | pinyin | confidence       |
| --- | ------- | ------ | ---------------- |
| 斜  | 石径斜  | `xiá`  | `rhyme_attested` |
| 衰  | 鬓毛衰  | `cuī`  | `rhyme_attested` |
| 骑  | 一骑    | `jì`   | `tone_split`     |

**公有领域取材规则由代码强制，不靠约定。** 函数 `located_evidence` 逐行校验依据：非空、存在
卷次 / 部次 / 页码 / 样本量之一的定位符、且含「据/據」与版本词；不满足即生成
`LexiconError::Unlocated` 并**点出行号**。冗余守门测试是
`every_poyin_row_carries_located_evidence`，负例侧另有四条：`empty_evidence_is_rejected`、
`evidence_without_a_locator_is_rejected`、`evidence_without_an_edition_is_rejected`、
`an_unlocated_row_is_rejected_with_its_line_number`。

**判据落到韵书声部一级**（《平水韵》《词林正韵》），与集评的出处标准同一严格度：一条空而存在的
依据是硬错误，不是警告。

两张配套表，各有独立用途：

- [`data/polyphone_index.tsv`](../data/polyphone_index.tsv)（表头 `字 / 兼收`，**1815 个数据行**）
  ——**独立于破读词表**的多音字候选集，判据是韵书中同字兼收多个声部或韵部。它存在的理由是避免
  循环论证：用破读词表自己检查自己的覆盖率，等于用结论证明前提。
- [`data/reading_roster.tsv`](../data/reading_roster.tsv)（表头
  `id / 选本 / 作者 / 题目 / 词牌 / 正文 / 依据`，**22 个数据行**）——写死覆盖闭合检查的作用域。
  断言的形态是「名册内每一个多音字都在 `poyin.tsv` 有一行」（`assert_coverage`，测试
  `coverage_over_the_roster_is_closed`），**不是一个覆盖百分比**——百分比可以靠扩大分母变好看，
  闭合不行。

**覆盖范围如实说明：** 名册是 22 首，不是全部语料。表外的字走现代普通话默认读音
（`engine_default`）。破读词表这一半是达标的。

## 词谱句式表：覆盖闭合，但依据类型不是词谱

**不要读成「词谱已就位」。** 这一节记录两件必须分开看的事：覆盖范围是**闭合的**，依据类型
**不合方案原文**，而后者是一个**已查证不可消除**的缺口，不是没做。

[`data/citune_rhythm.tsv`](../data/citune_rhythm.tsv) 表头是 `词牌 / 句式 / 来源 / 依据`，
当前 **只有 2 个数据行**：

| 词牌     | 来源           | 依据（实测口径）                                                     |
| -------- | -------------- | -------------------------------------------------------------------- |
| 念奴娇   | `corpus_modal` | 《全宋词》念奴娇 135 首实测，众数句式命中 58 首占 **43.0%**，n=135   |
| 水调歌头 | `corpus_modal` | 《全宋词》水调歌头 263 首实测，众数句式命中 89 首占 **33.8%**，n=263 |

### 一、覆盖：2 支不是「差 298 支」，是闭合的

方案要求「v1 覆盖宋词三百首里出现的词牌」。**这个集合在本仓库里无法枚举**：`corpus/sources.toml`
没有任何宋词三百首资产（同一根因已记在
[`data/reading_roster.tsv`](../data/reading_roster.tsv) 顶部），仓库能知道的全部成员就是名册里
标了该选本的作品——**恰好 2 首**（苏轼《念奴娇·赤壁怀古》《水调歌头·明月几时有》），词牌恰好
是表里这 2 支。

于是覆盖检查的形态与破读那半边完全相同：`assert_tune_coverage` 断言「名册词牌集 == 本表词牌
集」，多一支词牌就报 `TuneCoverageGap`（测试 `tune_coverage_over_the_roster_is_closed`、
`a_roster_tune_absent_from_the_table_is_a_coverage_gap`）。**分母取名册而不取「三百首」**：后者
是本仓库无法枚举的集合，拿它当分母只能得到一个假的百分比。「2 支 / 321 支」这种说法比的是
《全宋词》的词牌总量，不是方案写的那个集合。

### 二、依据类型：不合方案，且已查证不可消除

方案要求依据是「公有领域词谱，含卷次页码」，实际是 `source = corpus_modal`，即《全宋词》实测
众数句式。**这是统计推断而非词谱权威**，且命中率不足一半（43.0% / 33.8%）说明「众数句式」对
同名异体的代表性有限。

**为什么不补一部公有领域词谱进来——逐条判据，可复现：**

1. `corpus/sources.toml` 共 68 条逐资产判定（46 可分发 / 22 扣留），其中**唯一一份词谱**是
   `Ci_Tunes.json`，`license_class = "unverified"`，理由是抓自商业站点 `sou-yun.cn` 且授权链
   未核实；`corpus_gate` 对「`unverified` 且 `shippable = true`」是硬失败，代码里也没有它的
   读取路径。随包韵书只有《平水韵》《词林正韵》——**韵书不是词谱**。
2. 《钦定词谱》《白香词谱》《词律》在全仓**零命中**，从来不是候选：`verify-sources` 要求每个
   上游带锁定 40 位 commit、SPDX 许可、钉到该 revision 的许可 URL、随仓许可副本及其 SHA-256，
   一份影印底本满足不了这套 schema。
3. 底本过保护期**不等于**某个数字化转录本可再分发，而凭记忆写「《钦定词谱》卷 X 页 Y」是编造
   依据，本项目硬规则禁止。

**所以这一条的诚实交付不是「把表扩到 300 支」**——用统计众数扩表只会把一处小的不合规变成 300
处大的不合规。改成：让依据类型**被机器守住**，并把差距如实写在这里。

### 三、类型混淆由解析器拦，不靠自觉

`来源` 列允许 `citune` 与 `corpus_modal`，**v1 的 `citune` 行数为 0**。在此之上
`evidence_matches_source` **双向**判依据类型，判不过是解析失败（`ProvenanceMismatch`）而非警告：

| 声明的来源     | 必须有                 | 不许有                |
| -------------- | ---------------------- | --------------------- |
| `citune`       | 词谱书名 + 卷次 + 页码 | 实测口径、`n=` 样本量 |
| `corpus_modal` | `n=` 样本量 + 实测口径 | 词谱书名、卷次、页码  |

**「不许有」那一半才是关键。** 把实测众数写成「《钦定词谱》卷五页三；据某影印本」形式上处处
合规——卷与页都是 `located_evidence` 认的定位符——语义上却把统计推断冒充成了词谱权威。
`located_evidence` 只问「能不能被第三方翻到」，答不了「这条是词谱还是统计」，所以类型必须单独
判。守着它的测试：`every_shipped_row_evidence_matches_its_declared_source`、
`a_modal_row_dressed_up_as_a_citune_citation_is_rejected`、
`a_modal_row_claiming_a_volume_locator_is_rejected`、
`a_citune_row_must_carry_a_work_name_a_volume_and_a_page`、
`runtime_only_sources_cannot_be_declared_by_a_row`，以及原有的
`v1_claims_no_citune_authority`、`only_citune_claims_authority`、
`rhythm_source_identifiers_are_stable`。

**还有一条测试守的是本项目栽过的那个坑**：`citune_rhythm.tsv` 顶部声明里逐字写着「《钦定词谱》
卷 X 页 Y」当反例，若解析把注释行也读成数据行，那段声明本身就会被算作一条词谱依据——校验照
绿而表其实是空的。`the_header_comment_block_is_not_read_as_a_row` 同时断言该反例仍在文件里、
表内恰有 2 个数据行、且没有任何一行依据引了词谱书名。

**回退机制是完整且被断言的。** `RhythmSource` 四个变体与稳定标识：`CharCount`（`char_count`）、
`CiTune`（`citune`）、`CorpusModal`（`corpus_modal`）、`Punctuation`（`punctuation`）。
`segment_ci` **只在 `spec.pattern.len() == non_empty.len()` 时**采用表中句式；词牌缺失或句数不符
一律退化为 `RhythmSource::Punctuation`。

**句数守卫是加分项，值得说明它为什么存在：** 同名异体在词里很常见，句数不符时硬套词谱会把停顿
放错位置——那比不切分更糟。两条测试覆盖它：
`a_tune_absent_from_the_table_falls_back_to_punctuation` 与
`a_clause_count_mismatch_degrades_instead_of_misplacing_pauses`。

## crate 结构与特性边界

`yunjian-voice` 的四档特性，`voice` 是许可边界（见上）：

```toml
default  = []
capture  = ["dep:rodio"]
download = ["dep:ureq", "dep:tar", "dep:bzip2"]
voice    = ["dep:sherpa-rs", "capture", "download"]
```

模块与职责（取自各自的 `//!`）：

| 模块         | 特性门                                  | 职责                                                                   |
| ------------ | --------------------------------------- | ---------------------------------------------------------------------- |
| `error`      | 无（私有）                              | 语音路径的失败面（`VoiceError`）                                       |
| `permission` | 无                                      | 麦克风权限，以及权限拿不到时的降级判定                                 |
| `platform`   | 无                                      | 五平台的系统最低版本，以及每条底线的来源                               |
| `capture`    | `capture`                               | 麦克风采集，产出识别器要求的 16 kHz 单声道 PCM                         |
| `augment`    | 无                                      | 音频增强，用窄带往返、粉噪、时间伸缩近似信道与说话人变异（CER 实测用） |
| `audio`      | 无                                      | 采集与播放的判定层，以及每一种失败如何降级到打字练习                   |
| `models`     | 无（`models::transport` 需 `download`） | 模型按需下载、许可门禁与本地缓存                                       |
| `lexicon`    | 无                                      | 破读词表、词谱句式表与朗读覆盖名册                                     |
| `prosody`    | 无                                      | 朗读节奏：音步切分、静音拼接与逐音步时间戳                             |
| `asr`        | `voice`                                 | 离线识别（Whisper 族）                                                 |
| `tts`        | `voice`                                 | 离线合成，含破读词表注入与到 `prosody::FootSynthesizer` 的接线         |

两处命名容易记错，写在这里：**注册表的类型名是 `models::Registry`，不存在名为
`model_registry` 的模块；crate 里也没有 `session` 模块。**

三条已实测的原生依赖陷阱（完整版见[语音构建](VOICE-BUILD.zh.md)）：

- **`sherpa_rs::read_audio_file` 硬断言 16 kHz**，读不了它自己 TTS 输出的 24 kHz 音频。
  「合成 → 写 WAV → 读回校验」这条最自然的验证链因此走不通；本 crate 的测试改用 `hound`。
- **`sherpa-rs` 的 `static` 特性在 Linux 上不可用**，且第二道坎是上游打包缺陷：静态发布包里
  **没有 `lib/` 目录**，`.a` 数量为 0。动态链接 + `$ORIGIN` rpath 才是可行路径。
- **动态链接 + 无 rpath = CI 全绿、发布产物退出码 127。** `cargo test` 会自动注入
  `LD_LIBRARY_PATH`，所以测试全过而 release 二进制启动失败。**「测试通过」不等于「产物能跑」**，
  CI 里为此有一个独立步骤断言 `./target/release/yunjian` 自身可启动。

## 降级链：每一个失败点都回到打字练习

**降级的终点永远是打字练习，不是零分、不是 panic、不是挂住。** 统一类型是 `Practice`，两个变体：
`Voice` 与 `Typed { reason: DegradeReason, message: String }`，后者**必须**说明原因与恢复方式。

`DegradeReason` 九个变体：`FeatureDisabled`、`SystemTooOld`、`PermissionDenied`、
`PermissionRestricted`、`PermissionUndetermined`、`NoInputDevice`、`ModelUnavailable`、
`DeviceBusy`、`CaptureFailed`。`degrade(reason, platform)` 无条件构造 `Practice::Typed`，
`explain` 为每种原因生成带恢复办法的中文消息。

逐个失败点：

1. **未编译 `voice`**——顶层 `practice` 先查 `is_available()`，false 即
   `degrade(DegradeReason::FeatureDisabled, ...)`。
2. **系统版本不足**——`Preflight::check` 先返回 `AudioError::UnsupportedPlatformVersion`。
3. **权限不可用**——`PermissionState::{Denied, Restricted, Undetermined}` 分别映射到对应
   `DegradeReason`，只有 `Granted` 才返回 `Practice::Voice`。
4. **设备与音频失败**——`AudioError` 五个变体（`PermissionDenied`、`NoDevice`、
   `UnsupportedPlatformVersion`、`DeviceBusy`、`Failed`）经**穷尽**映射 `degrade_reason` 落地。
5. **采集停摆或截断**——`recv_timeout` 超时得 `VoiceError::CaptureStalled`，channel 断开得
   `VoiceError::AudioDevice`；完成度低于 `MIN_COMPLETION` 返回 `VoiceError::CaptureTruncated`，
   **不把半段音频交给评分**。
6. **模型失败**——`ModelError` 十个变体（含 `Denied`、`LicenseRefused`、`ChecksumMismatch`、
   `SizeMismatch`）全部经穷尽映射落到 `DegradeReason::ModelUnavailable`。

四条常驻测试保证这条链不是纸面承诺，且**要求每种失败给出互不相同的消息**（否则「降级了」
与「降级原因说对了」无法区分）：
`every_audio_error_degrades_to_typed_practice_with_its_own_explanation`、
`every_failure_degrades_to_typed_practice_with_a_specific_message`、
`denied_permission_degrades_to_typed_practice_with_an_explanation`、
`restricted_and_undetermined_also_degrade_but_with_distinct_reasons`。

## 尚未具备的部分（如实记录）

- **真人 CER 未测。** 只有合成音加增强的乐观上界（77.01%），因此语音路径的字准**永久是参考值**，
  不会升为分数。缺口来自许可：唯一公开的中文朗诵语料带 NC 条款。
- **流式识别未接通。** 许可链完整的流式包已在清单里，但 `sherpa-rs` 0.6.8 无流式封装，需经
  `sherpa_rs_sys` 直调 online API。它的 CER 是 NOT MEASURED。
- **`coverage_advisory` 未开放。** 它要求一个独立的 KWS spike 先通过事先冻结的门槛
  （完整朗读句级召回 ≥95%、缺句识别 ≥95%、静音/噪声不得完整通过、无关同格律 false-complete
  ≤1%、覆盖度 MAE ≤0.10、嵌套样本覆盖度必须单调、重复不得增分、乱序必须可观察）。
  该 spike 尚未执行。另有两条已知限制会写进 UI：**KWS 结果不返回置信度**，所以只能显示
  「检测到／未确认」，**不能伪造「87% 可信」**；同音错字声学上不可区分，只能证明「预期发音序列
  被检测到」，不能证明用户说的是预期汉字。
- **词谱依据是实测众数而非公有领域词谱，且已查证不可消除。** 覆盖对名册是闭合的（2 支词牌 ==
  名册里标了宋词三百首的 2 首所用词牌），但依据类型不合方案原文——仓库内唯一一份词谱
  `Ci_Tunes.json` 授权链未核实已扣留，《钦定词谱》《白香词谱》《词律》全仓零命中且满足不了
  `verify-sources` 的 schema。类型不混淆由解析器强制（`ProvenanceMismatch`），判据见上节。
- **iOS 只能 `cargo check`。** `sherpa-rs` 声明 `crate-type = ["cdylib", "rlib"]`，cargo 会为依赖
  构建每一种声明的 lib 类型，而那个 `cdylib` 在 iOS 上链接失败（`Undefined symbols for
architecture arm64`）——iOS 侧只有静态归档，独立 dylib 本就不该被构建。**这是上游打包限制，
  不是本项目配置问题**；`bindgen` 与原生产物获取都成功、`cargo check` 通过，所以编译通路是成立的。
  要产出可用的 iOS 库需要 fork `sherpa-rs` 去掉 `cdylib`（或推动上游修）。
- **`--features voice` 的代码不在任何 clippy 门禁里。** `voice-build.yml` 只构建与冒烟，不跑
  clippy；`make lint` 不覆盖特性门后的代码。这是真实的覆盖缺口，记在此处以免被当作已覆盖。
- **四项平台验证在本机与 CI 都未执行**，各需特定条件：macOS 公证构建的运行期行为（需付费
  Apple Developer 账号与公证凭据）、macOS TCC 授权弹窗（需签名产物 + 真机）、Android 运行时权限
  对话框（需真机或模拟器）、iOS 真机采集与授权（需 macOS + Xcode + 已配置签名的设备）。
  另有一项**部分验证**：Windows 真实采集——runner 枚举到 0 个输入设备，已验证的是降级路径
  （报 `NoInputDevice` + 解释，不 panic 不挂住），真实采集需有声卡的 Windows 宿主机。

## 相关文档

- [语音构建](VOICE-BUILD.zh.md)——五平台原生依赖构建、链接方式、许可影响
- [平台要求](PLATFORM-REQUIREMENTS.zh.md)——五平台系统最低版本、麦克风授权链、低于底线时的降级行为
- [CER 报告](reports/asr-cer.md)——文言语音识别字准实测，以及它为什么只能当参考值
- [第三方许可](../LICENSES.md)——逐资产的许可与署名
