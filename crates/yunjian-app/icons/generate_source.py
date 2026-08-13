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
（全是边缘抗锯齿混出的中间色），原生渲染的同一层只有 **2** 种。所以 ICO 由本脚本
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

## 设计：朱色切角方印（第三版）

**一枚朱砂方框，右下角以 45 度整体切掉。** 方框是印章的阳线框（不是承托徽标的圆角
底板：它四角透明、框内大面积透明，托盘把它合成到自己背景上时不出现硬边色块，而且
框是**方**的——圆角矩形正是 launcher 底板的形状语言）。右下切角是笺纸的折角。

造型只由**一条轮廓**构成，框内**没有任何元素**。这不是偷懒，是前两版失效与本轮
探针共同逼出来的唯一解，见下节。

### 三条硬约束（每条都由一次真实失效反推出来，且都被 `assert_uniform` 机械校验）

1. **所有笔画在 16 px 下不小于 2 px。**（v1 的月牙牙尖细到亚像素级而消失。）
   校验方式：逐行、逐列取连续不透明段，**任何一段的长度都必须 >= 2**。
2. **框内元素与外框净间距不小于 2 px。** 本版框内为空，故此约束**空成立**——
   这不是规避，而是第 3 节说明的推论：16 px 下框内根本装不下任何合格元素。
3. **对称。** 本形关于主对角线（左上→右下）镜像对称，
   校验方式：`image == image.transpose(TRANSPOSE)`，逐像素相等。
   （v2 的三竖条长短参差、右条顶穿右框线，正是这一条缺位的后果。）

### 为什么框内必须是空的：三轮探针的逐条淘汰记录

全部是「渲成联系表后按真实像素尺寸亲眼看」的结论，不是推理：

- **「云」字骨架（本任务用户建议的方向一）**：云 = 二 + 厶，骨架**本质上是两道平行横**。
  在 16 px 上无论怎样调比例、加粗、改厶的写法（对称 ⊔ / L+点 / 三横递宽），
  一律读作**等号**。这一条否掉了整个「框内放字」的方向，包括加框与不加框两种。
- **「笺」字**：竹字头 + 戋，笔画密度在 16 px 上连轮廓都合不出来，未进探针。
- **框内竖行（方向二）**：1 道读作**文本光标**，2 道读作**暂停键**，3 道读作
  **柱状图 / 均衡器**（v2 正是三道，被否）。竖行数量没有安全值。
- **等长成列的横线阵列（竖排诗行的字面模拟）**：单条线宽必须 1 px 才排得下，
  违反约束 1。
- **白文印（实心块挖出负形字）**：最耐缩放，但它正是 v2 被否时那句
  「深色行最小档几乎读作实心红方块中间挖了几道缝」。作废。
- **开卷（对称书页）**：16 px 读作两根柱子；页面弧度在 16 px 全丢，跨档形变最明显。
- **框内加天头横楣**：16 px 下横楣与上框线之间只剩 1 px，读作**浏览器窗口 / 对话框**。
- **「云字只在装得下的档位（>=48 px）出现」**：探针里 16/24 px 与 32/64/256 px
  读作**两个不同的图标**，且 32 px 的字被折角挤到左上、笔画间只剩 1 px。作废。
- **右上切角**：那是通用「文档」图标的角，语义被占用；改到**右下**后不再撞车。
- **实心折片（切角处再补一块三角当折起的纸）**：把右下并成一块厚区，
  四边不再等宽，视觉重心整体歪向右下。作废——「矩形 + 一个切角三角」本来就是
  把角**切掉**，不需要再补回来。

结论：**16 px 下没有任何东西能同时说清「古典诗词」又结构端正。** 因此本版把
辨识度全部押在**轮廓**上（方 + 右下切角），把文化指向交给**朱砂**与**方印比例**。
这是诚实的取舍，不是「256 px 下也能读出印章的篆字」——它读不出，本版没有字。

### 逐尺寸取整，而不是缩放母图

`layout()` 对每个目标尺寸各取整一次，**没有任何逐档手调覆写**——取整规则本身就够了。
规则不是审美偏好，是两条实测反推的：

