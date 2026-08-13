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

## 造型：朱底双笔印（第五版，按给定像素坐标施工）

**一块实心朱砂方印，印面上以不透明米白刻出两道等宽等高的竖笔**（白文印 / 阴刻），
模拟竖排诗行。四角透明、印面四周留 1 px 透明气口，没有圆角底板。

### 本版与前四版的流程差别：坐标是输入，不是产出

前四版都是「自由发挥造型」，四轮全部被目视否掉，每次失手都在同一个环节
（见下表）。第五版把设计自由度收回：**16×16 的逐像素坐标由需求方直接给定**，
本脚本只负责按坐标施工、逐档取整、并守住可机械验证的不变量。

| 版本 | 造型                     | 被否的根因                                                                       |
| ---- | ------------------------ | -------------------------------------------------------------------------------- |
| v1   | 朱红圆底 + 米白月牙      | 16 px 牙尖细到亚像素而消失成白斑；红白抗锯齿混出粉雾；圆环左厚右薄               |
| v2   | 红框 + 三条竖条          | 最右竖条顶穿右框线；竖条下端参差；读作柱状图 / 均衡器                            |
| v3   | 红框 + 右下 45° 斜切     | 斜边在 16 px 退化成 1 px 碎阶梯；右框被吃掉致方框开口；框内全空，语义为空        |
| v4   | 实心红块 + 米白「山」    | 碎线 / 顶穿 / 开口都修好了，但仍读作柱状图；外框删净致印章意象丢失；三笔非对称；六档中两档笔画变细（未逐档取整） |

### 给定坐标与那处 1 px 矛盾的解法

需求方给出的 16 px 骨架是（左上角为原点，`x` 向右、`y` 向下，半开区间）：

| 构件         | 坐标                                       |
| ------------ | ------------------------------------------ |
| 透明气口     | 最外一圈 1 px                              |
| 朱砂印身     | `x ∈ [1, 15)`、`y ∈ [1, 15)`，即 14×14     |
| 内框镂空     | `x ∈ [3, 13)` / `y ∈ [3, 13)` 边界一圈 1 px |
| 左笔（米白） | `x ∈ [5, 7)`、`y ∈ [5, 11)`                |
| 右笔（米白） | `x ∈ [9, 11)`、`y ∈ [5, 11)`               |
| 笔间朱缝     | `x ∈ [7, 9)`                               |

其中「1 px 镂空环」与硬约束「所有笔画 ≥ 2 px、所有净间距 ≥ 2 px」冲突。
需求方给了三条出路（a 收窄两笔 / b 去掉镂空环 / c 微调坐标凑到 2 px）并倾向 c。

**先证 a 与「保留镂空环的 c」都不可能，这不是取舍而是算术：** 16 px 一行的像素预算
必须容纳
`1(气口) + 2(外朱边) + 1(镂空) + 2(净距) + 2(笔) + 2(缝) + 2(笔) + 2(净距) + 1(镂空) + 2(外朱边) + 1(气口) = 18`，
**超出 16 两格**。收窄两笔（a）省不下这两格（笔已经在 2 px 下限上，再窄就违反同一条
硬约束），且它破坏左右对称。所以只要那一圈镂空还在，净间距就不可能达到 2 px。

**再证「保留边框感的 c」不可行——这一条是渲图目视否掉的，不是推理否掉的。**
把镂空环换成 16 px 下唯一能塞进去的合法边框形态（最外圈 2 px 米白外框，随后朱面
10×10、净距恰好 2 px，行预算
`1 + 2 + 2 + 2 + 2 + 2 + 2 + 2 + 1 = 16` 零自由度），与去掉环的版本一起渲成
16 / 24 / 32 × 浅底 / 深底的放大对照图后逐格目视，米白外框版被否，两条理由：

1. **朱面只剩 10×10（62.5%）**，而米白外框在浅色托盘上与背景对比仅 1.03:1、
   **完全隐形**——那两圈像素在浅底上是纯浪费，图标目视显著变小。
2. **跨底漂移**：同一份字节在浅底读作「一小块红方块」（外框隐形），在深底读作
   「白框套红块」（外框显形为高对比亮边），轮廓面积、层级数、第一读全变。
   而 16 px 托盘图标最依赖的恰恰是形状的肌肉记忆。

