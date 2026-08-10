简体中文 · [English](readme/ARCHITECTURE.md)

# 架构

> **占位文档。** 本文的完整内容由 todo 72 填充，届时会覆盖分层边界、`yunjian-core`
> 为什么不知道 Tauri 存在、以及移动端的两条备选通路。当前只记录已经定案、不再重新讨论的约束。

## 已定案的边界

- **`yunjian-core` 里没有 `tauri::`，也没有任何外壳假设。** 这是移动端逃生通道能一直保持
  自由的唯一原因：换外壳不需要改诗词逻辑。
- **检索是一个 SQLite 文件，不是搜索引擎。** `rusqlite` bundled SQLite 上的 FTS5 trigram
  索引，配一张 1/2 字 n-gram 候选表。不引入 tantivy（Android 上 `MmapDirectory` 不受支持）、
  不引入 jieba / lindera（文言的正确切分粒度是字，不是词）。
- **日志一律走 stderr 与滚动文件，永不走 stdout。** 同一个二进制要托管 MCP stdio 服务器。
- **`stable_id` 是唯一的用户可见键**，由与内容无关的 source locator 铸造，注册表是
  append-only 事件日志。详见 [语料与索引](CORPUS.zh.md)。

## 待补

分层图、crate 依赖方向、IPC 与流式取消、移动端 gate 的两条分支。