1. **v2 的框线比例在各档之间反复跳**（16 px 占 12.5%、24 px 占 8.3%，相差 1.5 倍），
   于是六档读作「粗框 / 细描边」交替出现的两种东西。本版框厚统一按 span 的 3/16
   **向下**取整，实测 14.3% / 16.7% / 18.2% / 17.9% / 16.7% / 17.9% / 18.8%，
   最大与最小相差 1.31 倍，且没有交替。
   **向下取整是关键**：3/16 是上限不是目标。向上取整时 64 px 得到 11 px 框，
   框内空腔掉到 span² 的 29.5%；16 px 得到 3 px 框，空腔从 90 px² 掉到 58 px²，
   探针里读作「实心红块被啃掉一角」。向下取整让这两档自然落在 10 px 与 2 px 上。
2. **切角长度一度被 `max(2 * thick, ...)` 夹住**，导致 1/4、5/16、3/8 三种切角比例
   在 16 px 与 256 px 上算出**完全相同的像素**——探针里三行同形，正是这个夹逼。

**切角长度取 span 的一半，不取任何分数比例。** 于是切口两端精确落在下框与右框的
**中点**上；span 恒为偶数（size 与 2 * margin 都是偶数），这个锚点在任何档位都是整数，
切角比例**零漂移**，是构造保证而非碰巧，`assert_uniform` 会复核 `fold * 2 == span`。
扫过的更短切角：3/8 与 7/16 在 16 px 上斜边只有 3~4 级台阶，读作「描边没对齐的毛刺」
而不是一刀；更长的 9/16 越过中点，斜边长度超过残留的下框与右框，方框感被削掉。

**45 度斜边按垂直于笔画的宽度对齐正交边**：半平面沿 `x + y` 轴的偏移量取
`round(thick * sqrt(2))`，于是斜带的垂直宽度 = 偏移 / sqrt(2) ≈ 框厚。
直接用 `thick` 当偏移会让斜边细成 0.71 倍（16 px 上是 1.41 px，违反约束 1）。

## 颜色

朱砂 `#D8452F` 单色。**比 v2 的 `#C0362C` 亮**，这是为了让深色托盘也可用：

| 组合                               | v2 `#C0362C` | 本版 `#D8452F` |
| ---------------------------------- | ------------ | -------------- |
| vs Windows 浅色托盘 `#F3F3F3`      | 4.97:1       | 3.93:1         |
| vs Windows 深色托盘 `#202020`      | **2.95:1**   | **3.73:1**     |
| vs 纯白 `#FFFFFF`                  | 5.52:1       | 4.36:1         |

v2 那个红把浅色底的对比度拉到 4.97:1，代价是深色底只有 2.95:1——低于 WCAG 1.4.11
对非文本图形的 3:1 门槛，深色行 16 px 因此发闷。本版**两侧都过 3:1**，
取的是平衡而不是把浅色底单边拉满。`_contrast()` 可当场复算这张表。

## macOS squircle：明示不处理，及理由

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
import math
import pathlib
import struct
import tempfile

from PIL import Image

CINNABAR = (216, 69, 47, 255)
TRANSPARENT = (0, 0, 0, 0)

# 框厚以「span（去掉外边距后的边长）」为基准，逐档各取整一次。
THICK_FRACTION = 3 / 16

SOURCE_SIZE = 1024
# Windows 通知区域与多数 Linux 面板按 32 px 取托盘图（高 DPI 下再放大）。
TRAY_SIZE = 32
# **顺序即写入顺序**：32 px 在最前，因为开发工具与任务栏取第一层。
ICO_LAYER_ORDER = (32, 16, 24, 48, 64, 256)
# 生成时逐档自检的尺寸。20 px 不是 ICO 层，但 Windows 某些位置按 20 px 取图，
# 而它恰好是取整最容易出岔子的档（span 18，3/16 落在 3.375 上），所以一并校验。
AUDIT_SIZES = (16, 20, 24, 32, 48, 64, 128, 256, SOURCE_SIZE)


def _round_half_up(value: float) -> int:
    """Python 的 `round` 对 .5 取偶，会让 20/32 px 这类档位的取整跳向意料之外的一侧。"""
    return math.floor(value + 0.5)