**因此采用 b：删掉那一圈 1 px 镂空，其余坐标一个数字都不改。** 代价是没有独立的
边框线，只有 4 px 均匀朱边——但 b **同时满足 c 列出的每一条要求**（两笔等宽等高、
左右镜像对称、缝宽均匀、四边留白对称、笔画 ≥ 2 px），且净间距是 4 px 而不是勉强
凑到的 2 px。「实心朱底 + 白色刻痕」本身就是白文印的形制，边框线不是印章的必要条件。

### 逐档取整：每一档独立按该档网格取整，不做浮点缩放

v4 六档里有两档笔画变细、印面偏小，成因是浮点缩放。本版的取整规则：

- **16 的整数倍档位**（32/48/64/128/256/512/1024）由 `MASTER_16` **精确整数倍复制**。
  本形全部由轴对齐矩形构成、没有任何斜边，整数倍复制得到数学上完全相同的形状——
  零抗锯齿、零比例漂移，`assert_scales_exactly` 逐像素复核。
- **20 px 与 24 px 不是 16 的整数倍，各自另有一份手调网格**。Windows 在通知区域与
  若干位置按这两档取图，靠缩放去凑它们正是前四版失效的入口。

三张网格共用同一套骨架参数 `(气口 m, 朱边 b, 笔宽 s, 缝宽 g)`，笔高恒为 `2s + g`
（即两笔加缝的外接框是正方形，这条由 `assert_uniform` 断言）：

| 档位   | 缩放  | m | b  | s(笔宽) | g(缝宽) | 笔高 | 印身占整格 |
| ------ | ----- | - | -- | ------- | ------- | ---- | ---------- |
| 16 px  | 1×    | 1 | 4  | 2       | 2       | 6    | 87.5%      |
| 20 px  | 1.25× | 1 | 5  | 3       | 2       | 8    | 90.0%      |
| 24 px  | 1.5×  | 2 | 6  | 3       | 2       | 8    | 83.3%      |
| 32 px  | 2×    | 2 | 8  | 4       | 4       | 12   | 87.5%      |
| 48 px  | 3×    | 3 | 12 | 6       | 6       | 18   | 87.5%      |
| 64 px  | 4×    | 4 | 16 | 8       | 8       | 24   | 87.5%      |
| 256 px | 16×   | 16| 64 | 32      | 32      | 96   | 87.5%      |

**笔宽逐档恒等于 `floor(2 × 缩放 + 0.5)`**（16→2、20→3、24→3、32→4、48→6、64→8、
256→32），这条在本文件与 `xtask verify-icons` 里各断言一遍：前者查渲染前的网格，
后者查**已写进 ICO 的字节**。v4 的「两档笔画变细」正是缺后一条。

20 与 24 px 的取整不是拍的：`2m + 2b + 2s + g = 边长` 且 `2m + 2b + 笔高 = 边长`，
于是 `g` 与边长同奇偶。20 px 的 `g` 目标值 2.5，可达 2 或 4，取更近的 2，随后
`m + b = 6` 且 `b` 应约等于 5（4 × 1.25），故 `m = 1, b = 5`。24 px 的 `g` 目标 3，
2 与 4 等距，取 2 后 `m + b = 8`、`b = 6`（4 × 1.5 恰好），`m = 2`；取 4 则
`m = 1`、印身占 91.7% 超出 83%~90% 的取景一致区间。

### 深色模式补偿：做了，但要如实说明它的天花板

v4 的朱砂 `#D8452F` 对浅色托盘 3.93:1、对深色托盘 3.73:1——**深色侧明显更弱**，
深色行有「陷进背景」感。需求方给的两条补偿路径逐条判定：

- **给红块加 1 px 米白外描边**：违反本版自己的硬约束（1 px 笔画），且 1 px 在
  24 px 档 ×1.5 = 1.5 px 必然糊。加厚到 2 px 就是上面渲图目视否掉的外框版。**不采用。**
- **把红色提亮**：采用。单个 ICO 无法按系统主题切色（那需要运行时 `set_icon`，属
  产品代码，本任务不改），所以能做的是把这一个朱砂色移到**浅深两侧对比相等**的点。

