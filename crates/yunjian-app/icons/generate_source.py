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

## 造型：朱底白文「山」印（第四版）

**一块实心朱砂方印，印面上以不透明米白刻出一个「山」字。** 朱色是印面主体，米白是
刻痕（白文印 / 阴刻）。四角透明、印面四周留 1 px 透明气口，没有圆角底板——托盘把它
合成到自己背景上时不出现硬边色块。

### 方法论：从 16 px 网格反推，而不是先画大图再缩小

**前三版全部死在同一个流程错误上：先画 1024 母图，再靠取整规则往下缩。** 缩下来的
16 px 一律出事——细负空间消失、斜边退化成 1 px 碎阶梯、笔画顶穿框线。第四版把流程
倒过来：

1. 先在 **16×16 的像素网格上逐像素把图形定下来**（本文件里的 `MASTER_16`，
   一张可以直接读的 ASCII 图），逐条确认每一笔恰好 ≥2 px、每处净空恰好 ≥2 px、
   没有 1 px 碎阶梯、没有贴边、没有开口、左右镜像对称。
2. 16 px 定稿之后才往上放大。**16 的整数倍档位（32/48/64/128/256/512/1024）用精确
   整数倍复制**：本形全部由轴对齐矩形构成，没有任何斜边，所以整数倍复制得到的是
   数学上完全相同的形状——零抗锯齿、零比例漂移，`assert_scales_exactly` 会逐像素复核。
3. **20 px 与 24 px 不是 16 的整数倍，各自另有一份手调网格**（`MASTER_20` /
   `MASTER_24`）。这是专业做法而不是补丁：Windows 在通知区域与若干位置按 20 / 24 px
   取图，靠缩放去凑这两档正是前三版失效的入口。

三张手调网格共用同一套骨架（朱印 + 白文山 + 四周朱边），只在笔画绝对像素数上各自
取整，因此六档读起来是同一个图标，而不是「粗框 / 细描边」交替出现的两种东西。

### 为什么是「山」，以及被逐条淘汰的候选

全部是「渲成放大对照图后按真实像素亲眼看」的结论，不是推理。本轮扫了 27 个候选，
按第一读淘汰：

- **两条全高白竖槽**：第一读**暂停按钮**。加厚朱边只能压低概率，消不掉——暂停键的
  识别特征就是「两条等长竖条居中」，和这个造型同构。
- **三条白竖槽**：第一读**柱状图 / 均衡器**。这与 v2 被否的原因是同一条。
- **三条白横槽（「三」）**：第一读**汉堡菜单**。等长等宽等距的横条是汉堡图标的规范
  形态，换成朱底只改配色不改拓扑。
- **白「回」环（同心方框）**：第一读**二维码定位点**（finder pattern 就是这个形状）。
- **点阵（两列三行 2×2 方格）**：第一读**六点盲文 / 骰子 / 拖拽手柄**。离散孤立的
  方块没有笔画连续性，模式匹配落不到汉字上。`assert_uniform` 的「刻痕必须四连通」
  就是把这条淘汰机械化：一个字是连通的，点阵不是。
- **白「工」/「丁」/「王」**：分别第一读**拉丁字母 I / T**、以及「王」（常见姓氏，
  与古典诗词无语义关联）。「王」的印章质感其实最好，但它把一个不相关的字印在图标上。
- **右下 45° 斜切空框（v3）**：16 px 下斜边退化成一串 1 px 碎阶梯，右框被吃掉导致
  方框读起来是开口的；且框内全空，语义为空而不是错——第一读是「折角文档」。
- **朱红圆底 + 米白月牙（v1）**：16 px 下牙尖细到亚像素而消失，退化成偏心白斑。

「山」是本轮唯一**第一读就是汉字**的候选：三竖一横的结构有横竖相交（篆刻可读性的
关键），不撞任何拉丁字母，不撞任何 UI 控件，而且**语义正好落在山水诗上**。

### 三竖必须高度各不相同——左右对称的「山」会被读成柱状图

**这是本轮最后一次目视推翻。** 先做出来的是左右两竖等高的对称「山」，六格（16/24/32
× 浅底/深底）的**第一读全部是「柱状图 / 信号强度条 / 西里尔字母 Ш」**，不是汉字。
成因是「两侧等高、中间高」正是「中间峰值」的图表语汇，而左右严格对称把汉字线索抹平
成了几何图形。