def layout(size: int) -> dict:
    """把设计比例在给定尺寸上取整成一组整数几何。"""
    margin = max(1, size // 16)
    span = size - 2 * margin
    # 框厚**向下**取整：3/16 是上限而不是目标值。向上取整过的实测后果是 64 px 得到
    # 11 px 框（占 19.6%），框内空腔掉到 span² 的 29.5%，读作实心块。向下取整落在 10 px。
    #
    # 下限是 **3 px 而不是 2 px**，这是被约束 1（笔画不小于 2 px）逼出来的，不是审美：
    # 斜带每行占 `slant = thick` 个像素，它**垂直于笔画**的宽度是 thick / sqrt(2)。
    # thick = 2 时垂直宽度只有 1.41 px，低于 2 px——真实发生过，四轮目视都指着 16 px
    # 说斜边比正交边弱。而把 slant 单独加到 3 又会让斜带每行比正交边宽一个像素，
    # 于是「四边等宽」不成立（那一版被判「斜边明显偏粗」）。两条同时满足只有一个解：
    # thick >= ceil(2 * sqrt(2)) = 3。于是 16 px 与 20 px 都用 3 px 框。
    thick = max(3, math.floor(span * THICK_FRACTION))

    # 切角长度恰为 span 的一半，于是切口两端**精确落在**下框与右框的中点上。
    # span 恒为偶数（size 与 2*margin 都是偶数），所以这个锚点在任何档位都是整数，
    # 切角比例零漂移——这是构造保证，不是碰巧，`assert_uniform` 会复核。
    # 试过的更短切角：3/8 与 7/16 在 16 px 上只有 3~4 级台阶，读作描边没对齐的毛刺；
    # 更长的 9/16 越过中点，斜边反客为主，方框感被削掉。
    fold = span // 2

    # 斜边沿 `x + y` 轴的内移量 = 框厚，于是斜带**每行的连续段长度恰好等于框厚**，
    # 与正交边逐像素等宽。试过 round(thick * sqrt(2))（让斜带的*垂直*宽度等于框厚）：
    # 那让斜带每行占 sqrt(2) 倍的像素，16/24/32 px 上两轮目视都判「斜边明显比正交边粗」，
    # 16 px 更被读成「框内一道斜杠」而不是外轮廓被切掉一角。
    # 人眼比的是每行的可见段长，不是数学上的垂直宽度；斜线本身还会被感知得更重。
    slant = thick

    return {
        "margin": margin,
        "span": span,
        "thick": thick,
        "fold": fold,
        "slant": slant,
        "outer0": margin,
        "outer1": size - margin - 1,
        "inner0": margin + thick,
        "inner1": size - margin - 1 - thick,
    }


def render(size: int) -> Image.Image:
    """按目标尺寸原生渲染，全部几何落在整数像素边界上。

    没有超采样、没有降采样：两者都会引入抗锯齿中间色，而 v1 正是死在中间色渗色上。
    形状 = 外五边形 减去 内五边形，两者的斜边共用同一族 45 度半平面，
    因此斜边与正交边在拐角处自然成一个干净的斜接（miter），不留 1 px 碎片。
    """
    spec = layout(size)
    o0, o1 = spec["outer0"], spec["outer1"]
    i0, i1 = spec["inner0"], spec["inner1"]
    cut = o1 + o1 - spec["fold"]

    img = Image.new("RGBA", (size, size), TRANSPARENT)
    for y in range(size):
        for x in range(size):
            outer = o0 <= x <= o1 and o0 <= y <= o1 and (x + y) <= cut
            inner = i0 <= x <= i1 and i0 <= y <= i1 and (x + y) <= cut - spec["slant"]
            if outer and not inner:
                img.putpixel((x, y), CINNABAR)
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


def _opaque(image: Image.Image) -> list[list[bool]]:
    return [
        [image.getpixel((x, y))[3] > 0 for x in range(image.width)]
        for y in range(image.height)
    ]


def assert_uniform(image: Image.Image, size: int) -> None:
    """机械校验这一档真的达到了设计意图，而不是「看起来生成成功了」。

    每条断言各对应一种已实际发生过的失效，见文件头。
    """
    colors = {image.getpixel((x, y)) for y in range(size) for x in range(size)}
    assert colors <= {TRANSPARENT, CINNABAR}, f"{size} px 出现中间色：{sorted(colors)}"

    corners = [
        image.getpixel(p)
        for p in ((0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1))
    ]
    assert all(px[3] == 0 for px in corners), f"{size} px 四角必须透明：{corners}"

    grid = _opaque(image)
    # 约束 3：关于主对角线镜像对称。v2 的长短参差与顶穿框线都是这一条缺位的后果。
    # 直接比 `grid[y][x]` 与 `grid[x][y]`，不走 `Image.getdata()`——后者在 Pillow 13
    # 上已弃用，而这条断言必须在无警告的情况下长期可跑。
    asymmetric = [
        (x, y) for y in range(size) for x in range(size) if grid[y][x] != grid[x][y]
    ]
    assert not asymmetric, (
        f"{size} px 不满足主对角线镜像对称，首个不对称像素 {asymmetric[0]}"
    )
    # 约束 1：任何一段连续笔画都不小于 2 px，逐行与逐列各查一遍。
    for y, row in enumerate(grid):
        widths = _runs(row)
        assert all(w >= 2 for w in widths), f"{size} px 第 {y} 行出现 <2 px 笔画：{widths}"
    for x in range(size):
        widths = _runs([grid[y][x] for y in range(size)])
        assert all(w >= 2 for w in widths), f"{size} px 第 {x} 列出现 <2 px 笔画：{widths}"

    spec = layout(size)
    thick, o0, o1 = spec["thick"], spec["outer0"], spec["outer1"]
    i0, i1 = spec["inner0"], spec["inner1"]

    # 四边**与斜边**逐行逐列等宽。凡不是「整条实心」的行/列，其每一段都必须等于框厚。
    # 只探一行量不出斜边偏粗——v2 的「右边明显比左边厚」就是这样漏过去的。
    for y, row in enumerate(grid):
        widths = _runs(row)
        if len(widths) == 1:
            continue
        assert all(w == thick for w in widths), (
            f"{size} px 第 {y} 行段宽 {widths} 不等于框厚 {thick}"
        )
    for x in range(size):
        widths = _runs([grid[y][x] for y in range(size)])
        if len(widths) == 1:
            continue
        assert all(w == thick for w in widths), (
            f"{size} px 第 {x} 列段宽 {widths} 不等于框厚 {thick}"
        )

    # 切口两端必须精确落在下框与右框的中点上，否则切角比例会随档位漂移。
    assert spec["span"] % 2 == 0, f"{size} px 的 span {spec['span']} 不是偶数"
    assert spec["fold"] * 2 == spec["span"], (
        f"{size} px 切角 {spec['fold']} 不是 span {spec['span']} 的一半"
    )

    # 全部墨色像素必须四连通，即整条轮廓是一笔闭合的、没有游离像素或断口。
    # 这一条把「斜边与下框之间有缝、斜边悬浮」这类目视指控变成可判定的：
    # 放大 16 倍看台阶时人眼极容易把 45 度阶梯误读成断开，字节不会。
    ink = [(x, y) for y in range(size) for x in range(size) if grid[y][x]]
    reached = {ink[0]}
    frontier = [ink[0]]
    while frontier:
        x, y = frontier.pop()
        for nx, ny in ((x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)):
            if (
                0 <= nx < size
                and 0 <= ny < size
                and grid[ny][nx]
                and (nx, ny) not in reached
            ):
                reached.add((nx, ny))
                frontier.append((nx, ny))
    assert len(reached) == len(ink), (
        f"{size} px 轮廓不是单一连通域：{len(ink)} 个墨色像素里只有 {len(reached)} 个连通"
    )

    # 约束 1 用在斜带上：斜带**垂直于笔画**的宽度也必须 >= 2 px。
    # 单看每行段长会漏掉这条——段长 2 px 的 45 度斜带垂直宽度只有 1.41 px。
    perpendicular = spec["slant"] / math.sqrt(2)
    assert perpendicular >= 2, (
        f"{size} px 斜带垂直宽度 {perpendicular:.2f} px < 2 px（框厚 {thick}）"
    )

    # 空腔不能被框线与切角吃干净：v2 的 16 px 就是这样读成实心块的。
    cavity = sum(
        1
        for y in range(i0, i1 + 1)
        for x in range(i0, i1 + 1)
        if not grid[y][x]
    )
    ratio = cavity / (spec["span"] ** 2)
    # 下限 25%：16 px 是最紧的一档（27.6%），因为 3 px 框在 14 px 的 span 上占 21.4%。
    assert ratio >= 0.25, f"{size} px 框内空腔只占 span² 的 {ratio:.1%}，低于 25%"


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
    report(f"audited sizes={list(AUDIT_SIZES)}")

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
