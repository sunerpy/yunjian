# 应用图标集

三步生成，一步验收，**顺序不可换**：

```bash
python3 crates/yunjian-app/icons/generate_source.py
cd crates/yunjian-app && cargo tauri icon icons/source-1024.png
python3 crates/yunjian-app/icons/generate_source.py --ico-only
cargo run -p xtask -- verify-icons
```

**第三步不是多余的。** `cargo tauri icon` 生成 `icon.ico` 的方式是把 1024 源图**降采样**，
于是脚本在 16 px 原生画出来的清晰版本根本进不了 ICO——用户在任务栏看到的仍是一张
1024 缩下来的糊图。实测对照：降采样的 16 px 层有 **76** 种颜色（全是边缘抗锯齿混出的
中间色），原生渲染的同一层只有 **2** 种。`--ico-only` 就是用原生层覆盖它那一份，
而 `verify-icons` 有一条颜色数上限断言专门抓「漏跑了第三步」。

设计取值（形、色、比例）的由来在 `generate_source.py` 的文件头——它们不是审美偏好，
而是由 **16 px 可辨性**反推出来的结论，包含被淘汰候选的逐条记录。

## 手写与生成产物的分工

| 文件                 | 来源                            | 说明                                                                             |
| -------------------- | ------------------------------- | -------------------------------------------------------------------------------- |
| `generate_source.py` | 手写                            | 几何、颜色、逐尺寸取整规则与它们的由来。**与产物一起入库**。                     |
| `source-1024.png`    | 脚本                            | 1024×1024 RGBA，喂给 `cargo tauri icon` 的源图。                                 |
| `tray.png`           | 脚本                            | 32×32 RGBA，托盘图标，按 32 px **原生**渲染。                                    |
| `icon.ico`           | 脚本（覆盖 `cargo tauri icon`） | 六层 `[32, 16, 24, 48, 64, 256]`，每层该尺寸原生渲染。                           |
| 其余全部             | `cargo tauri icon`              | `icon.png`（512×512）、`icon.icns`、Appx `Square*Logo.png`、`android/`、`ios/`。 |

**源图刻意不叫 `icon.png`。** `cargo tauri icon` 会把 `icons/icon.png` **覆写成 512×512**
（本机实测）。源图与它同名会导致下一轮拿上一轮的 512 当源，每跑一次掉一档分辨率，
而 `verify-icons` 那条「源图必须 1024×1024」的断言会莫名变红。

## 为什么必须同时有 `icon.png` 与 `icon.ico`（两条不同的代码路径各自强制）

- **`icon.png`** — `tauri-codegen` 的 `context.rs` 在 Unix 目标上无条件要求
  `icons/icon.png`（`find_icon(..., |i| i.ends_with(".png"), "icons/icon.png")`），
  缺失即编译失败。
- **`icon.ico`** — `tauri-build` 的 `lib.rs:618` 在 **Windows 目标**上从 `bundle.icon`
  里找一个以 `.ico` 结尾的项，找不到就回落 `icons/icon.ico`；文件不存在时直接报
  `` `icons/icon.ico` not found; required for generating a Windows Resource file during tauri-build ``
  并让构建失败（`lib.rs:672`）。**这条路径没有 PNG 回落**，与上面那条不同。

后者是一条**只在 Windows 上出现**的编译失败：Linux 上只有 `icon.png` 一切正常，
Windows CI 第一次跑才红。`tests/window_config.rs` 因此有一条断言两个文件都存在。

## 验收结论（实测，非采信生成器的成功消息）

`cargo tauri icon` 退出 0 并打印一长串 `Creating ...`，那不是验收。逐字节实测得到：

- **源图** 1024×1024 RGBA，四角 alpha 全为 0。
- **`icon.ico` 六层齐备，层序 `[32, 16, 24, 48, 64, 256]`，32 px 在最前**。层序**只能
  靠解析目录字节发现**：图像库的 `sizes()` 之类接口返回集合，顺序在那里就丢了。
- **每一层颜色数均为 2**（朱砂 + 全透明），即抗锯齿中间色为 0。上一版 16 px 上那圈
  粉雾不是调参数调掉的，是让所有边缘落在整数像素上、不再需要抗锯齿。
- **托盘图标**四角 alpha 为 0，无自绘圆角底板。
- **六档联系表**写在 `docs/reports/icon-contact-sheet.png`（浅色 `#F3F3F3` 与深色
  `#202020` 两块底板 × 16/24/32/48/64/256 六列，与必需的 ICO 层一一对应）。像素取自
  **ICO 里真实那几层**，即 Windows 实际会取的那份字节，按**整数倍**最近邻放大；
  256 px 那一列倍率为 1，即原尺寸单独呈现。

### 亲眼过目的逐档结论

| 档位   | 结论                                                                             |
| ------ | -------------------------------------------------------------------------------- |
| 16 px  | 印框四边完整闭合、粗细一致；三道竖行清晰可数，长短递减可辨；**无任何中间色渗色** |
| 24 px  | 同上，行距更松，长短节奏更明确                                                   |
| 32 px  | 框线与行宽比例最舒展，这是托盘与任务栏的主用档                                   |
| 48 px  | 框内留白开始显出「笺」的纸面感                                                   |
| 64 px  | 全部细节到位，与 256 px 观感一致                                                 |
| 256 px | 印章边框与竖排诗行的关系完全成立；左下留白读作落款位而非缺失                     |

两块底板都成立，无需为深浅主题各发一套。

### 失败场景（真跑过，不是推理）

1. **把源图压平到白底**（`RGB` 无 alpha）：`verify-icons` 退出 **1**，报
   `源图必须是透明背景 ... 第 [0, 1, 2, 3] 个角不透明`。
2. **用 Pillow 从 1024 降采样写 ICO**（模拟漏跑 `--ico-only`）：退出非 0，一次报出
   3 项——16/24/32 三层分别有 76/97/51 种颜色，均超上限 4。

两次都已还原，还原后 `verify-icons` 退出 0、ICO 的 SHA-256 与还原前一致。

## macOS squircle：明示不处理

Apple 期望 macOS 图标是一块 squircle 底板；本图标无底板，在 Dock 里以字形形态呈现，
比相邻图标视觉上小一圈。**不处理**，理由与将来要修时的最小改动写在
`generate_source.py` 文件头末节。核心是：本机 Linux 无法目视验证 macOS 产物，
而发一份没人看过的图标比接受这个已知差异更糟。
