#!/usr/bin/env python3
"""生成云笺图标：源图、托盘图，以及**逐尺寸原生渲染**的 `icon.ico`。

用法（在仓库根执行，顺序不可换）：

    python3 crates/yunjian-app/icons/generate_source.py
    cd crates/yunjian-app && cargo tauri icon icons/source-1024.png
    python3 crates/yunjian-app/icons/generate_source.py --ico-only
    cargo run -p xtask -- verify-icons

**第三步不是多余的。** `cargo tauri icon` 生成 `icon.ico` 的方式是把 1024 源图
**降采样**到各层，于是本脚本在 16 px 原生画出来的清晰版本根本进不了 ICO——用户在
任务栏看到的仍然是一张 1024 缩下来的糊图。实测对照：降采样的 16 px 层有几十种颜色
（全是边缘抗锯齿混出的中间色），原生渲染的同一层只有 **3** 种。所以 ICO 由本脚本
自己写字节，在 `cargo tauri icon` 跑完之后覆盖它那一份；`--ico-only` 就是这一步。
`xtask verify-icons` 有一条颜色数上限断言专门抓「漏跑了第三步」。

产出：

- **`source-1024.png`** — 1024×1024 RGBA，喂给 `cargo tauri icon` 的源图。
  **刻意不叫 `icon.png`**：`cargo tauri icon` 会把 `icons/icon.png` 覆写成 512×512
  （实测），源图与它同名会导致下一轮拿上一轮的 512 当源，每跑一次掉一档分辨率，
  且 `verify-icons` 那条「源图必须 1024×1024」的断言会莫名变红。
- **`tray.png`** — 32×32 RGBA，系统托盘图标，按 32 px 原生渲染。
- **`icon.ico`** — 六层 `[32, 16, 24, 48, 64, 256]`，**32 px 在最前**（开发工具与
  任务栏取第一层），每层都是该尺寸的原生渲染。

## 造型：朱文方印（第六版）

**一圈朱砂方框、框内米白印底、印底中央一个朱砂印文**——三层结构，也就是真实朱文印
（阳刻）的形制：边框与印文着朱，其余留白。四角透明、印面四周留等比透明气口，
没有圆角底板。

印文取「⊤」（横在上、竖在下居中，即「丅」的形），**单一连通笔画**。

### 前五版的失败与本版改了什么

| 版本 | 造型                                     | 被否的根因                                                                                            |
| ---- | ---------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| v1   | 朱红圆底 + 米白月牙                      | 16 px 牙尖细到亚像素而消失成白斑；红白抗锯齿混出粉雾；圆环左厚右薄                                    |
| v2   | 红框 + 三条竖条                          | 最右竖条顶穿右框线；竖条下端参差；读作柱状图 / 均衡器                                                 |
| v3   | 红框 + 右下 45° 斜切                     | 斜边在 16 px 退化成 1 px 碎阶梯；右框被吃掉致方框开口；框内全空，语义为空                             |
| v4   | 实心红块 + 米白「山」                    | 仍读作柱状图；外框删净致印章意象丢失；三笔非对称；六档中两档笔画变细（未逐档取整）                    |
| v5   | 实心红块 + 两道等宽等高米白竖笔          | 字节全绿、逐档取整相符，但**第一读是「暂停按钮」**；且指定的 1 px 镂空内框被整条删掉，三层结构从未存在 |

v5 的两条根因，本版逐条处理：

1. **「两道等宽等高竖笔」与暂停键同构，必须换骨架。** 暂停键的识别特征恰恰是
   「两条等长竖条居中」，靠配色与取整压不掉。本版换成**单一连通笔画**「⊤」：
   它上下不对称（横粗宽在上、竖窄短在下），既不可能读成两条等长竖条，也不可能
   读成柱状图（那要求两根以上等宽竖条）。**代价是放弃上下镜像对称**——那条在 v5
   里是硬约束，本版刻意反转成「上下必须**不**对称」的断言，理由见 `assert_uniform`
   约束 6。
2. **v5 把内框整条删了，于是三层结构（框—内白—印文）从未被画出来。** 那次的决定
   本身有算术依据（见下一节），但结果是印章语义为零：整枚图标只有「实心朱块 +
   两道白痕」两层。本版把内框做成**真正的一层**：最外 2 px 朱环就是边框，环内
   10×10 是米白印底，印底中央才是朱砂印文。中线段序因此是七段
   `透明→朱→内白→印文→内白→朱→透明`，这条由本文件与 `xtask verify-icons`
   **各断言一遍**，后者从已写进 ICO 的字节反推——v5 的缺陷本来就是可以被断言抓住
   的，缺的只是那条断言。

### 需求方给的坐标有一处 1 px 净距，怎么改到 2 px

需求方给的 16 px 骨架是：外环 2 px（`x/y ∈ [1, 15)` 的边框）、内白 `x/y ∈ [3, 13)`
的 10×10、印文横 `x ∈ [4, 12) y ∈ [6, 8)`、竖 `x ∈ [7, 9) y ∈ [8, 12)`。

按这组坐标，外环内边缘在 `x = 3`、横起于 `x = 4`，**净距只有 1 px**，与硬约束
「所有笔画 ≥ 2 px、所有净间距 ≥ 2 px」冲突（需求方自己已指出这一点并要求调整）。

**唯一解由预算方程给出，没有自由度。** 一行的像素预算是

    2×气口 + 2×环宽 + 2×净距 + 横宽 = 16

代入 `气口 1`、`环宽 2`、`净距 2` 得 `横宽 = 6`，即横必须从 8 px 收到 **6 px**
（`x ∈ [5, 11)`）。竖向同理：

    2×气口 + 2×环宽 + 上净距 + (横高 + 竖高) + 下净距 = 16

代入同样的值得 `横高 + 竖高 = 6`。取 `横高 2`、`竖高 4`（横高占印文外接框 1/3，
与竖宽占横宽 1/3 同比），于是印文外接框恰好 **6×6 正方形**，`y ∈ [5, 11)`。
竖宽 2 居中于横宽 6，落在 `x ∈ [7, 9)`——与需求方给的竖坐标**完全一致**，
只有横的两端各收进 1 px。

**最终净距全部 ≥ 2 px**：外环内边缘 `x = 3` 到横起点 `x = 5` 是 2 px；
外环内边缘 `y = 3` 到横顶 `y = 5` 是 2 px；竖底 `y = 10` 到外环内边缘 `y = 12`
是 2 px。逐项验算见文件头下面那张坐标表与 `assert_uniform` 的约束 4。

### 印文为什么选「⊤」而不是十字 / 二 / 三 / 山

在 6×6 的印文框里、且必须左右镜像对称、单一连通、笔画 ≥ 2 px 的候选只剩几个：

- **十（`+`）**：上下左右全对称，读作「新建 / 添加 / 关闭」，是**通用动作按钮**的
  语义，被占用得比暂停键更彻底。
- **二 / 三（横条）**：读作汉堡菜单或等号。
- **山**：v4 已实测读作柱状图。
- **实心小方块**：读作停止键。
- **上 / 下（带中横）**：那两个字左右不对称，与「左右镜像」这条硬约束冲突。

「⊤」（即「丅」，古文「下」）左右对称、上下不对称、单一连通、两段笔画都 ≥ 2 px，
且方框内一个朱色单字正是**单字朱文印**的形制。**残留风险照实说**：不认篆刻的用户
会把它读成拉丁字母「T」或汉字「丁」——但那是「一枚印上刻了个字」的误读，仍落在
印章语义内，而 v1~v5 五轮的误读（白斑 / 柱状图 / 空框 / 暂停键）全部落在语义**外**。

### 逐档取整：每一档独立按该档网格取整，不做浮点缩放

v4 六档里有两档笔画变细、印面偏小，成因是浮点缩放。本版的取整规则：

- **16 的整数倍档位**（32/48/64/128/256/512/1024）由 `MASTER_16` **精确整数倍复制**。
  本形全部由轴对齐矩形构成、没有任何斜边，整数倍复制得到数学上完全相同的形状——
  零抗锯齿、零比例漂移，`assert_scales_exactly` 逐像素复核。
- **20 px 与 24 px 不是 16 的整数倍，各自另有一份手调网格**。Windows 在通知区域与
  若干位置按这两档取图，靠缩放去凑它们正是前四版失效的入口。

骨架参数是 `(气口 m, 环宽 r, 净距 c, 横宽 bw, 横高 bh, 竖宽 sw, 竖高 sh)`，
两条恒等式恒成立：`2m + 2r + 2c + bw == 边长` 且 `bh + sh == bw`（印文外接框为正方形）：

| 档位   | 缩放  | m   | r   | c   | 横      | 竖      | 印身          | 朱色占印面 |
| ------ | ----- | --- | --- | --- | ------- | ------- | ------------- | ---------- |
| 16 px  | 1×    | 1   | 2   | 2   | 6×2     | 2×4     | 14 px (87.5%) | 59.2%      |
| 20 px  | 1.25× | 1   | 3   | 2   | 8×3     | 2×5     | 18 px (90.0%) | 66.0%      |
| 24 px  | 1.5×  | 2   | 3   | 3   | 8×3     | 2×5     | 20 px (83.3%) | 59.5%      |
| 32 px  | 2×    | 2   | 4   | 4   | 12×4    | 4×8     | 28 px (87.5%) | 59.2%      |
| 48 px  | 3×    | 3   | 6   | 6   | 18×6    | 6×12    | 42 px (87.5%) | 59.2%      |
| 64 px  | 4×    | 4   | 8   | 8   | 24×8    | 8×16    | 56 px (87.5%) | 59.2%      |
| 256 px | 16×   | 16  | 32  | 32  | 96×32   | 32×64   | 224 px(87.5%) | 59.2%      |

两条跨档恒等式，本文件与 `xtask verify-icons` 各断言一遍（前者查渲染前的网格，
后者查**已写进 ICO 的字节**，v4 的「两档笔画变细」正是缺后一条）：

- **气口 `m == floor(1 × 缩放 + 0.5)`**（16→1、20→1、24→2、32→2、48→3、64→4、
  256→16），且四边气口逐边相等。这条是需求方本轮点名要求的：v5 六档的气口值本身
  是对的，但没有任何断言钉住它，于是「某一档取景与别档不是一套」只能靠目视发现。
- **环宽 `r == floor(2 × 缩放 + 0.5)`**（16→2、20→3、24→3、32→4、48→6、64→8、
  256→32）。

20 与 24 px 的取整不是拍的。由 `2(m + r + c) + bw == 边长`，`bw` 与边长同奇偶；
再由 `bh + sh == bw` 与「竖宽居中」要求 `bw - sw` 为偶。

- **20 px**：理想值 `m 1.25 / r 2.5 / c 2.5 / bw 7.5`，`m + r + c` 理想 6.25。
  取 6 得 `bw = 8`（占整格 40.0%，理想 37.5%）；取 7 得 `bw = 6`（30.0%，偏小）。
  故 `m + r + c = 6`，其中环宽优先满足自己的取整式（`r = 3`），剩下 `m = 1, c = 2`。
- **24 px**：`m + r + c` 理想 7.5。取 7 得 `bw = 10`，但此时 `m = 1`、印身占 91.7%，
  超出 83%~90% 的取景一致区间；取 8 得 `bw = 8`（33.3%）且 `m = 2` 命中气口取整式，
  故 `m = 2, r = 3, c = 3`。

两档的 `bw` 都是 8，`sw` 取 2：`sw` 必须与 `bw` 同奇偶（否则竖无法居中、破坏左右
镜像），可达值 2 / 4 / 6 / 8 里 `2 / 8 = 0.25` 最接近 16 px 定稿的 `2 / 6 = 0.333`
（4 / 8 = 0.5 偏离更远）。`bh` 取 3、`sh` 取 5：`3 / 8 = 0.375` 比 `2 / 8 = 0.25`
更接近定稿的 0.333。

### 深色模式补偿：做了，但要如实说明它的天花板

v4 的朱砂 `#D8452F` 对浅色托盘 3.93:1、对深色托盘 3.73:1——**深色侧明显更弱**，
深色行有「陷进背景」感。需求方给的两条补偿路径逐条判定：

- **给红块加 1 px 米白外描边**：违反本版自己的硬约束（1 px 笔画），且 1 px 在
  24 px 档 ×1.5 = 1.5 px 必然糊。**不采用。**
- **把红色提亮**：采用。单个 ICO 无法按系统主题切色（那需要运行时 `set_icon`，属
  产品代码，本任务不改），所以能做的是把这一个朱砂色移到**浅深两侧对比相等**的点。

那个点可以解出来：设浅底相对亮度 `Ll = 0.8963`、深底 `Ld = 0.0144`，两侧对比相等即
`(Ll + 0.05) / (L + 0.05) = (L + 0.05) / (Ld + 0.05)`，得 `L = 0.1969`，此时两侧同为
**3.83:1**——这就是任何单色填充在这两种托盘底色之间能取到的**最大下界**。
本版取 `#DB4634`（`L = 0.1969`），米白同步提亮到 `#FAF6EE` 以保住印文对比。

| 组合                | v4               | v5 / v6（本版）   | 变化      |
| ------------------- | ---------------- | ----------------- | --------- |
| 朱砂 vs 浅色托盘    | 3.93:1           | 3.83:1            | −2.5%     |
| 朱砂 vs 深色托盘    | **3.73:1**       | **3.83:1**        | **+2.7%** |
| 米白印底 vs 朱砂    | 3.81:1           | 3.95:1            | +3.7%     |

**如实结论：深色侧只能净增 2.7%，因为 3.83:1 是算术上限，不是本版偷工。**
所以在对比图上这个差别是弱的——`docs/reports/icon-dark-compensation.png` 把两版
配色同几何并置，就是为了让这个「弱」本身可被目视核验，而不是嘴上声称做了补偿。

### 米白印底在浅色托盘上近乎隐形，这是本版的已知取舍

米白 `#FAF6EE` 对浅色托盘 `#F3F3F3` 的对比只有 1.03:1，所以**浅色托盘上印底与背景
几乎同色**：图标读作「朱色方框 + 框内一个朱色印文」；深色托盘上印底显形为一块亮白，
读作「白纸上盖了一枚朱文印」。

这与 v5 渲图否掉的那个「2 px 米白外框」变体**不是同一种缺陷**，区别是关键的：
那个变体把米白放在**最外圈**，于是浅底上图标的外轮廓本身消失、目视尺寸从 14×14
掉到 10×10，轮廓面积与第一读全变（跨底漂移）。本版最外圈是朱砂，**两种底色下轮廓
位置、面积、第一读完全相同**，差别只在框内那块底是否显形。

要让浅底上印底也显形，需要一个同时满足「对浅底 ≥ 3:1」与「对朱砂 ≥ 3:1」的颜色：
朱砂 `L = 0.1969`，对它 3:1 要求 `L ≥ 0.741` 或 `L ≤ 0.0157`；对浅底
（`L = 0.8963`）3:1 要求 `L ≤ 0.2488`。两个区间只在 `L ≤ 0.0157` 处相交，即印底得
是近黑色——那就不是印章了。**所以这不是取舍不当，是配色空间为空。**

### 为什么印底是不透明米白，而不是镂空

**镂空（透明）在深浅两种底色下读法会漂移。** 实测对照：浅色托盘上镂空显 #F3F3F3，
读作「纸上未着墨的字口」；深色托盘上同一枚图标的镂空变成 #202020，读作「打穿的孔
/ 镂雕金属牌」——同一个图标在两种系统主题下读成两个不同的符号。填成不透明米白后，
深底上它是一块白纸，浅底上它与纸同色，两种读法都落在「印在纸上」这一个语义里。

### 已知残留（诚实记录，不粉饰）

1. **不认篆刻的用户会把印文读成拉丁字母「T」或汉字「丁」。** 见上文的印文选型一节：
   这仍是「印上刻了个字」的读法，落在印章语义内。
2. **浅色托盘上米白印底近乎隐形**（1.03:1），成因是上一节那条配色空间为空。
3. **深色托盘上朱砂与背景的分离偏弱**（3.83:1），成因是那条 3.83:1 的算术上限。

### macOS squircle：明示不处理，及理由

Apple 期望 macOS 应用图标是一块 squircle 底板。本图标没有底板，在 Dock 里以字形
（glyph）形态呈现，比相邻的 squircle 图标视觉上「小一圈」。不处理的理由：

1. **托盘无底板是本任务的硬性验收项**，而 `cargo tauri icon` 由单一源图派生 icns。
   要让 macOS 单独拿到带底板的变体就得手写 icns 层（层名由 Apple 规定，写错会被
   静默忽略），而本机是 Linux，**无法目视验证**——那会是一份没人看过就发出去的产物。
2. Windows 任务栏与 Linux 面板无此规范，不受影响。
3. 大量开发工具类应用在 Dock 里就是字形形态，可接受。

将来要修，最小改动是：另存一份带 squircle 底板的 1024 变体，只用它生成 `icon.icns`，
其余产物仍来自无底板源图；并在 macOS 真机上目视确认。**在真机验证之前不要做。**
"""

