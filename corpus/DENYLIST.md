# 拒绝的上游数据源

`cargo run -p xtask -- verify-sources` 会解析本文件。**只有** `## 拒绝清单` 一节里
形如 ``- `标识符` —— 理由`` 的列表项会被当作条目；标识符对 `corpus/sources.toml`
里每个 source 的 `name` 与 `url` 做大小写无关的子串匹配，命中即构建失败。

匹配只针对 **source 级** 的 `name` / `url`，绝不针对 asset 的 `path`。这一点是刻意的：
被拒绝的独立仓库 `huajianji` 与 MIT 仓库内部的子目录 `五代诗词/huajianji/` 同名，
若拿标识符去匹配路径，后者会被误杀——它已在 `sources.toml` 里按
`license_class = "unverified"` 单独扣留，走的是逐资产判定，不是拒绝清单。

同时 `verify-sources` 会断言本文件包含一组**必须存在**的标识符（见
`xtask/src/verify_sources.rs` 的 `REQUIRED_DENYLIST`）。删掉其中任何一条以便偷偷
放行某个源，构建会直接失败。

## 拒绝清单

- `huajianji` —— 仓库没有 LICENSE 文件；其 `notes` 字段虽然存在但本来就是空的。
- `VMIJUNV` —— 没有 LICENSE，README 写明「仅限于交流学习」；且仓库缺失 唐/宋/明/清，完整数据只放在百度网盘。
- `xcc3641/chinese-gushiwen` —— 没有 LICENSE。附带的 `audioUrl` 也已失效（上游：「因音频文件被用户大量下载，音频地址添加了权限，无法直接访问」）。
- `Provinm/chinese-poetry-simplified` —— 没有 LICENSE。本项目自行用 OpenCC 做繁简转换，不需要它。
- `THUNLP-AIPoet` —— 全部数据集均无 LICENSE，且声明 "released for academic use only"，学术用途授权无法覆盖本项目的分发与商用。
- `THU-CRRD` —— 同属学术用途限定。它的 `pingshui_amb.pkl` 是**唯一**的多音字韵部歧义消解数据，仍然不能用。
- `byj233/ChinesePoetryLibrary` —— 声明 MPL-2.0，但 README 又写「商用需授权」，自相矛盾；矛盾未澄清即不可用。
- `StewartXiang/poetry_with_labels` —— GPL-3.0，会传染整个应用；内容本身也是 2017 年从古诗文网爬取的。
- `sheepzh/poetry` —— LICENSE 写 MIT，README 写「不得用于任何商业盈利行为」，自相矛盾；且内容是现代诗，与本项目无关。
- `Poetry_CN` —— OpenDataLab 数据集，平台条款写明「仅用于学术目的，请勿商用」；来源为国学迷网/读古诗词网/古诗句网/古诗文网。
- `OpenDataLab` —— 同上，按整个来源方拒绝，避免换个数据集名再进来。
- `yht050511/gushiwen` —— 内容「爬取自古诗文网(2022年12月)」，原站声明「与此有关的版权由出版者所有……非经许可，不准复制」。
- `Tianyijian/GushiWenSpider` —— 爬虫本体，硬编码 `so.gushiwen.org/shiwen2017/ajaxfanyi.aspx`，产出物与上一条同源。
- `MCGA` —— 朗诵音频语料，CC BY-NC-SA-4.0（NC 条款排除本项目），且仅发布 test split。朗读一律走 TTS 合成。
- `jkak/pingShuiYun` —— **在锁定 revision 处没有任何 LICENSE 文件**。计划原文把它列为 MIT，实测为误：`GET /repos/jkak/pingShuiYun/license` 返回 404，`repos/.../commits?path=LICENSE` 与 `path=LICENSE.md` 均为 0 次提交（全仓库共 11 次提交），README 也没有任何许可/版权声明。「上游在锁定 revision 处必须存在 LICENSE」是硬规则，故拒绝。它的 `data/baseCharDict.json`（形如 `"临": [["去","二十七沁",""], ["平","十二侵",""]]`）本是最好用的逐字平水韵结构，替代方案见下。
- `caoxingyu/chinese-gushiwen` —— 上游已消失（HTTP 404），无法核实许可。
- `javayhu/poetry` —— 上游已消失（HTTP 404），无法核实许可。

## 被拒绝后的替代方案

`jkak/pingShuiYun` 原本承担「逐字 -> (声调, 韵部)」的反向索引。同一能力可以从已核实的
`charlesix59/chinese_word_rhyme`（MIT，LICENSE 实存）派生：`data/Pingshui_Rhyme.json`
是 声部 -> 韵部 -> [字] 的正向表，构建期反转即得逐字索引，并且 `data/Word_Tune.json`
已直接给出 8232 字的平仄。因此拒绝 `jkak` 不损失任何能力，只是把一次转换从「引用上游」
变成「构建期计算」，反而少一个数据源。详见 todo 15。

## 为什么现代注释/译文/赏析一概不收

上游 `chinese-poetry` 维护者 jackeyGao 在 issue #227 给出的立场，与本项目一致：
「尊重而且不进行收录各种基于诗词演绎之后的数据成果（赏析、朗读/评论、翻译），
但作品（诗词）本身已属于公有领域」。issue #76 进一步界定 MIT 的范围：
「此数据库(JSON结构化数据)是计算机数据属于软件，仅对衍生整理的工作产物保留许可权」。

需要强调的是，**仅靠拒绝清单挡不住这类内容**：本清单落地时，逐资产核查在 MIT 仓库
`chinese-poetry` **内部**发现了 10 个夹带现代注释/赏析/介绍的文件（`五代诗词/huajianji/`、
`五代诗词/nantang/`、`水墨唐诗/`、以及 `蒙学/` 下 5 个带 `abstract` 的文件），
它们全部被标为 `license_class = "unverified"`、`shippable = false`。这就是校验粒度
必须是逐资产而不是逐仓库的实证理由。

合法的分析内容有两个来源：前现代**历代诗话辑评**（公有领域，逐条注明出处，例如
`幽梦影/youmengying.json` 里清代友人的评语），以及标注清楚的 **AI 赏析**。