改成真实楷书「山」的形态——**中竖最高、右竖次高、左竖最短**——之后第一读变成
「一枚红印上刻着一个汉字」。非对称同时排掉了 Ш（要求三竖等高）与柱状图（高度序列
「矮-高-中」不符合任何数据排序直觉）。这一条不是审美偏好，`assert_uniform` 用
**逐行刻痕段数序列必须恰好是 `[1, 2, 3, 1]`** 把它钉死：中竖先单独出头（1 段）、
右竖加入（2 段）、左竖加入（3 段）、底横通宽（1 段）。对称版的序列是 `[1, 3, 1]`，
会直接变红。

**注意这一条与「左右镜像对称」是互斥的**，而后者曾是 v2 的补救措施（v2 三竖长短参差
是无意的 bug，于是加了镜像对称断言）。本版把它换成更准的两条：刻痕外接框在印身内
**左右等距、上下等距**（这才是 v2 「右条顶穿右框线」真正要防的），加上上面那条段数
序列。对称本身从来不是目的。

已知残留：不认汉字的用户只能读到「一枚红印章 + 某个东方文字」，读不出「山」。
这对品牌识别够用，对语义传达不够——是诚实的取舍，候选里没有任何造型两边都占。

### 为什么刻痕是不透明米白，而不是镂空

**镂空（透明）在深浅两种底色下读法会漂移。** 实测对照：浅色托盘上镂空显 #F3F3F3，
读作「纸上未着墨的字口」；深色托盘上同一枚图标的镂空变成 #202020，读作「打穿的孔
/ 镂雕金属牌」——同一个图标在两种系统主题下读成两个不同的符号。填成不透明米白后，
两种底色下刻痕都是浅色，读法一致。米白在浅色底上几乎与背景同明度（对比 1.02:1），
看上去仍是「留白」，这与真实印章盖在白纸上的样子一致，是特性不是缺陷。

### 颜色

| 组合                            | 对比度  |
| ------------------------------- | ------- |
| 朱砂 vs Windows 浅色托盘 #F3F3F3 | 3.93:1  |
| 朱砂 vs Windows 深色托盘 #202020 | 3.73:1  |
| 米白刻痕 vs 朱砂印面             | 3.91:1  |

朱砂沿用第三版的 `#D8452F`：它让浅色底与深色底**双侧都过 WCAG 1.4.11 对非文本图形
的 3:1 门槛**（更暗的 `#C0362C` 浅色底能到 4.97:1，代价是深色底只有 2.95:1，深色行
最小档因此发闷）。米白 `#F5EFE1` 对朱砂 3.91:1，与朱砂对浅色底几乎等值，两行发色
平衡。`_contrast()` 可当场复算这张表。

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

CINNABAR = (216, 69, 47, 255)
CREAM = (245, 239, 225, 255)
TRANSPARENT = (0, 0, 0, 0)

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
# 16 px 定稿网格：本文件的真源。改造型就改这三张图，不改任何公式。
# --------------------------------------------------------------------------- #
# 印身 14×14（外留 1 px 透明气口），朱边 2 px，印面内区 10×10。
# 逐像素账（全部恰好 2 px，没有任何 1 px 的笔画或净空）：
#   朱边 2 · 三竖各 2 · 竖间朱地 2 · 底横 2 · 中竖上方朱地 2
# 三竖高度**刻意不等**：中竖 10 行、右竖 8 行、左竖 6 行。等高会读成柱状图，见文件头。
MASTER_16 = """
................
.##############.
.##############.
.######oo######.
.######oo######.
.######oo##oo##.
.######oo##oo##.
.##oo##oo##oo##.
.##oo##oo##oo##.
.##oo##oo##oo##.
.##oo##oo##oo##.
.##oooooooooo##.
.##oooooooooo##.
.##############.
.##############.
................
"""

# 20 px：印身 18×18（外气口 1 px），朱边 3 px，内区 12×12；笔画 2 px、竖间朱地 3 px。
# 20 不是 16 的整数倍，故单独手调而不是缩放——见文件头的方法论一节。
# 三竖露出底横以上 9 : 7 : 5（中 : 右 : 左），即 16 px 那档 8 : 6 : 4 的同比放大
# （1 : 0.75 : 0.5）。比例必须跨档一致，否则六档读作字重不同的两种东西。
MASTER_20 = """
....................
.##################.
.##################.
.##################.
.########oo########.
.########oo########.
.########oo###oo###.
.########oo###oo###.
.###oo###oo###oo###.
.###oo###oo###oo###.
.###oo###oo###oo###.
.###oo###oo###oo###.
.###oo###oo###oo###.
.###oooooooooooo###.
.###oooooooooooo###.
.###oooooooooooo###.
.##################.
.##################.
.##################.
....................
"""