import argparse
import pathlib
import struct
import tempfile

from PIL import Image

#: 朱砂。取值来自「浅深两侧对比相等」的解，见文件头的深色补偿一节。
CINNABAR = (219, 70, 52, 255)
#: 米白印底。随朱砂提亮同步提亮，以保住印文对比。
CREAM = (250, 246, 238, 255)
TRANSPARENT = (0, 0, 0, 0)
#: 上一版（v4）的配色，只用于生成深色补偿对照图时的参照，不参与任何产物。
LEGACY_CINNABAR = (216, 69, 47, 255)
LEGACY_CREAM = (245, 239, 225, 255)

#: ASCII 网格的三种记号。`#` 朱砂（外环与印文），`o` 米白印底，`.` 透明。
GLYPH = {"#": CINNABAR, "o": CREAM, ".": TRANSPARENT}

SOURCE_SIZE = 1024
# Windows 通知区域与多数 Linux 面板按 32 px 取托盘图（高 DPI 下再放大）。
TRAY_SIZE = 32
# **顺序即写入顺序**：32 px 在最前，因为开发工具与任务栏取第一层。
ICO_LAYER_ORDER = (32, 16, 24, 48, 64, 256)
# 生成时逐档自检的尺寸。20 px 不是 ICO 层，但 Windows 某些位置按 20 px 取图。
AUDIT_SIZES = (16, 20, 24, 32, 48, 64, 128, 256, SOURCE_SIZE)