那个点可以解出来：设浅底相对亮度 `Ll = 0.8963`、深底 `Ld = 0.0144`，两侧对比相等即
`(Ll + 0.05) / (L + 0.05) = (L + 0.05) / (Ld + 0.05)`，得 `L = 0.1969`，此时两侧同为
**3.83:1**——这就是任何单色填充在这两种托盘底色之间能取到的**最大下界**。
本版取 `#DB4634`（`L = 0.1969`），米白同步提亮到 `#FAF6EE` 以保住刻痕对比。

| 组合                | v4               | v5（本版）        | 变化   |
| ------------------- | ---------------- | ----------------- | ------ |
| 朱砂 vs 浅色托盘    | 3.93:1           | 3.83:1            | −2.5%  |
| 朱砂 vs 深色托盘    | **3.73:1**       | **3.83:1**        | **+2.7%** |
| 米白刻痕 vs 朱砂    | 3.81:1           | 3.94:1            | +3.4%  |

**如实结论：深色侧只能净增 2.7%，因为 3.83:1 是算术上限，不是本版偷工。**
所以在对比图上这个差别是弱的——`docs/reports/icon-dark-compensation.png` 把两版
配色同几何并置，就是为了让这个「弱」本身可被目视核验，而不是嘴上声称做了补偿。
两侧对比不再一头轻一头重，是这次补偿真正拿到的东西。要把深色分离做强，只有两条
路：浅色描边（已渲图否掉，理由跨底漂移）或主题感知图标（产品代码，本任务禁改）。

### 为什么刻痕是不透明米白，而不是镂空

**镂空（透明）在深浅两种底色下读法会漂移。** 实测对照：浅色托盘上镂空显 #F3F3F3，
读作「纸上未着墨的字口」；深色托盘上同一枚图标的镂空变成 #202020，读作「打穿的孔
/ 镂雕金属牌」——同一个图标在两种系统主题下读成两个不同的符号。填成不透明米白后，
两种底色下刻痕都是浅色，读法一致。米白在浅色底上几乎与背景同明度（对比 1.03:1），
看上去仍是「留白」，这与真实印章盖在白纸上的样子一致，是特性不是缺陷。

### 已知残留（诚实记录，不粉饰）

1. **两笔等高是需求方指定的硬骨架，代价是「暂停按钮」的第一读风险。** 三根或不等高
   的竖笔在 v2 / v4 被读成柱状图，等高两笔避开了柱状图，但换来与暂停键同构的风险。
   本版靠「笔不通高（上下各留 4 px 朱边）+ 实心方印」压低它，消不掉。
2. **不认汉字或不熟悉篆刻的用户读到的是「一枚红印 + 两道白痕」**，读不出竖排诗行。
   这对品牌识别够用，对语义传达不够。
3. **深色托盘上印面与背景的分离仍偏弱**（3.83:1），原因见上一节的算术上限。

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
#: 米白刻痕。随朱砂提亮同步提亮，以保住刻痕对朱面的对比。
CREAM = (250, 246, 238, 255)
TRANSPARENT = (0, 0, 0, 0)
#: 上一版（v4）的配色，只用于生成深色补偿对照图时的参照，不参与任何产物。
LEGACY_CINNABAR = (216, 69, 47, 255)
LEGACY_CREAM = (245, 239, 225, 255)

#: ASCII 网格的三种记号。`#` 朱砂印面，`o` 米白刻痕，`.` 透明。
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
# 键是边长，值是 (气口 m, 朱边 b, 笔宽 s, 缝宽 g)。笔高恒为 2s + g。
# 两条恒等式：2m + 2b + 2s + g == 边长，且 2m + 2b + (2s + g) == 边长。
# 只列手调档位；16 的整数倍档位由 16 px 定稿按倍数推出，不在这里重复。
SKELETON = {
    16: (1, 4, 2, 2),
    20: (1, 5, 3, 2),
    24: (2, 6, 3, 2),
}

# --------------------------------------------------------------------------- #
# 16 px 定稿网格：本文件的真源。坐标由需求方给定，见文件头那张坐标表。
# --------------------------------------------------------------------------- #
# 逐像素账：气口 1 · 朱边 4 · 笔宽 2 · 缝宽 2 · 笔高 6。
# 左笔 x ∈ [5, 7)、右笔 x ∈ [9, 11)、两笔同为 y ∈ [5, 11)、缝 x ∈ [7, 9)。
# 全网格左右镜像且上下镜像；四边朱边各 4 px，四角气口各 1 px。
MASTER_16 = """
................
.##############.
.##############.
.##############.
.##############.
.####oo##oo####.
.####oo##oo####.
.####oo##oo####.
.####oo##oo####.
.####oo##oo####.
.####oo##oo####.
.##############.
.##############.
.##############.
.##############.
................
"""

