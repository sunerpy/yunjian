# changelog/ · 按大版本切分的变更日志

本目录是仓库变更日志的唯一存放处，**一个大版本序列一个文件**：

| 文件                  | 覆盖范围      |
| --------------------- | ------------- |
| `CHANGELOG-v0.x.md`   | `0.x` 系列    |
| `CHANGELOG-v1.x.md`   | `1.x` 系列（尚未开始） |

不放单一的根 `CHANGELOG.md`：那个文件会随版本累积成几千行，每次发布都在同一处产生
巨大 diff，也让「只看某个大版本发生了什么」变成翻页。切分之后，进入 `1.0` 时新建
`CHANGELOG-v1.x.md` 并把 `release-please-config.json` 的 `changelog-path` 指过去，
历史文件就此冻结，不再被改动。

## 这些文件由谁维护

- **`CHANGELOG-v*.md` 由 release-please 在它的发布 PR 里自动生成**，路径来自
  [`release-please-config.json`](../release-please-config.json) 的
  `"changelog-path": "changelog/CHANGELOG-v0.x.md"`。
- **GitHub Release 页上的说明另有一套**：发版工作流用 [git-cliff](https://git-cliff.org)
  按 [`cliff.toml`](../cliff.toml) 现场渲染，只覆盖本次 tag 的区间，不落盘进仓库。
  两者用途不同——本目录是仓库内的长期账本，只留用户可感知的变化；Release notes 是单次
  发布的完整记录，`ci` / `test` / `build` 这类也会列出来。

## 不要手工编辑

`changelog/` 整个目录在 [`.oxfmtignore`](../.oxfmtignore) 中被排除，人工改动会有两个
后果：

1. 生成方下次覆写时产生无意义的冲突或假 diff；
2. 更糟的是让发布 PR 上的 `make fmt-check` 变红——而发布 PR 恰恰是唯一必须绿的那一个，
   它红了就发不出版。

需要修正措辞时，改的是**提交信息**（未合并时用 `git commit --amend`，已合并的通过
后续提交补充），而不是这里的生成物。

## 首个版本为什么是 `0.1.0`

`.release-please-manifest.json` 播种为 `{".": "0.0.0"}`，但**只靠这个播种拿不到 `v0.1.0`**。
release-please 会拿清单里的版本去找对应的 tag（日志里那句 `looking for tagName: v0.0.0`），
仓库里没有 `v0.0.0` 这个 tag，于是它认定「没有任何历史发布」，退回内置的首发版本
`1.0.0`——`bump-minor-pre-major` 在这条路径上根本不参与，因为它要有一个 `0.x` 的基线才
生效。实测确认过：只播种清单时 dry-run 给出的标题是 `release 1.0.0`。

因此配置里显式写了 `"initial-version": "0.1.0"`，把首发钉死。`v0.1.0` 发出去之后清单变成
`0.1.0`、tag 也存在了，基线成立，此后由 `bump-minor-pre-major` 正常接管
（`0.x` 阶段的不兼容变更只升 minor，不会跳到 `1.0.0`）。

**不要因为「清单已经是 0.0.0 了」就把 `initial-version` 删掉**——删掉的后果是首发直接变成
`1.0.0`，而且这个错误只在第一次发版时暴露一次。

## 什么样的提交会进入变更日志

`release-please-config.json` 的 `changelog-sections` 显式列出了全部提交类型及其可见性，
与提交规范一一对应：

- **可见**（会出现在变更日志里，并且能触发发版）：`feat` `fix` `perf` `deps` `revert`
- **隐藏**（不出现，单独出现时也不会触发发版）：`docs` `style` `refactor` `test`
  `build` `ci` `chore`

这也是发版触发不过宽的机制：release-please 渲染出的变更日志为空时会跳过发布 PR，
所以一批只含 `docs(...)` / `ci(...)` 的提交合进 `main` 之后不会产生任何发版动作。