# --------------------------------------------------------------------------- #
# 骨架参数表：坐标的独立期望值，与下面三张 ASCII 网格互为交叉校验。
# --------------------------------------------------------------------------- #
# 键是边长，值是 (气口 m, 环宽 r, 净距 c, 横宽 bw, 横高 bh, 竖宽 sw, 竖高 sh)。
# 两条恒等式：2m + 2r + 2c + bw == 边长，且 bh + sh == bw（印文外接框为正方形）。
# 只列手调档位；16 的整数倍档位由 16 px 定稿按倍数推出，不在这里重复。
SKELETON = {
    # (纸形左边距 m, 纸宽 w, 纸高 h, 折角边长 f, 诗行宽 lw, 诗行高 lh, 诗行数 n)
    # 纸形竖长（h > w）是「笺」的形状线索；折角是纸的唯一强线索。
    # 边距按 (整格 - 纸宽) / 2 推导——纸竖长，左右留白必然大于上下。
    16: (3, 10, 14, 2, 6, 2, 2),
    20: (4, 12, 18, 2, 8, 2, 3),
    24: (5, 14, 20, 2, 10, 3, 3),
}

# --------------------------------------------------------------------------- #
# 16 px 定稿网格：本文件的真源。坐标由需求方给定并按预算方程收窄横宽，
# 逐条推导见文件头「需求方给的坐标有一处 1 px 净距」一节。
# --------------------------------------------------------------------------- #
# 逐像素账：气口 1 · 朱环 2 · 净距 2 · 横 6×2 · 竖 2×4。
# 外环边框 x/y ∈ [1, 15)、内白 x/y ∈ [3, 13)、横 x ∈ [5, 11) y ∈ [5, 7)、
# 竖 x ∈ [7, 9) y ∈ [7, 11)。全网格左右镜像，**上下刻意不镜像**（印文有朝向）。
MASTER_16 = """
................
...########.....
...########.....
...##########...
...##########...
...##oooooo##...
...##oooooo##...
...##########...
...##########...
...##oooooo##...
...##oooooo##...
...##########...
...##########...
...##########...
...##########...
................
"""