# 20 px：气口 1 · 朱边 5 · 笔宽 3 · 缝宽 2 · 笔高 8。印身 18×18（90.0%）。
# 20 不是 16 的整数倍，故单独手调而不是缩放——见文件头的取整一节。
MASTER_20 = """
....................
.##################.
.##################.
.##################.
.##################.
.##################.
.#####ooo##ooo#####.
.#####ooo##ooo#####.
.#####ooo##ooo#####.
.#####ooo##ooo#####.
.#####ooo##ooo#####.
.#####ooo##ooo#####.
.#####ooo##ooo#####.
.#####ooo##ooo#####.
.##################.
.##################.
.##################.
.##################.
.##################.
....................
"""

# 24 px：气口 2 · 朱边 6 · 笔宽 3 · 缝宽 2 · 笔高 8。印身 20×20（83.3%）。
# 气口取 2 而不是 1：取 1 时印身占 91.7%，超出 83%~90% 的取景一致区间，
# 目视表现为「24 px 那格的印几乎撑满、取景与别档不是一套」。
MASTER_24 = """
........................
........................
..####################..
..####################..
..####################..
..####################..
..####################..
..####################..
..######ooo##ooo######..
..######ooo##ooo######..
..######ooo##ooo######..
..######ooo##ooo######..
..######ooo##ooo######..
..######ooo##ooo######..
..######ooo##ooo######..
..######ooo##ooo######..
..####################..
..####################..
..####################..
..####################..
..####################..
..####################..
........................
........................
"""

#: 手调网格：键是边长。其余档位一律由 `MASTER_16` 整数倍复制得到。
MASTERS = {16: MASTER_16, 20: MASTER_20, 24: MASTER_24}


def round_half_up(value: float) -> int:
    """四舍五入，`.5` 向上。

    **不用内建 `round`**：它是银行家舍入，`round(2.5) == 2`，而 20 px 档的笔宽
    目标正是 2.5，用内建会算出 2 并让「笔宽 == floor(2×缩放+0.5)」这条断言
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


def skeleton_for(size: int) -> tuple[int, int, int, int]:
    """该档的骨架参数 `(m, b, s, g)`。整数倍档位按倍数从 16 px 定稿推出。"""
    if size in SKELETON:
        return SKELETON[size]
    assert size % 16 == 0, f"{size} px 既不是手调档位，也不是 16 的整数倍"
    factor = size // 16
    m, b, s, g = SKELETON[16]
    return (m * factor, b * factor, s * factor, g * factor)


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


def _runs_with_start(values: list[bool]) -> list[tuple[int, int]]:
    """各段连续 True 的 (起始下标, 长度)。用来定位两笔各自落在哪几列。"""
    spans: list[tuple[int, int]] = []
    start = None
    for index, on in enumerate(values):
        if on and start is None:
            start = index
        elif not on and start is not None:
            spans.append((start, index - start))
            start = None
    if start is not None:
        spans.append((start, len(values) - start))
    return spans


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
    """
    size = len(grid)
    xs = [x for y in range(size) for x in range(size) if grid[y][x] != "."]
    ys = [y for y in range(size) for x in range(size) if grid[y][x] != "."]
    x0, x1, y0, y1 = min(xs), max(xs), min(ys), max(ys)

    stroke_rows = [y for y in range(size) if len(_runs([c == "o" for c in grid[y]])) == 2]
    assert stroke_rows, f"{size} px 找不到任何「恰有两段刻痕」的行"
    spans = _runs_with_start([c == "o" for c in grid[stroke_rows[0]]])
    (left_x, left_w), (right_x, right_w) = spans
    return {
        "size": size,
        "margin": x0,
        "margin_top": y0,
        "mark": x1 - x0 + 1,
        "mark_h": y1 - y0 + 1,
        "border": left_x - x0,
        "stroke_w": left_w,
        "stroke_w_right": right_w,
        "gap": right_x - (left_x + left_w),
        "stroke_h": len(stroke_rows),
        "first_stroke_row": stroke_rows[0],
        "top_border": stroke_rows[0] - y0,
        "bottom_border": y1 - stroke_rows[-1],
        "right_border": x1 - (right_x + right_w - 1),
    }


