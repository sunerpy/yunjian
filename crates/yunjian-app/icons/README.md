# 图标占位

`icon.png` 是一枚**占位图**：1024×1024 RGBA、四角 alpha 为 0，仅用于让
`generate_context!` 通过——它在 Unix 目标上无条件要求一个 `icons/icon.png`
（`tauri-codegen` 的 `find_icon` 回落到该路径），缺失即编译失败。

真正的图标集属 **todo 65**：由 1024×1024 RGBA 源经 `cargo tauri icon` 生成，
并由 `xtask verify-icons` 逐项验收（解析 ICO 字节确认六个尺寸且 32 px 在最前、
渲染 16 px 联系表实际看过、托盘图标四角 alpha 为 0）。

因此这里刻意**只有一个文件**：放一套看起来完整但未经验收的图标，会让 todo 65
的验收变成对既有产物的追认，而那正是它明确禁止的（「不要把生成器的成功消息当作验收」）。