# 24 px：印身 20×20（外气口 2 px），朱边 3 px，内区 14×14；笔画 2 px、竖间朱地 4 px。
# 外气口取 2 px 而不是 1 px：1 px 时印身占 22/24 = 91.7%，与 16 px 的 87.5%、
# 32 px 的 87.5% 不一致，目视判为「24 px 那格的印几乎撑满、取景与别档不一样」。
# 笔画取 2 px 而不是 4 px：4 px 时朱边(3) < 笔画(4)，中竖顶端目视读作「顶穿印面」。
# **朱边必须不小于笔画**，这是本档取整的硬约束。
# 三竖露出底横以上 11 : 8 : 6，同为 1 : 0.75 : 0.5 的同比取整。
MASTER_24 = """
........................
........................
..####################..
..####################..
..####################..
..#########oo#########..
..#########oo#########..
..#########oo#########..
..#########oo####oo###..
..#########oo####oo###..
..###oo####oo####oo###..
..###oo####oo####oo###..
..###oo####oo####oo###..
..###oo####oo####oo###..
..###oo####oo####oo###..
..###oo####oo####oo###..
..###oooooooooooooo###..
..###oooooooooooooo###..
..###oooooooooooooo###..
..####################..
..####################..
..####################..
........................
........................
"""

#: 手调网格：键是边长。其余档位一律由 `MASTER_16` 整数倍复制得到。
MASTERS = {16: MASTER_16, 20: MASTER_20, 24: MASTER_24}


def parse_master(art: str) -> list[str]:
    """把 ASCII 网格解析成逐行字符串，并校验它是正方形、只含三种记号。"""
    rows = [line for line in art.strip("\n").split("\n")]
    side = len(rows)
    for index, row in enumerate(rows):
        assert len(row) == side, f"第 {index} 行长度 {len(row)} != 边长 {side}"
        unknown = set(row) - set(GLYPH)
        assert not unknown, f"第 {index} 行出现未知记号 {sorted(unknown)}"
    return rows