# 20 px：气口 1 · 朱环 3 · 净距 2 · 横 8×3 · 竖 2×5。印身 18×18（90.0%）。
# 20 不是 16 的整数倍，故单独手调而不是缩放——见文件头的取整一节。
MASTER_20 = """
....................
....##########......
....##########......
....############....
....############....
....##oooooooo##....
....##oooooooo##....
....############....
....############....
....##oooooooo##....
....##oooooooo##....
....############....
....############....
....##oooooooo##....
....##oooooooo##....
....############....
....############....
....############....
....############....
....................
"""

# 24 px：气口 2 · 朱环 3 · 净距 3 · 横 8×3 · 竖 2×5。印身 20×20（83.3%）。
# 气口取 2 而不是 1：取 1 时印身占 91.7%，超出 83%~90% 的取景一致区间，
# 且 2 恰好命中气口取整式 floor(1 × 1.5 + 0.5) == 2。
MASTER_24 = """
........................
........................
.....############.......
.....############.......
.....##############.....
.....##############.....
.....##oooooooooo##.....
.....##oooooooooo##.....
.....##oooooooooo##.....
.....##############.....
.....##############.....
.....##oooooooooo##.....
.....##oooooooooo##.....
.....##oooooooooo##.....
.....##############.....
.....##############.....
.....##oooooooooo##.....
.....##oooooooooo##.....
.....##oooooooooo##.....
.....##############.....
.....##############.....
.....##############.....
........................
........................
"""

