简体中文 · [English](readme/CORPUS.md)

# 语料与索引

本文记录语料的来源、许可判定、排除理由、规范化管线、身份模型与缺陷基线。
**每一条许可与 revision 都取自仓库内的真实清单文件，未决问题如实标为未决。**

## 目录

- [上游数据源：三个，各带锁定 revision 与许可](#上游数据源三个各带锁定-revision-与许可)
- [排除清单：十七条，逐条附理由](#排除清单十七条逐条附理由)
- [为什么现代注释、译文、赏析一概不收](#为什么现代注释译文赏析一概不收)
- [规范化管线](#规范化管线)
- [身份模型：为什么 `stable_id` 必须与 `content_hash` 分开](#身份模型为什么-stable_id-必须与-content_hash-分开)
- [缺陷报告与漂移基线](#缺陷报告与漂移基线)
- [索引选型（已实测，具有约束力）](#索引选型已实测具有约束力)
- [随包工件：什么在里面，什么不在（已实测定案）](#随包工件什么在里面什么不在已实测定案)
- [韵书（已实测定案）](#韵书已实测定案)
- [历代集评：逐条出处是准入条件](#历代集评逐条出处是准入条件)

## 上游数据源：三个，各带锁定 revision 与许可

已核实可用的上游是 **3 个**（不是早期草案里写的 4 个——原定的第四个 `jkak/pingShuiYun`
实测在任何 revision 都没有 LICENSE，已拒绝，见下）。逐条取自
[`corpus/sources.toml`](../corpus/sources.toml)：

| 来源                                                                                  | 锁定 revision                              | 仓库许可 | 随仓 LICENSE 副本                                        | 副本 SHA-256                                                       |
| ------------------------------------------------------------------------------------- | ------------------------------------------ | -------- | -------------------------------------------------------- | ------------------------------------------------------------------ |
| [`chinese-poetry/chinese-poetry`](https://github.com/chinese-poetry/chinese-poetry)   | `b8594f81a89752241442f2ce267d6f66f96704ee` | MIT      | `corpus/licenses/chinese-poetry.LICENSE`                 | `c195319aeaa3ffcbe16aa5d26eec19eae5a42f84337dd2b3dc3c9d5ccbbd6507` |
| [`Werneror/Poetry`](https://github.com/Werneror/Poetry)                               | `4cfe49c06858e00d15f84d192fe5294295f79689` | MIT      | `corpus/licenses/Werneror-Poetry.LICENSE`                | `3c2630eb84efab60868d5195aa656b954f77d3cc1127dc886601e21cfd9fb63b` |
| [`charlesix59/chinese_word_rhyme`](https://github.com/charlesix59/chinese_word_rhyme) | `ff0e9c13fb037c43e0eaa5dc929c0fe4fa2ffb18` | MIT      | `corpus/licenses/charlesix59-chinese_word_rhyme.LICENSE` | `e1464036d0f0ca738de9ebcb697b8faaf6dc2eafd193dc98555f23b409e87599` |

**锁定的是 revision，不是分支。** 分支名会移动，等于没锁；`xtask verify-sources` 的联网模式
校验「上游字节 == 随仓字节 == 记录摘要」，`--offline` 只校验后两者。

**判定粒度是单个资产，不是仓库。** 清单里共 68 条 asset 判定，`license_class` 三种取值的分布是
**42 条 `public_domain`、5 条 `permissive`、21 条 `unverified`**：

- `public_domain`——底本为前现代作品，已过保护期；
- `permissive`——该仓库自身的整理或计算产物，由其 LICENSE 授权；
- `unverified`——授权链未核实（现代出版物、抓自商业站点、来源不明的现代散文）。

`shippable = false` 的资产一律不进入分发产物，而 **`unverified` 且 `shippable = true` 是硬失败**。

这条粒度规则不是洁癖，它落地时**立刻在一个 MIT 仓库内部命中 10 个文件**：
`五代诗词/huajianji/`（50 条里 48 条 `notes` 是现代白话注释）、`五代诗词/nantang/`、
`水墨唐诗/`（176 条里 152 条 `prologue` 是现代赏析）、以及 `蒙学/` 下 5 个带现代百科式
`abstract` 的文件。全部标 `unverified` + `shippable = false`。

**反例同样重要，且判据不是字段名。** `幽梦影/youmengying.json` 里 219 条 `comment` 中 209 条是
清代友人评语（「曹秋岳曰」「庞笔奴曰」），`全唐诗/authors.*.json` 的 `desc` 与
`御定全唐詩` 的 `biography` 是原书文言小传——都是公有领域，可以上架。判据只能是**文本本身是
文言原刻还是现代白话**。`御定全唐詩` 的 `notes` 结构上存在但 88 条全是空串，属于「看着危险
其实是空的」。

## 排除清单：十七条，逐条附理由

完整清单在 [`corpus/DENYLIST.md`](../corpus/DENYLIST.md)，本节逐条复述**因为理由本身就是语料
站得住脚的记录**——一份没有理由的拒绝清单下一个人只会想删掉它。

| 被拒标识符                          | 理由                                                                                                                                                                       |
| ----------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `huajianji`                         | 仓库没有 LICENSE 文件；其 `notes` 字段虽然存在但本来就是空的                                                                                                               |
| `VMIJUNV`                           | 没有 LICENSE，README 写明「仅限于交流学习」；且仓库缺失唐/宋/明/清，完整数据只放在百度网盘                                                                                 |
| `xcc3641/chinese-gushiwen`          | 没有 LICENSE；附带的 `audioUrl` 也已失效                                                                                                                                   |
| `Provinm/chinese-poetry-simplified` | 没有 LICENSE。本项目自行做繁简转换，不需要它                                                                                                                               |
| `THUNLP-AIPoet`                     | 全部数据集均无 LICENSE，且声明 "released for academic use only"，学术用途授权覆盖不了本项目的分发                                                                          |
| `THU-CRRD`                          | 同属学术用途限定。它的 `pingshui_amb.pkl` 是**唯一**的多音字韵部歧义消解数据，仍然不能用                                                                                   |
| `byj233/ChinesePoetryLibrary`       | 声明 MPL-2.0，但 README 又写「商用需授权」，自相矛盾；矛盾未澄清即不可用                                                                                                   |
| `StewartXiang/poetry_with_labels`   | GPL-3.0，会传染整个应用；内容本身也是 2017 年从古诗文网爬取的                                                                                                              |
| `sheepzh/poetry`                    | LICENSE 写 MIT，README 写「不得用于任何商业盈利行为」，自相矛盾；且内容是现代诗                                                                                            |
| `Poetry_CN`                         | OpenDataLab 数据集，平台条款写明「仅用于学术目的，请勿商用」                                                                                                               |
| `OpenDataLab`                       | 同上，**按整个来源方拒绝**，避免换个数据集名再进来                                                                                                                         |
| `yht050511/gushiwen`                | 内容爬取自古诗文网（2022-12），原站声明版权由出版者所有、非经许可不准复制                                                                                                  |
| `Tianyijian/GushiWenSpider`         | 爬虫本体，硬编码古诗文网翻译接口，产出物与上一条同源                                                                                                                       |
| `MCGA`                              | 朗诵音频语料，CC BY-NC-SA-4.0（NC 条款排除本项目），且仅发布 test split                                                                                                    |
| `jkak/pingShuiYun`                  | **在锁定 revision 处没有任何 LICENSE 文件**。计划原文列它为 MIT，实测为误：`/license` 返回 404，`LICENSE` 与 `LICENSE.md` 均 0 次提交（全仓 11 次提交），README 无许可声明 |
| `caoxingyu/chinese-gushiwen`        | 上游已消失（HTTP 404），无法核实许可                                                                                                                                       |
| `javayhu/poetry`                    | 上游已消失（HTTP 404），无法核实许可                                                                                                                                       |

三条关于这份清单的机制说明：

- **匹配只针对 source 级的 `name` / `url`，绝不针对 asset 的 `path`。** 这是刻意的：被拒的独立
  仓库 `huajianji` 与 MIT 仓库内的子目录 `五代诗词/huajianji/` 同名，拿标识符去匹配路径会误杀
  后者——后者走的是逐资产判定，不是拒绝清单。
- **只维护一份清单不够，删掉一行就能放行。** `verify-sources` 因此额外断言
  `REQUIRED_DENYLIST` 里的 14 个标识符必须都在 `DENYLIST.md` 出现，删条目直接构建失败。
  该场景已实测退出 1。
- **被拒不等于丢能力。** `jkak/pingShuiYun` 原本承担「逐字 → (声调, 韵部)」反向索引，同一能力
  改由构建期反转 `charlesix59/chinese_word_rhyme` 的 `Pingshui_Rhyme.json` 得到，反而少一个
  数据源、且索引与实际分发的韵部数据必然自洽。

## 为什么现代注释、译文、赏析一概不收

上游 `chinese-poetry` 维护者在 issue #227 给出的立场与本项目一致：尊重且不收录基于诗词演绎的
数据成果（赏析、朗读评论、翻译），但作品本身已属公有领域。issue #76 进一步界定 MIT 的范围：
那份许可只覆盖「衍生整理的工作产物」。

合法的分析内容只有两个来源：前现代**历代诗话辑评**（公有领域，逐条注明出处），以及**标注
清楚的 AI 赏析**。这就是为什么 AI 功能不是锦上添花——它填的正是版权墙留下的那个洞。

## 规范化管线

管线的形状由实测事实决定，不由方便决定。

**一、转换只在构建期发生，运行期一个转换字典都不带。** 全文索引只建在 `NormalizedRecord::body`
这一列（简体）。用户输入「國破山河在」能搜到，靠的**不是**运行期再跑一次转换、也**不是**建
第二个繁体索引列，而是同时产出的 `variant_map`：一张 `(src_char, dst_char)` 表进语料库，
运行期逐字查表改写查询。两条硬约束由此而来——不建第二个索引列（trigram 本身放大 2.2–2.6 倍，
再复制一列会让体积预算直接失效）；`yunjian-core` 不依赖任何转换 crate（`ferrous-opencc` 只在
`yunjian-corpus` 的依赖里）。

**二、原字形逐字保留，因为转换会放大上游的抄录错误。** 上游 issue #261 记录约 4,278 处疑似讹误，
其中一类是形近误录：「傅」被录成「傳」，转换后成了「传」，错误从此更难辨认。所以
`NormalizedRecord::body_original` 与输入逐字节相同；凡转换非往返稳定的记录都得到一条
`conversion_unstable`。

**三、`Script::Mixed` 在全宋诗占约 40%（143882/357448），这不是探测器过敏。** 正文真的混用
`却/卻`、`烟/煙`、`峰/峯`、`凉/涼`、`里/裏`。任何「全唐诗一律繁体」的假定都是错的。

**四、四个清洗与分类阶段，各有自己的失败模式与 reason code：**

- **修正双重切句语义。** 切句被拆成两个语义明确的函数：`split_rhyme_feet` 只切 `。！？` 与换行，
  供尾字、韵脚、尾字检索；`split_metrical_lines` 切 `，。！？；` 与换行，**仅**供体裁判定。
  合并两者会污染韵部投票——《静夜思》含逗号切出 光/霜/月/乡 四个候选，其中「月」是入声，
  与 光/霜/乡（下平七阳）不同韵部。
- **占位正文检测。** 上游有 `无正文。` 这类整条占位串，它们**不是空 body**，`empty_body` 抓不到。
  判据是拼接全文后**整串相等**（不用 substring——「空。」做 substring 会误杀正文含「空」的诗），
  reason code `placeholder_body`，命中走 quarantine 而非静默丢弃。
- **粘连句拆分。** 有的记录把多句塞进一个数组元素，会让句数、首句、尾字全部算错。按 `。！？`
  切分，且句末标点后紧跟的右引号并入该句，reason code `glued_lines`。
- **结构化体裁判定。** `poem.form` 取值 `wujue` / `qijue` / `wulv` / `qilv` / `yuefu` / `ci` /
  `irregular` / `unknown`，判定优先级显式可审计。**各句字数不齐即 `irregular`，绝不猜。**
  乐府标记是附加维度（`is_yuefu` 布尔）而**不覆盖**结构判定：《黄鹤楼》得到
  `form=qilv, is_yuefu=false`，《将进酒》得到 `form=irregular, is_yuefu=true`，两者都对。

## 身份模型：为什么 `stable_id` 必须与 `content_hash` 分开

`stable_id` 从**与内容无关**的 source locator 铸造（`mint_stable_id(identity_anchor,
first_seen_corpus_version)`），`content_hash` 由 `(author, dynasty, title, body)` 算出。两者是
**两个独立字段**，同时存在于规范记录里。

**分开的理由是可验证的事实，不是设计偏好。** 上游数据已知存在数千处待修正的错字
（issue #261 约 4,278 处），修正**一定会发生**。若身份是内容的函数，那么一次错字修正就会：

- 换掉那首诗的用户可见键；
- 于是赏析缓存与复习历史（`appreciation_shipped` / `appreciation_cache` / FSRS 记录都键在
  `stable_id` 上）全部对不上；
- 而这一切**不会报错**，只表现为「我背过的诗不见了」。

所以硬规则是：**绝不把内容派生的标识符用作用户可见键。** `content_hash` 的用途是相反的——
检测内容变了，从而在注册表里记一条 `ContentChanged`。

注册表 `corpus/id_registry.jsonl` 是 append-only 事件日志，三种合法事件：

- `Mint { source_locator, stable_id, content_hash, at_corpus_version }`
- `ContentChanged { stable_id, from_content_hash, to_content_hash, at_corpus_version }`
- `Alias { stable_id, from_source_locator, to_source_locator, reason, at_corpus_version }`

**位移探测器发现整段 id 平移时让构建失败**，而不是静默重新分配——静默重排是唯一能同时破坏
所有用户数据且不留痕迹的失败形态。

判重与身份分组是**两套刻意不同的策略**：身份分组保守，判重激进。`work_group`
（`compute_work_group(body)`，**不含作者**）让双重归属可检测——上游 issue #232 里一首《赤壁》
同时被归给杜牧与李商隐；`edition_group`（`compute_edition_group(author, body)`）标记异文而
**不删除**。**刻意不引入单一正文哈希去重**：那会把《赤壁》那类双重归属静默合并掉，而那正是
需要被看见的数据缺陷。

只有 `全唐诗/poet.*` 与 `strains/json/*` 带上游原生 `id`；`宋词`、`元曲`、`楚辞`、`诗经`、
`五代诗词` 都没有。所以「优先用上游原生 key」只覆盖一小部分，位移探测器的适用面比计划设想的大。

## 缺陷报告与漂移基线

**两个语义不同的工件，不可互换：**

- [`corpus/reports/defects.json`](../corpus/reports/) ——**一行一个 finding**。一条记录可以合法地
  产生多个 finding：同一首诗既是重出、又归属冲突、又长度可疑，就是三个。
- `corpus/reports/dispositions.json` ——**一行一条输入记录**，取值只有 `Shipped` / `Quarantined` /
  `Excluded` 三种。

**守恒式必须建立在处置台账上，不能建立在 finding 上。** 因为「保留下来的记录也会产生 finding」
与「一条记录能产生三个 finding」同时成立，`poem_count + defect_count == 输入行数` **在算术上
就是假的**——左边把一条记录数了三次。正确的不变量是：

```text
count(shipped) + count(quarantined) + count(excluded) == input_rows
poem_count == count(shipped)
```

由 `QualityReport::check_conservation` 强制。拆出 `corpus-audit.db` 后这三条等式逐字保留，
只是两端分处两个文件，由 `db::verify_conservation_across_files` 同时打开两份核对，并额外要求
两个文件自称属于同一次构建（`schema_version` / `corpus_version` / `source_manifest_sha256`
三元组相等）——否则拿一份旧审计库配一份新语料库也可能让等式凑巧成立。

**漂移基线** [`corpus/reports/baseline.json`](../corpus/reports/baseline.json) 是逐 reason code 的
期望计数加容差，由 `xtask corpus-quality --write-baseline` 生成。当前 `scope = "fixtures"`，
`input_rows = 67`、`poem_count = 54`，逐 code：

| reason code               | 期望 | 容差   |
| ------------------------- | ---- | ------ |
| `lossy_char`              | 3    | 10%    |
| `conversion_unstable`     | 1    | 10%    |
| `duplicate_in_group`      | 6    | 10%    |
| `conflicting_attribution` | 4    | 10%    |
| `suspect_length`          | 2    | 10%    |
| `unknown_dynasty`         | 0    | 10%    |
| `empty_body`              | 1    | 10%    |
| `placeholder_body`        | 1    | 10%    |
| `glued_lines`             | 47   | 10%    |
| `excluded_by_policy`      | 6    | **0%** |
| `restricted_license`      | 0    | **0%** |
| `rhyme_unresolved`        | 0    | 10%    |

两处口径必须如实说明：

- **`restricted_license` 与 `excluded_by_policy` 的容差是 0%**，因为它们是许可判定的结果，
  「多了一点点」在这里没有可接受的含义。
- **容差按整数下取整，所以小计数在实践中要求精确相等。** 基线的作用是让一次上游 bump
  不能静默降低数据质量：数字变了就必须有人解释为什么。

一条真实语料形态值得记下，它会让「逐首派生出至少一行」这类断言在真实语料上必然失败而
fixture 覆盖不到：**唐宋集合里有 176 首的正文就是一个 `。`**（上游空记录，`body` 非空所以过了
质量门禁）。覆盖判据必须用「有正文字符的首数」，且用 `content_chars` 这同一个折叠器算，
不要在 SQL 里重写一遍标点集。

## 索引选型（已实测，具有约束力）

结论：**`detail=full` + 启用 n-gram 辅助表**。机器可读的 verdict 在
[`corpus/reports/index-mode.json`](../corpus/reports/index-mode.json)，人类可读版在同目录的
`.md`；建出来的索引与之不符时构建应当失败。

两条关键实测：

- `detail=none` 与 `detail=column` 在整句五言、整句七言、繁体输入三类上 **`hits=0`**——
  这正是一条三字冒烟测试会全绿放过的静默缺陷。规划阶段按第三方博客数据倾向 `none`
  （2 倍体积节省），实测否掉了它。
- n-gram 辅助表把「明月」这类两字查询的 p95 从 4.97 ms 降到 0.074 ms（**67 倍**），
  代价是索引 2.36 MB → 28.9 MB。原因是 `%明月%` 只有两个字面字符，FTS5 推不出任何
  trigram 约束，所谓「索引 LIKE」在 1-2 字下退化成虚表全扫。

## 随包工件：什么在里面，什么不在（已实测定案）

发布物是**两个文件加一份描述**，由 `xtask corpus-build` 与 `xtask corpus-package` 产出：

| 文件                          | 内容                                   | 是否随包                      |
| ----------------------------- | -------------------------------------- | ----------------------------- |
| `yunjian-corpus-<版本>.db.gz` | 正文、作者、韵书、韵脚、异体字、元数据 | 是（211 MiB，唐宋 474162 首） |
| `corpus-audit.db`             | `defect` + `disposition` 构建期台账    | 否（CI 工件与开发者可选下载） |
| `manifest.json`               | 兼容范围、摘要、体积、实测结论         | 是（旁文件）                  |

### 两类东西被移出随包工件

**一、构建期审计台账。** `defect`（逐条数据缺陷）与 `disposition`（逐条输入记录的处置）
回答的是「这次构建丢了什么、为什么丢」，是给排查的人看的，用户一行都不需要。实测它们
合计占原文件的 **67%**（defect 50.5% + disposition 16.8%）。

拆库**没有削弱守恒**。todo 14/17 的三条等式逐字保留，只是两端分处两个文件，由
`db::verify_conservation_across_files` 同时打开两份核对，并额外要求两个文件自称属于
同一次构建（`schema_version` / `corpus_version` / `source_manifest_sha256` 三元组相等）
——否则拿一份旧审计库配一份新语料库也可能让等式凑巧成立。

**二、三张可派生的检索结构。** `ngram`、`poem_fts`、`poem_last_char` 都是 `poem.body`
的**确定性派生物**：给定同一份 `poem` 表，任何机器上派生出来的内容逐行相同。它们由应用
**首启在本机构建**（`yunjian_core::derive`）。

| 阶段                                  | 唐宋 474162 首实测        |
| ------------------------------------- | ------------------------- |
| 随包库（三者与审计表都不在）          | 603 MiB，gzip **211 MiB** |
| 首启派生 `ngram`（5573 万行）         | 487.5 s                   |
| 首启派生 `poem_last_char`（503 万行） | 30.1 s                    |
| 首启建 `poem_fts`                     | 53.1 s                    |
| **首启合计**                          | **571.8 s**               |
| 派生后运行期文件                      | 4464 MiB                  |

**这不是功能缩减。** 首启完成后，`crates/yunjian-core/tests/queries.toml` 的 37 条契约
逐条与随包时相同，两字查询照样走 `ngram_gram_idx` 覆盖索引；最差 p95 22.0 ms，预算 150 ms。
`crates/yunjian-corpus/tests/first_launch_contracts.rs` 把这句话钉成门禁，并带一条可证伪
对照：把三张结构拿掉，两字查询连准备都过不去。

### 索引裁决的语义

[`corpus/reports/index-mode.json`](../corpus/reports/index-mode.json) 描述的是**运行时应有的
索引形态**，不是随包工件的形态。构建期把 `chosen_mode` 刻进 `corpus_meta.index_detail_mode`，
首启照那一列建 `poem_fts`。裁决因此仍然有牙齿：改掉它就改掉了运行时真正建出来的索引，
37 条契约立刻变红。

### 体积预算：250 → 300 MB，且如实记录它不是被迫的

`xtask corpus-package` 在写出任何文件**之前**跑完五条中止断言（完整性 `ok`、随包库无诊断表
与派生结构、跨文件守恒成立、实测结论 `within_budget`、库的形态与结论一致），落盘后再判第六条
（最终 gzip 不超预算，超了就把刚写出的文件删掉），最后**解压回读**核对工件里的 `corpus_meta`
与 manifest 逐项一致。

预算由方案声明的 250 MB 上调为 **300 MB**。**如实记录：把上述两类东西移出后，工件实测
211 MiB，原来的 250 MB 也装得下**——上调不是为了让当前工件达标，而是留余量：语料会长
（新增公有领域来源、集评），一个刚好贴着当前产物的预算会在下一次扩充时变成假警报。

全量语料（896127 首）仍然一首不少，作为应用内可选下载。工件发布在独立的 `corpus-v*` tag
上，与应用发布 tag 分离——语料修订不该强迫发一版应用，反之亦然。

## 韵书（已实测定案）

**产品实际发两本韵书**：平水韵用于诗，词林正韵用于词。第三本中华新韵的 schema 槽位
（`rhyme_book = xinyun`）从第一天就在，但**不随包分发**。

### 逐资产判定

来源是 `charlesix59/chinese_word_rhyme`（MIT，revision `ff0e9c13`）。仓库整体 MIT，但那份许可
只覆盖它自己的整理工作，于是判定逐文件做：

| 资产                                             | 底本                      | 判定               | 实测规模                        |
| ------------------------------------------------ | ------------------------- | ------------------ | ------------------------------- |
| `data/Pingshui_Rhyme.json`                       | 平水韵，前现代韵书        | 公有领域，**分发** | 105 韵部 / 10,671 条 / 8,232 字 |
| `data/Cilin_Rhyme.json`                          | 词林正韵（清 戈载，1821） | 公有领域，**分发** | 19 部 / 5,575 条 / 5,037 字     |
| `data/Word_Tune.json`                            | 由平水韵派生的逐字平仄    | 派生物，**分发**   | 8,232 字                        |
| `data/Xinyun_Rhyme.json` 及四声版                | 中华新韵（2005 年出版）   | **扣留**           | 14 部 / 7,693 条                |
| `data/Ci_Tunes.json`                             | 抓自 sou-yun.cn 的词谱    | **扣留**           | 19.6 MB                         |
| `data/Ci_Catalog.json`、`data/Word_Explain.json` | 同上，抓自 sou-yun.cn     | **扣留**           | —                               |

扣留的资产在代码里**没有读取路径**：`yunjian_corpus::rhyme` 只接受
`SHIPPED_ASSETS` 白名单内的路径，传入被扣留资产得到的是错误而不是数据。`xtask verify-sources`
另有一道独立门禁，任何 `license_class = "unverified"` 的资产一旦被标成 `shippable = true`
就让校验失败并点名该资产。

### 未决的来源问题

两条都还没有结论，处置方式相同（扣留），理由不同：

1. **`Ci_Tunes.json`（词谱）的授权链未核实。** 上游仓库自带 `crawler/getTunes.py`，抓取目标是
   商业站点 `sou-yun.cn`。仓库的 MIT 无法为抓来的内容授权，而 `sou-yun.cn` 是否许可再分发
   **未经核实**。这是目前最完整的词谱数据（含逐字平仄、句读、换阙），扣留它有实际代价：
   todo 51 的词 句读因此改由项目自建的 `data/citune_rhythm.tsv` 承担，每行必须注明公有领域
   词谱的卷次页码。
2. **中华新韵是 2005 年的现代出版物。** 内容极可能仍在保护期，与上游仓库有没有 MIT 文件无关。

两者一旦授权链核实清楚，加入都只是**数据变更而非迁移**——枚举槽位、错误类型与查询签名都已就位。

### 三处必须知道的实现细节

**一、两本书的嵌套顺序相反。** 平水韵是 `声部 -> 韵部 -> [字]`，词林正韵是 `部 -> 声 -> [字]`。
两个解析器，一种输出行（`RhymeEntry { book, rhyme_group, tone, tone_raw, character }`）。
词林正韵把上去两声并成「仄声」，**不拆**——上游没有那个信息，拆就是编造。

**二、逐字反向索引是构建期推导的，不是引来的。** 计划原本要引 `jkak/pingShuiYun` 的
`baseCharDict.json`，todo 9 实测该仓库在任何 revision 都没有 LICENSE，故拒绝。反向索引改由反转
平水韵得到，结果与被拒仓库的记录形状等价（`临 -> [(平, 十二侵), (去, 二十七沁)]`）。这样反而
更好：索引与实际分发的韵部数据必然自洽。实测 1,823 个字归属多个不同（声调, 韵部）。

**三、`Word_Tune.json` 只作交叉核对，不作依据。** 它的 8,232 个字键与平水韵的不同字数完全相同，
可见就是后者的逐字归约，但两者有 **157 处不一致**，形态整齐：全部是反向索引判为平仄两读而
上游只写了「仄」。以「空」为例，它在上平一东、上声一董、去声一送都出现，确实两读，上游标成仄。
采信上游会把「空山不见人」判为出律，所以声调维度以反向索引为准，157 处分歧进质量报告。

### 缺书是错误，不是「不押韵」

对 `rhyme_book = xinyun` 的查询返回类型化的 `Error::RhymeBookUnavailable`，**绝不返回空结果集**。
理由是「查不到」与「不押韵」在格律上是两回事：返回空集，调用方就无从区分「这两个字在中华新韵里
不同韵部」与「我们根本没有中华新韵」，缺数据会被当成否定判断呈现给用户——那是对格律的虚假陈述。

### 两处与通行说法不符的实测数字

- **平水韵是 105 个韵部，不是通行说的 106。** 上游这份缺上声「三讲」（键从 `二肿` 直接跳到
  `四纸`）。断言写死 105，上游哪天补上会让测试失败，于是那是一次被看见的数据变更。
- **「平水韵三十韵部」这句话只在一个读法下成立**：去声部恰好 30 个韵部。逐声部是
  上平 15 / 下平 15 / 上声 28 / 去声 30 / 入声 17。

## 历代集评：逐条出处是准入条件

集评通道存在的理由是法律性的。凡带**现代**注释、译文、赏析的数据集，授权链一律立不住；
而**前现代**的诗话辑评本身已过保护期——宋人评唐诗与现代赏析在法律上是两个类别。所以随包
语料只可能是「公有领域原文 + 逐条注明出处的前现代集评 + 明确标注的 AI 赏析」这个组合，
集评是其中的第二项。

条目放在 [`corpus/commentary/sources/`](../corpus/commentary/) 下，`index.json` 是由
`cargo xtask commentary-index` 生成的聚合产物，`--check` 是它的漂移门禁。每条形如：

```json
{
  "id": "helin-001",
  "poem": {
    "author": "辛弃疾",
    "title": "念奴娇",
    "first_line": "龙山何处，记当年高会，重阳佳节。"
  },
  "text": "……辛幼安《九日》詞云：“誰與老兵供一笑，落帽參軍華發。……”意謂嘉不當從溫……",
  "citation": {
    "work": "鹤林玉露",
    "author": "罗大经",
    "dynasty": "宋",
    "work_completed_by": 1252,
    "source_note": "甲編・卷一・第 1 段；据维基文库《鶴林玉露·甲編·卷一》校录本，修订号 1545733"
  }
}
```

四条准入规则，任一不满足即**拒绝入库并报出具名理由**，不存在「校验失败」这种回答：

- **`citation` 四个字段都必填。** 只有非空 `work` 会让「《某某诗话》云」这种无法复核的引用
  溜过去，而可审计性正是这条管道存在的理由。
- **`dynasty` 必须归一到十五个前 1912 规范键。** 词表止于清，`现代` / `当代` / `民国` 在
  类型层面就进不来；`work_completed_by` 另设 1912 上界，拦住伪装成前现代书名的现代出版物。
- **`source_note` 必须同时含卷次/章节定位符与所据版本。** 定位符判据是「卷/则/条/篇/章 之一
  且相邻位置有序号」——只查关键字会让「卷帙浩繁」蒙混过关，只查数字又会漏掉「卷上」。
- **`poem` 三元组必须唯一解析到一个 `stable_id`。** 种子文件写人类可核对的
  （作者，题目，首句），构建期解析；零命中与多命中都硬失败。手写 `stable_id` 等于把内容
  地址硬编码进人工数据，上游一次重排就全错。

**绝不批量导入任何数据集的 `Comment` 类字段。** 唯一被核实含前现代诗话的数据集自身无
LICENSE 且声明「仅限于交流学习」，因此它只能用作定位原始公有领域出处的**指针**，随后直接
引用那个原始出处；它的文本一个字都没有进入本仓库。

现有种子集 487 条，覆盖 398 首诗，取自 10 部前现代诗话（《苕溪渔隐丛话》《诗人玉屑》
《鹤林玉露》《野客丛书》《岁寒堂诗话》《人间词话》《后山诗话》《沧浪诗话》《六一诗话》
《藏一话腴》），逐条以维基文库校录本的**固定修订号**为据，可离线逐字复核。

## 尚未具备的部分（如实记录）

- **应用侧的下载与落地路径尚未接线。** `manifest.json` 已备好 `sha256` / `size_bytes` /
  `min_app_version` / `schema_version`，但目前没有代码消费它们；随包赏析种子的导入路径同样
  尚未存在。见计划的 todo 23 与 76。
- **`corpus-release.yml` 的真实发布路径未验证。** 工作流已过 `actionlint`，但「CI 里重建出来的
  工件与本机逐字节相同」这条**没有验证过**——它依赖构建可复现在不同宿主上也成立。第一次真
  发布时必须比对摘要。
- **全量集合（896,127 首）作为可选下载的体积未实测。** 当前 `corpus-build` 只产出唐宋
  （`SHIPPED_DEFAULT_SCOPE` 是常量、刻意不接受 `--scale`）。`measurements.json` 里 `full`
  规模只有拆分前形态的行。
- **首启耗时是参考机上的单次测量。** 571.8 s 测于 32 逻辑核 / NVMe 的构建机，其中 85% 在
  n-gram。移动端会显著更慢，在真机上量到之前**不要把「约 10 分钟」写进面向用户的文案**。

## 相关文档

- [架构](ARCHITECTURE.zh.md)——分层边界、检索路由、语料解析与原子物化
- [AI 赏析](AI.zh.md)——版权墙留下的洞由标注清楚的 AI 赏析填补
- [第三方许可](../LICENSES.md)——逐资产的许可与署名