def assert_uniform(image: Image.Image, size: int) -> None:
    """机械校验这一档真的达到了设计意图，而不是「看起来生成成功了」。

    每条断言各对应一种**已实际发生过**的失效，逐条见文件头那张四版对照表。
    """
    grid = _classify(image, size)
    got = measure(grid)
    m, b, s, g = got["margin"], got["border"], got["stroke_w"], got["gap"]

    # 约束 0：反推出来的骨架必须与参数表逐项相同。ASCII 网格里少敲一个字符，
    # 这条就会带着「哪一项差多少」变红，而不是让后面的断言给出难以定位的失败。
    want = skeleton_for(size)
    assert (m, b, s, g) == want, (
        f"{size} px 从像素反推的骨架 (m,b,s,g)={(m, b, s, g)} 与参数表 {want} 不符——"
        "ASCII 网格与 SKELETON 至少有一处写错了"
    )

    corners = [(0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1)]
    opaque = [p for p in corners if grid[p[1]][p[0]] != "."]
    assert not opaque, f"{size} px 四角必须透明，但 {opaque} 不是"

    # 约束 1：印面必须是**实心**的。v3 的空心细框加内部全空读不出印章，
    # 用户的原话是「用没有语义替换了错误语义」。这条把「实心朱底」变成可判定的。
    # 印身必须是正方形且四边气口等宽，否则下面用同一对 (x0, x1) 扫描内外都不成立。
    assert got["mark"] == got["mark_h"], (
        f"{size} px 印身 {got['mark']}×{got['mark_h']} 不是正方形"
    )
    assert got["margin"] == got["margin_top"], (
        f"{size} px 左气口 {got['margin']} != 上气口 {got['margin_top']}"
    )
    x0, x1 = m, size - 1 - m
    hollow = [
        (x, y)
        for y in range(x0, x1 + 1)
        for x in range(x0, x1 + 1)
        if grid[y][x] == "."
    ]
    assert not hollow, f"{size} px 印面内出现 {len(hollow)} 个透明像素，首个 {hollow[0]}"

    # 约束 2：任何一段连续朱色或连续刻痕都不小于 2 px，逐行与逐列各查一遍。
    # v1 的月牙牙尖细到亚像素而消失，正是这一条缺位的后果。
    for mark, label in (("#", "朱色"), ("o", "刻痕")):
        for y in range(size):
            widths = _runs([c == mark for c in grid[y]])
            assert all(w >= 2 for w in widths), (
                f"{size} px 第 {y} 行出现 <2 px 的{label}：{widths}"
            )
        for x in range(size):
            widths = _runs([grid[y][x] == mark for y in range(size)])
            assert all(w >= 2 for w in widths), (
                f"{size} px 第 {x} 列出现 <2 px 的{label}：{widths}"
            )

    # 约束 3：刻痕必须被朱色**完全围合**，一个刻痕像素都不许与透明相邻。
    # 印章的识别特征就是「实心朱底 + 边框内有内容」，刻痕一旦破口，印面就散成
    # 几条独立笔画。v2 的「最右竖条顶穿右框线」正是这条缺位的后果。
    for y in range(size):
        for x in range(size):
            if grid[y][x] != "o":
                continue
            for nx, ny in ((x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)):
                assert 0 <= nx < size and 0 <= ny < size and grid[ny][nx] != ".", (
                    f"{size} px 的刻痕像素 ({x},{y}) 与透明相邻，朱边被刻穿"
                )

    # 约束 4：四侧朱边等宽且不小于 2 px——这就是需求方那条「净间距 ≥ 2px」。
    sides = {
        "左": b,
        "右": got["right_border"],
        "上": got["top_border"],
        "下": got["bottom_border"],
    }
    assert len(set(sides.values())) == 1, f"{size} px 四侧朱边不等宽：{sides}"
    assert b >= 2, f"{size} px 朱边只有 {b} px，净间距必须 ≥ 2 px"
    assert g >= 2, f"{size} px 笔间朱缝只有 {g} px，必须 ≥ 2 px"

    # 约束 5：全网格左右镜像**且**上下镜像。
    # v4 刻意放弃了镜像对称（真实楷书「山」三竖不等高），代价是六格第一读全是柱状图；
    # 本版的骨架是两道等宽等高竖笔，镜像对称因此重新成为硬约束而不是审美偏好。
    for y in range(size):
        assert grid[y] == grid[y][::-1], f"{size} px 第 {y} 行不左右镜像"
    for y in range(size):
        assert grid[y] == grid[size - 1 - y], f"{size} px 第 {y} 行与第 {size - 1 - y} 行不上下镜像"

    # 约束 6：刻痕恰为**两个**四连通分量，且两者等宽等高。
    # 「两笔等高等宽」是与柱状图的分水岭（v2/v4 都因不等高被读成柱状图），
    # 这条把它从描述变成断言。
    ink = _components(grid, "o")
    assert len(ink) == 2, f"{size} px 的刻痕有 {len(ink)} 个连通分量，应当恰好 2 道竖笔"
    shapes = []
    for part in ink:
        px = [p[0] for p in part]
        py = [p[1] for p in part]
        shapes.append((max(px) - min(px) + 1, max(py) - min(py) + 1, len(part)))
    assert shapes[0] == shapes[1], (
        f"{size} px 两笔的 (宽, 高, 像素数) 不同：{shapes}——两笔必须等宽等高"
    )
    # 朱色必须仍是一个连通分量：印面不该被刻痕切出孤立朱岛。
    red_parts = len(_components(grid, "#"))
    assert red_parts == 1, f"{size} px 的朱色有 {red_parts} 个连通分量，印面被刻痕切断了"

    # 约束 7：笔宽必须恰等于 `floor(2 × 缩放 + 0.5)`。
    # v4 六档里有两档笔画明显变细，成因是浮点缩放；这条把「逐档取整」变成可判定的。
    scale = size / 16
    expected_stroke = round_half_up(2 * scale)
    assert s == expected_stroke, (
        f"{size} px 笔宽 {s}，但 floor(2 × {scale} + 0.5) = {expected_stroke}——"
        "该档没有按自己的网格取整"
    )
    assert got["stroke_w"] == got["stroke_w_right"], (
        f"{size} px 左笔宽 {got['stroke_w']} != 右笔宽 {got['stroke_w_right']}"
    )

    # 约束 8：两笔加缝的外接框必须是正方形（笔高 == 2×笔宽 + 缝宽）。
    # 这条把「笔群比例跨档一致」变成一个不含容差的等式，不需要 v4 那种 ±0.09 的余量。
    assert got["stroke_h"] == 2 * s + g, (
        f"{size} px 笔高 {got['stroke_h']} != 2×{s} + {g} = {2 * s + g}，笔群外接框不是正方形"
    )

    # 约束 9：印身占整格必须落在 83%~90%，各档取景才是同一个。
    # 目标是 16 / 32 px 那两档恰好命中的 87.5%；20 与 24 px 命中不了（等边距只能给出
    # 离散的几个值），各取可达值里更合适的一个，逐条推导在文件头的取整一节。
    seal = got["mark"] / size
    assert 0.83 <= seal <= 0.90, f"{size} px 印身占整格 {seal:.1%}，应落在 83%~90%"

    # 约束 10：朱色必须占印面的绝大多数，否则「实心朱底」名不副实。
    area = got["mark"] ** 2
    red = sum(1 for y in range(x0, x1 + 1) for x in range(x0, x1 + 1) if grid[y][x] == "#")
    ratio = red / area
    assert 0.83 <= ratio <= 0.92, f"{size} px 朱色占印面 {ratio:.1%}，应落在 83%~92%"


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
        got = measure(_classify(render(size), size))
        report(
            f"  {size:>4} px  气口={got['margin']:<3} 朱边={got['border']:<3} "
            f"笔宽={got['stroke_w']:<3} 缝宽={got['gap']:<3} 笔高={got['stroke_h']:<4} "
            f"印身={got['mark']}({got['mark'] / size:.1%})  "
            f"笔宽期望={round_half_up(2 * size / 16)}"
        )
    for label, cinnabar, cream in (
        ("v5", CINNABAR, CREAM),
        ("v4", LEGACY_CINNABAR, LEGACY_CREAM),
    ):
        report(
            "contrast {} 朱砂/浅托盘={:.2f}:1 朱砂/深托盘={:.2f}:1 刻痕/朱砂={:.2f}:1".format(
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