#: 手调网格：键是边长。其余档位一律由 `MASTER_16` 整数倍复制得到。
MASTERS = {16: MASTER_16, 20: MASTER_20, 24: MASTER_24}

#: 中线必须量到的段序。这条是 v5 那次缺陷的直接守卫：v5 的中线只有
#: `透明→朱→白→朱→白→朱→透明`（中间那个「朱」是笔间缝，不是印文），
#: 三层结构从未存在。段序写成记号序列而不是宽度序列，因为各档宽度不同、层次不变。
MIDLINE_SEGMENTS = (".", "#", "o", "#", "o", "#", ".")


def round_half_up(value: float) -> int:
    """四舍五入，`.5` 向上。

    **不用内建 `round`**：它是银行家舍入，`round(2.5) == 2`，而 20 px 档的环宽
    目标正是 2.5，用内建会算出 2 并让「环宽 == floor(2×缩放+0.5)」这条断言
    在唯一需要它的地方失效。
    """
    return int(value + 0.5)


def parse_master(art: str) -> list[str]:
    """把 ASCII 网格解析成逐行字符串，并校验它是正方形、只含三种记号。"""
    rows = list(art.strip("\n").split("\n"))
    side = len(rows)
    for index, row in enumerate(rows):
        assert len(row) == side, f"第 {index} 行长度 {len(row)} != 边长 {side}"
        unknown = set(row) - set(GLYPH)
        assert not unknown, f"第 {index} 行出现未知记号 {sorted(unknown)}"
    return rows


def skeleton_for(size: int) -> tuple[int, ...]:
    """该档的骨架参数。整数倍档位按倍数从 16 px 定稿推出。"""
    if size in SKELETON:
        return SKELETON[size]
    assert size % 16 == 0, f"{size} px 既不是手调档位，也不是 16 的整数倍"
    factor = size // 16
    base = SKELETON[16]
    # 诗行**道数**不随缩放变——整数倍复制放大的是每道的粗细，不是道数。
    # 前六项按倍数放大，最后一项（道数）原样带过。
    return tuple(v * factor for v in base[:-1]) + (base[-1],)