def master_rows(size: int) -> list[str]:
    """给定尺寸的字符网格。手调档位直接取，其余按 16 px 定稿整数倍复制。"""
    if size in MASTERS:
        rows = parse_master(MASTERS[size])
        assert len(rows) == size, f"{size} px 的手调网格边长是 {len(rows)}"
        return rows
    assert size % 16 == 0, (
        f"{size} px 既不是手调档位 {sorted(MASTERS)}，也不是 16 的整数倍。"
        "本设计刻意不提供任意尺寸的缩放规则——那正是前三版失效的入口。"
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
    """各段连续 True 的 (起始下标, 长度)。用来定位三竖各自落在哪几列。"""
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


def _components(grid: list[list[str]], mark: str) -> int:
    """`mark` 记号的四连通分量个数。"""
    size = len(grid)
    cells = {(x, y) for y in range(size) for x in range(size) if grid[y][x] == mark}
    count = 0
    while cells:
        count += 1
        frontier = [cells.pop()]
        while frontier:
            x, y = frontier.pop()
            for nb in ((x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)):
                if nb in cells:
                    cells.remove(nb)
                    frontier.append(nb)
    return count


def assert_uniform(image: Image.Image, size: int) -> None:
    """机械校验这一档真的达到了设计意图，而不是「看起来生成成功了」。

    每条断言各对应一种**已实际发生过**的失效，逐条见文件头。
    """
    grid = _classify(image, size)

    corners = [(0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1)]
    opaque = [p for p in corners if grid[p[1]][p[0]] != "."]
    assert not opaque, f"{size} px 四角必须透明，但 {opaque} 不是"

    xs = [x for y in range(size) for x in range(size) if grid[y][x] != "."]
    ys = [y for y in range(size) for x in range(size) if grid[y][x] != "."]
    x0, x1, y0, y1 = min(xs), max(xs), min(ys), max(ys)

    # 约束 1：印面必须是**实心**的。v3 的空心细框加内部全空读不出印章，
    # 用户的原话是「用没有语义替换了错误语义」。这条把「实心朱底」变成可判定的。
    hollow = [
        (x, y)
        for y in range(y0, y1 + 1)
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
    # 这一条同时锁住「朱边厚度」与「刻痕不贴边」：印章的识别特征就是
    # 「实心朱底 + 边框内有内容」，刻痕一旦破口，印面就散成几条独立笔画。
    for y in range(size):
        for x in range(size):
            if grid[y][x] != "o":
                continue
            for nx, ny in ((x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)):
                assert 0 <= nx < size and 0 <= ny < size and grid[ny][nx] != ".", (
                    f"{size} px 的刻痕像素 ({x},{y}) 与透明相邻，朱边被刻穿"
                )

    # 约束 4：朱边四侧各不小于 2 px。上面那条只保证「不破口」，保不了「够厚」。
    #
    # 左右两侧只能在**底横那一行**上量：底横通宽，所以该行的首尾朱段恰好就是朱边。
    # 在别的行上量会把「竖间朱地」和朱边连成一段（例如 20 px 的中段量出 8 而不是 3）。
    # 上下两侧在**中竖那一列**上量，同理。
    base_row = max(y for y in range(size) if any(c == "o" for c in grid[y]))
    mid_x = (x0 + x1) // 2
    borders = {
        "上": _runs([grid[y][mid_x] == "#" for y in range(y0, y1 + 1)])[0],
        "下": _runs([grid[y][mid_x] == "#" for y in range(y0, y1 + 1)])[-1],
        "左": _runs([grid[base_row][x] == "#" for x in range(x0, x1 + 1)])[0],
        "右": _runs([grid[base_row][x] == "#" for x in range(x0, x1 + 1)])[-1],
    }
    thin = {k: v for k, v in borders.items() if v < 2}
    assert not thin, f"{size} px 朱边过薄：{thin}（四侧实测 {borders}）"

    # 约束 5：刻痕外接框在印身内**左右等距、上下等距**。
    # v2 的「右条顶穿右框线」真正要防的是这个。刻意**不**用逐像素镜像对称：
    # 那会顺手禁掉真实「山」的三竖不等高，而三竖等高正是本轮读成柱状图的成因。
    ink_x = [x for y in range(size) for x in range(size) if grid[y][x] == "o"]
    ink_y = [y for y in range(size) for x in range(size) if grid[y][x] == "o"]
    insets = {
        "左": min(ink_x) - x0,
        "右": x1 - max(ink_x),
        "上": min(ink_y) - y0,
        "下": y1 - max(ink_y),
    }
    assert insets["左"] == insets["右"], f"{size} px 刻痕左右不等距：{insets}"
    assert insets["上"] == insets["下"], f"{size} px 刻痕上下不等距：{insets}"

    # 约束 6：刻痕必须是**一个**四连通分量——也就是一个字，而不是一片点阵。
    # 本轮的点阵候选（两列三行 2×2 方格）第一读是六点盲文 / 骰子 / 拖拽手柄，
    # 因为离散方块没有笔画连续性。这条把那次目视淘汰机械化。
    ink_parts = _components(grid, "o")
    assert ink_parts == 1, f"{size} px 的刻痕有 {ink_parts} 个连通分量，应当只有 1 个（一个字）"
    # 朱色同理：印面不该被刻痕切出孤立朱岛。
    red_parts = _components(grid, "#")
    assert red_parts == 1, f"{size} px 的朱色有 {red_parts} 个连通分量，印面被刻痕切断了"

    # 约束 6.5：印身占整格的比例必须落在 83%~90%，各档取景才是同一个。
    #
    # 目标是 16 / 32 px 那两档恰好命中的 87.5%（外气口 1 / 2 px）。**手调的两档都命中
    # 不了它**，因为等边距只能给出离散的几个值：
    #   20 px：87.5% 要求印身 17.5 px。可达 90%（气口 1）或 80%（气口 2）→ 取 90%。
    #   24 px：87.5% 要求印身 21 px，21 是奇数。可达 83.3%（气口 2）或 91.7%（气口 1）
    #          → 取 83.3%，因为 91.7% 那版目视判「印几乎撑满、取景与别档不一样」。
    # 区间上下界就是这两个不可命中值，不是拍的。
    seal = (x1 - x0 + 1) / size
    assert 0.83 <= seal <= 0.90, f"{size} px 印身占整格 {seal:.1%}，应落在 83%~90%"

    # 约束 7：朱色必须占印面的多数，否则「实心朱底」名不副实（会读成白底红框）。
    area = (x1 - x0 + 1) * (y1 - y0 + 1)
    red = sum(1 for y in range(y0, y1 + 1) for x in range(x0, x1 + 1) if grid[y][x] == "#")
    ratio = red / area
    assert 0.55 <= ratio <= 0.85, f"{size} px 朱色占印面 {ratio:.1%}，应落在 55%~85%"

    # 约束 8：「山」的骨架，同时钉住「三竖高度各不相同」。
    #
    # 逐行数刻痕段数，序列必须**恰好**是 `[1, 2, 3, 1]`：中竖先单独出头（1 段）、
    # 右竖加入（2 段）、左竖加入（3 段）、底横通宽（1 段）。这一条一次挡掉四种失效：
    #   `[1, 3, 1]`  → 左右两竖等高的对称「山」，六格第一读全是柱状图 / Ш（本轮实测）
    #   `[3]`        → 三条平行竖槽，读作柱状图 / 均衡器（v2 的根因）
    #   `[1]`        → 三条平行横槽，读作汉堡菜单
    #   出现 `2` 却无 `3` 或段数来回跳 → 点阵 / 散碎笔画
    profile = [len(_runs([c == "o" for c in grid[y]])) for y in range(size)]
    seen = [n for n in profile if n]
    collapsed = [n for i, n in enumerate(seen) if i == 0 or n != seen[i - 1]]
    assert collapsed == [1, 2, 3, 1], (
        f"{size} px 刻痕逐行段数序列是 {collapsed}，应为 [1, 2, 3, 1]"
        "（中竖 → 加右竖 → 加左竖 → 底横）"
    )
    # 三竖的高度必须两两不等，且中竖最高。上面那条序列已经隐含了它，
    # 这里直接量出来当断言，好让失败信息里带着三个具体高度而不是一串段数。
    widest = next(y for y in range(size) if profile[y] == 3)
    columns = [x for x, _ in _runs_with_start([c == "o" for c in grid[widest]])]
    heights = [
        sum(1 for y in range(size) if grid[y][x] == "o") for x in columns
    ]
    left, middle, right = heights
    assert middle > right > left, (
        f"{size} px 三竖高度 左={left} 中={middle} 右={right}，"
        "必须中竖最高、右竖次高、左竖最短——等高会读成柱状图"
    )
    # 三竖露出底横以上的高度比必须跨档一致（16 px 定稿是 8 : 6 : 4 = 1 : 0.75 : 0.5）。
    # 不一致时六档会读成字重不同的两种东西，与 v2 六档「粗框 / 细描边交替」同类。
    # 容差 ±0.09 是取整不可避免的余量（例如 20 px 的 7/9 = 0.78 对目标 0.75）。
    base_thick = sum(1 for y in range(widest + 1, size) if profile[y] == 1)
    exposed = [h - base_thick for h in heights]
    for label, actual, want in (
        ("右", exposed[2] / exposed[1], 0.75),
        ("左", exposed[0] / exposed[1], 0.50),
    ):
        assert abs(actual - want) <= 0.09, (
            f"{size} px {label}竖露出底横以上 {exposed} 之比为 {actual:.2f}，"
            f"应约等于 {want}（16 px 定稿 8 : 6 : 4 的同比）"
        )
    # 底横必须真的通宽（顶满内区），否则读作「凵」而不是「山」。
    last = base_row
    base = _runs([c == "o" for c in grid[last]])[0]
    inner = x1 - x0 + 1 - 2 * borders["左"]
    assert base == inner, f"{size} px 底横宽 {base}，内区宽 {inner}，底横没有通宽"


def assert_scales_exactly(size: int) -> None:
    """16 的整数倍档位必须与 16 px 定稿**逐像素**一致，比例零漂移。

    这是构造保证（整数倍复制轴对齐矩形），但仍要复核：一旦有人把某档改成
    「缩放母图」，比例就会漂移，而那正是 v2 六档读作两种东西的成因。
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
    """WCAG 对比度。留在这里以便有人改颜色时能当场复算文件头那张表。"""

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
    report(
        "contrast 朱砂/浅托盘={:.2f}:1 朱砂/深托盘={:.2f}:1 刻痕/朱砂={:.2f}:1".format(
            _contrast(CINNABAR[:3], (243, 243, 243)),
            _contrast(CINNABAR[:3], (32, 32, 32)),
            _contrast(CREAM[:3], CINNABAR[:3]),
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
