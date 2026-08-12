# 图标占位

这两个文件是**占位图**，不是验收过的图标集。真正的图标集属 **todo 65**。

## 为什么必须有这两个文件（各由不同代码路径强制）

- **`icon.png`** — `tauri-codegen` 的 `context.rs` 在 Unix 目标上无条件要求
  `icons/icon.png`（`find_icon(..., |i| i.ends_with(".png"), "icons/icon.png")`），
  缺失即编译失败。
- **`icon.ico`** — `tauri-build` 的 `lib.rs:618` 在 **Windows 目标**上从 `bundle.icon`
  里找一个以 `.ico` 结尾的项，找不到就回落 `icons/icon.ico`；文件不存在时直接报
  `` `icons/icon.ico` not found; required for generating a Windows Resource file during tauri-build ``
  并让构建失败（`lib.rs:672`）。**这条路径没有 PNG 回落**，与上面那条不同。

后者是一条**只在 Windows 上出现**的编译失败：Linux 上只有 `icon.png` 一切正常，
Windows CI 第一次跑就红。`tests/window_config.rs` 因此有一条断言两个文件都存在，
把它变成本机就能发现的失败。

## 当前占位图的已知不足（刻意不修，留给 todo 65）

- `icon.png` 是几何构成的圆环加一道横笔，1024×1024 RGBA、四角 alpha 为 0。
  **没有在 16 px 下看过**，而 todo 65 明确要求渲染 16 px 联系表实际过目。
- `icon.ico` 由 Pillow 写出，逐字节解析确认含 16/24/32/48/64/256 六层、均 32 bpp，
  但**层序是升序（16 在最前）**，而 todo 65 要求 **32 px 层在最前**。
  已知 Pillow 的 ICO 写入器会重排层序，所以这份产物在层序这一项上是不合格的。
- 托盘图标尚未生成。

刻意只放满足编译所需的最小两件、并把不足写在这里：放一套看起来完整但未经验收的图标，
会让 todo 65 的验收退化成对既有产物的追认，而那正是它明确禁止的
（「不要把生成器的成功消息当作验收」）。