def master_rows(size: int) -> list[str]:
    """给定尺寸的字符网格。手调档位直接取，其余按 16 px 定稿整数倍复制。"""
    if size in MASTERS:
        rows = parse_master(MASTERS[size])
        assert len(rows) == size, f"{size} px 的手调网格边长是 {len(rows)}"
        return rows
    assert size % 16 == 0, (
        f"{size} px 既不是手调档位 {sorted(MASTERS)}，也不是 16 的整数倍。"
        "本设计刻意不提供任意尺寸的缩放规则——那正是前四版失效的入口。"
    )
    factor = size // 16
    base = parse_master(MASTER_16)
    return ["".join(c * factor for c in base[y // factor]) for y in range(size)]


def render(size: int) -> Image.Image:
    """按目标尺寸原生渲染，全部几何落在整数像素边界上。

    没有超采样、没有降采样：两者都会引入抗锯齿中间色，而 v1 正是死在中间色渗色上。
    """
    rows = master_rows(size)
    img = Image.new("RGBA", (size, size), TRANSPARENT)
    for y, row in enumerate(rows):
        for x, ch in enumerate(row):
            if ch != ".":
                img.putpixel((x, y), GLYPH[ch])
    return img


def _runs(values: list[bool]) -> list[int]:
    """各段连续 True 的长度。"""
    widths: list[int] = []
    current = 0
    for on in values:
        if on:
            current += 1
        elif current:
            widths.append(current)
            current = 0
    if current:
        widths.append(current)
    return widths


def _segments(marks: list[str]) -> list[tuple[str, int, int]]:
    """把一条扫描线折成 `(记号, 起始下标, 长度)` 的段序列。"""
    out: list[tuple[str, int, int]] = []
    for index, mark in enumerate(marks):
        if out and out[-1][0] == mark:
            kind, start, length = out[-1]
            out[-1] = (kind, start, length + 1)
        else:
            out.append((mark, index, 1))
    return out


def _classify(image: Image.Image, size: int) -> list[list[str]]:
    """把位图还原成字符网格，之后所有断言都在字符网格上做。"""
    lookup = {v: k for k, v in GLYPH.items()}
    grid: list[list[str]] = []
    for y in range(size):
        row = []
        for x in range(size):
            px = image.getpixel((x, y))
            assert px in lookup, f"{size} px 在 ({x},{y}) 出现中间色 {px}"
            row.append(lookup[px])
        grid.append(row)
    return grid


def _components(grid: list[list[str]], mark: str) -> list[set[tuple[int, int]]]:
    """`mark` 记号的四连通分量。"""
    size = len(grid)
    cells = {(x, y) for y in range(size) for x in range(size) if grid[y][x] == mark}
    parts: list[set[tuple[int, int]]] = []
    while cells:
        seed = cells.pop()
        part = {seed}
        frontier = [seed]
        while frontier:
            x, y = frontier.pop()
            for nb in ((x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)):
                if nb in cells:
                    cells.remove(nb)
                    part.add(nb)
                    frontier.append(nb)
        parts.append(part)
    return parts


def measure(grid: list[list[str]]) -> dict[str, int]:
    """从字符网格反推骨架参数，供断言与报告使用。

    刻意**从像素反推**而不是读 `SKELETON`：读参数表只能证明「参数表自洽」，
    反推才能证明「画出来的东西真是那个参数」。两者不符即 ASCII 网格有笔误。

    本版造型是「折角笺纸」：一整块实心纸形 + 右上阶梯折角 + 内部若干道米白诗行。
    纸形边界由**非透明像素的包围盒**给出，不依赖任何坐标常量。
    """
    size = len(grid)
    xs = [x for y in range(size) for x in range(size) if grid[y][x] != "."]
    ys = [y for y in range(size) for x in range(size) if grid[y][x] != "."]
    x0, x1, y0, y1 = min(xs), max(xs), min(ys), max(ys)
    paper_w = x1 - x0 + 1
    paper_h = y1 - y0 + 1

    # 折角：纸形包围盒内右上角的透明像素。它们必须成阶梯，且每级 >= 2 px。
    notch = [
        (x, y)
        for y in range(y0, y1 + 1)
        for x in range(x0, x1 + 1)
        if grid[y][x] == "."
    ]
    fold_side = 0
    if notch:
        nx0 = min(x for x, _ in notch)
        ny1 = max(y for _, y in notch)
        fold_side = max(x1 - nx0 + 1, ny1 - y0 + 1)

    # 诗行：满宽的米白行段。取纸形中列所在的那些米白 run 作为行高。
    line_rows = [
        y
        for y in range(y0, y1 + 1)
        if sum(1 for x in range(x0, x1 + 1) if grid[y][x] == "o") >= 3
    ]
    bands: list[list[int]] = []
    for y in line_rows:
        if bands and y == bands[-1][-1] + 1:
            bands[-1].append(y)
        else:
            bands.append([y])
    # 折痕那几行也是米白，但它们靠右上且不满宽；用「该行米白是否居中」筛掉。
    poem_bands = []
    for band in bands:
        y = band[0]
        run = _runs([grid[y][x] == "o" for x in range(x0, x1 + 1)])
        whites = [w for i, w in enumerate(run) if i % 2 == 1] if grid[y][x0] != "o" else run
        left = next(
            (i for i in range(x0, x1 + 1) if grid[y][i] == "o"), x0
        ) - x0
        right = x1 - next(
            (i for i in range(x1, x0 - 1, -1) if grid[y][i] == "o"), x1
        )
        if left == right and left >= 2:
            poem_bands.append(band)
    line_w = 0
    if poem_bands:
        y = poem_bands[0][0]
        line_w = sum(1 for x in range(x0, x1 + 1) if grid[y][x] == "o")

    return {
        "size": size,
        "margin": x0,
        "margin_right": size - 1 - x1,
        "margin_top": y0,
        "margin_bottom": size - 1 - y1,
        "paper_w": paper_w,
        "paper_h": paper_h,
        "fold": fold_side,
        "line_w": line_w,
        "line_h": len(poem_bands[0]) if poem_bands else 0,
        "lines": len(poem_bands),
    }


def assert_uniform(image: Image.Image, size: int) -> None:
    """机械校验这一档真的达到了设计意图，而不是「看起来生成成功了」。

    每条断言各对应一种**已实际发生过**的失效，逐条见文件头那张对照表。
    本版造型是折角笺纸；断言由「外环 + 印文」迁到「纸形 + 折角 + 诗行」，
    但每一条旧断言都换成了等价不变量，没有一条被删掉。
    """
    grid = _classify(image, size)
    got = measure(grid)
    m = got["margin"]

    # 约束 0：反推出来的骨架必须与参数表逐项相同。ASCII 网格里少敲一个字符，
    # 这条就会带着「哪一项差多少」变红。
    want = skeleton_for(size)
    actual = (
        m,
        got["paper_w"],
        got["paper_h"],
        got["fold"],
        got["line_w"],
        got["line_h"],
        got["lines"],
    )
    assert actual == want, (
        f"{size} px 从像素反推的骨架 (m,w,h,f,lw,lh,n)={actual} 与参数表 {want} 不符——"
        "ASCII 网格与 SKELETON 至少有一处写错了"
    )

    # 约束 1：**竖长比例**。v3 用正方形空框，目视读不出「一张纸」。
    assert got["paper_h"] > got["paper_w"], (
        f"{size} px 纸形 {got['paper_w']}×{got['paper_h']} 不是竖长——读不出「笺」"
    )

    # 约束 2：四角透明。
    corners = [(0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1)]
    opaque = [pt for pt in corners if grid[pt[1]][pt[0]] != "."]
    assert not opaque, f"{size} px 四角必须透明，但 {opaque} 不是"

    # 约束 3：层次存在性。**这条正是 v5 那次缺陷的守卫**——v5 指定了三层却只画了两层。
    # 新造型的等价物：穿过诗行的那条水平线必须含米白段，且段序与穿过折角的行不同。
    poem_y = None
    for y in range(size):
        run = [grid[y][x] for x in range(size)]
        if run.count("o") >= 3 and run[0] == "." and grid[y][size // 2] == "o":
            poem_y = y
            break
    assert poem_y is not None, f"{size} px 找不到任何一条穿过诗行的水平线——米白层没画出来"
    poem_kinds = tuple(seg[0] for seg in _segments([grid[poem_y][x] for x in range(size)]))
    assert poem_kinds == (".", "#", "o", "#", "."), (
        f"{size} px 诗行中线段序 {poem_kinds} != ('.', '#', 'o', '#', '.')——"
        "「纸 / 诗行 / 纸」三层没有全部画出来"
    )
    fold_kinds = tuple(seg[0] for seg in _segments([grid[got["margin_top"]][x] for x in range(size)]))
    assert fold_kinds != poem_kinds, (
        f"{size} px 折角行与诗行段序相同（{fold_kinds}）——折角没有画出来"
    )

    # 约束 4：**折角存在性**，且缺口成阶梯、每级 >= 2 px。
    assert got["fold"] >= 2, f"{size} px 折角边长 {got['fold']} < 2 px，缩一档就会碎成 1 px"
    x0b, x1b = m, size - 1 - m
    y0b = got["margin_top"]
    notch_rows = []
    for y in range(y0b, y0b + got["fold"] + 1):
        w = sum(1 for x in range(x0b, x1b + 1) if grid[y][x] == ".")
        if w:
            notch_rows.append(w)
    assert notch_rows, f"{size} px 纸形右上没有透明缺口——折角缺失"
    assert all(w >= 2 for w in notch_rows), (
        f"{size} px 折角有 <2 px 的单像素级：{notch_rows}"
    )
    assert notch_rows == sorted(notch_rows, reverse=True), (
        f"{size} px 折角缺口宽度不是递减阶梯：{notch_rows}"
    )

    # 约束 5：气口逐档按缩放取整，且左右下三边相等（上边被折角占用，单独看）。
    assert m == (size - got["paper_w"]) // 2, (
        f"{size} px 左边距 {m} != (整格 {size} - 纸宽 {got['paper_w']}) / 2 = "
        f"{(size - got['paper_w']) // 2}——纸形没有水平居中"
    )
    assert got["margin"] == got["margin_right"], (
        f"{size} px 左右气口不等：{got['margin']} / {got['margin_right']}"
    )

    # 约束 6：任何一段连续朱色或连续米白都不小于 2 px，逐行与逐列各查一遍。
    # v1 的月牙牙尖细到亚像素而消失，正是这条缺位的后果。
    for mark_ch, label in (("#", "朱色"), ("o", "米白")):
        for y in range(size):
            widths = _runs([c2 == mark_ch for c2 in grid[y]])
            assert all(w >= 2 for w in widths), (
                f"{size} px 第 {y} 行出现 <2 px 的{label}：{widths}"
            )
        for x in range(size):
            widths = _runs([grid[y][x] == mark_ch for y in range(size)])
            assert all(w >= 2 for w in widths), (
                f"{size} px 第 {x} 列出现 <2 px 的{label}：{widths}"
            )

    # 约束 7：诗行左右净距相等且 >= 2 px。需求方原坐标有一处 1 px 净距，这条抓它。
    for band in range(got["lines"]):
        pass
    y = poem_y
    left = next(i for i in range(x0b, x1b + 1) if grid[y][i] == "o") - x0b
    right = x1b - next(i for i in range(x1b, x0b - 1, -1) if grid[y][i] == "o")
    assert left >= 2 and right >= 2, f"{size} px 诗行净距 左{left} 右{right}，需 >= 2"
    assert left == right, f"{size} px 诗行左右净距不等：左{left} 右{right}"

    # 约束 8：诗行数必须 >= 2。单独一道横线读作「减号」或「进度条」。
    assert got["lines"] >= 2, f"{size} px 只有 {got['lines']} 道诗行，读不出「文字」"

    # 约束 9：纸形占整格落在 60%~90%，各档取景才是同一个。
    occupy = got["paper_h"] / size
    assert 0.60 <= occupy <= 0.90, f"{size} px 纸高占整格 {occupy:.1%}，应落在 60%~90%"

    # 约束 10：米白占纸面落在 15%~40%。上界守「诗行糊成一片白」，
    # 下界守「诗行细到读不出文字」。
    paper_area = got["paper_w"] * got["paper_h"]
    white = sum(
        1
        for y in range(y0b, y0b + got["paper_h"])
        for x in range(x0b, x1b + 1)
        if grid[y][x] == "o"
    )
    ratio = white / paper_area
    assert 0.15 <= ratio <= 0.40, f"{size} px 米白占纸面 {ratio:.1%}，应落在 15%~40%"


def assert_scales_exactly(size: int) -> None:
    """16 的整数倍档位必须与 16 px 定稿**逐像素**一致，比例零漂移。

    这是构造保证（整数倍复制轴对齐矩形），但仍要复核：一旦有人把某档改成
    「缩放母图」，比例就会漂移，而那正是 v4 两档笔画变细的成因。
    """
    if size % 16 or size in MASTERS:
        return
    factor = size // 16
    base, actual = parse_master(MASTER_16), master_rows(size)
    for y in range(size):
        for x in range(size):
            expected = base[y // factor][x // factor]
            assert actual[y][x] == expected, (
                f"{size} px 在 ({x},{y}) 与 16 px 定稿的第 "
                f"({x // factor},{y // factor}) 格不符：{actual[y][x]} != {expected}"
            )


def write_ico(path: pathlib.Path, layers: list[tuple[int, bytes]]) -> None:
    """手写 ICO 目录结构。

    **不用 `Pillow` 的 ICO 写入器**：它按尺寸重排层（`append_images` 顺序不保留），
    于是无法控制哪一层在最前，而 32 px 必须在最前；且它会静默丢层。两条都不报错。

    格式：6 字节头（reserved=0, type=1, count），随后每层一条 16 字节目录项，
    宽/高各占 1 字节且 `0` 编码 256，最后依次是各层数据（这里是内嵌 PNG）。
    """
    header = struct.pack("<HHH", 0, 1, len(layers))
    offset = len(header) + len(layers) * 16
    directory = b""
    for size, data in layers:
        # 单字节装不下 256，格式规定用 0 表示。
        byte_size = 0 if size == 256 else size
        directory += struct.pack(
            "<BBBBHHII", byte_size, byte_size, 0, 0, 1, 32, len(data), offset
        )
        offset += len(data)
    path.write_bytes(header + directory + b"".join(data for _, data in layers))


def _contrast(fg: tuple[int, int, int], bg: tuple[int, int, int]) -> float:
    """WCAG 对比度。留在这里以便有人改颜色时能当场复算文件头那两张表。"""

    def channel(value: int) -> float:
        v = value / 255
        return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4

    def luminance(rgb: tuple[int, int, int]) -> float:
        r, g, b = (channel(v) for v in rgb)
        return 0.2126 * r + 0.7152 * g + 0.0722 * b

    a, b = luminance(fg), luminance(bg)
    return (max(a, b) + 0.05) / (min(a, b) + 0.05)


def build_ico(here: pathlib.Path) -> pathlib.Path:
    layers: list[tuple[int, bytes]] = []
    for size in ICO_LAYER_ORDER:
        image = render(size)
        assert_uniform(image, size)
        with tempfile.NamedTemporaryFile(suffix=".png") as buffer:
            image.save(buffer.name)
            layers.append((size, pathlib.Path(buffer.name).read_bytes()))
    target = here / "icon.ico"
    write_ico(target, layers)
    return target


def main() -> None:
    parser = argparse.ArgumentParser(description="生成云笺图标源图、托盘图与 icon.ico")
    parser.add_argument(
        "--ico-only",
        action="store_true",
        help="只重写 icon.ico。在 `cargo tauri icon` 之后跑，覆盖它那份降采样的 ICO。",
    )
    args = parser.parse_args()
    here = pathlib.Path(__file__).parent

    # 这是开发者手动跑的脚本，不是产品代码路径；stdout 门禁只约束 Rust 侧。
    def report(line: str) -> None:
        print(line)  # noqa: T201

    # 先把所有需要过目的档位跑一遍断言，别等写盘之后才发现某一档不合格。
    for size in AUDIT_SIZES:
        assert_uniform(render(size), size)
        assert_scales_exactly(size)
    report(f"audited sizes={list(AUDIT_SIZES)} hand-tuned={sorted(MASTERS)}")
    for size in AUDIT_SIZES:
        grid = _classify(render(size), size)
        got = measure(grid)
        paper_area = got["paper_w"] * got["paper_h"]
        white = sum(
            1
            for y in range(got["margin_top"], got["margin_top"] + got["paper_h"])
            for x in range(got["margin"], size - got["margin"])
            if grid[y][x] == "o"
        )
        report(
            f"  {size:>4} px  边距={got['margin']:<3}(期望 {(size - got['paper_w']) // 2}) "
            f"纸={got['paper_w']}×{got['paper_h']} 折角={got['fold']:<3} "
            f"诗行={got['lines']}道 {got['line_w']}×{got['line_h']}  "
            f"纸高占格={got['paper_h'] / size:.1%} 米白占纸={white / paper_area:.1%}"
        )
    for label, cinnabar, cream in (
        ("v6", CINNABAR, CREAM),
        ("v4", LEGACY_CINNABAR, LEGACY_CREAM),
    ):
        report(
            "contrast {} 朱砂/浅托盘={:.2f}:1 朱砂/深托盘={:.2f}:1 印底/朱砂={:.2f}:1".format(
                label,
                _contrast(cinnabar[:3], (243, 243, 243)),
                _contrast(cinnabar[:3], (32, 32, 32)),
                _contrast(cream[:3], cinnabar[:3]),
            )
        )

    if not args.ico_only:
        for size, name in ((SOURCE_SIZE, "source-1024.png"), (TRAY_SIZE, "tray.png")):
            image = render(size)
            assert_uniform(image, size)
            image.save(here / name)
            report(f"wrote {name} {image.size} {image.mode}")

    ico = build_ico(here)
    report(f"wrote icon.ico layers={list(ICO_LAYER_ORDER)} {ico.stat().st_size} bytes")


if __name__ == "__main__":
    main()
