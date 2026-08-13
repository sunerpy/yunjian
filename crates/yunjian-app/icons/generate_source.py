#!/usr/bin/env python3
"""生成云笺图标：源图、托盘图，以及**逐尺寸原生渲染**的 `icon.ico`。

用法（在仓库根执行，顺序不可换）：

    python3 crates/yunjian-app/icons/generate_source.py
    cd crates/yunjian-app && cargo tauri icon icons/source-1024.png
    python3 crates/yunjian-app/icons/generate_source.py --ico-only
    cargo run -p xtask -- verify-icons

**第三步不是多余的。** `cargo tauri icon` 生成 `icon.ico` 的方式是把 1024 源图
**降采样**到各层，于是本脚本在 16 px 原生画出来的清晰版本根本进不了 ICO——用户在
任务栏看到的仍然是一张 1024 缩下来的糊图。实测对照：降采样的 32 px 层有 **30** 种
颜色（全是边缘抗锯齿混出的中间色），原生渲染的同一层只有 **2** 种。所以 ICO 由本
脚本自己写字节，在 `cargo tauri icon` 跑完之后覆盖它那一份；`--ico-only` 就是这一步。
`xtask verify-icons` 有一条颜色数上限断言专门抓「漏跑了第三步」。

产出：

- **`source-1024.png`** — 1024×1024 RGBA，喂给 `cargo tauri icon` 的源图。
  **刻意不叫 `icon.png`**：`cargo tauri icon` 会把 `icons/icon.png` 覆写成 512×512
  （实测），源图与它同名会导致下一轮拿上一轮的 512 当源，每跑一次掉一档分辨率，
  且 `verify-icons` 那条「源图必须 1024×1024」的断言会莫名变红。
- **`tray.png`** — 32×32 RGBA，系统托盘图标，按 32 px 原生渲染。
- **`icon.ico`** — 六层 `[32, 16, 24, 48, 64, 256]`，**32 px 在最前**（开发工具与
  任务栏取第一层），每层都是该尺寸的原生渲染。

## 设计：朱印方框 + 三道悬顶竖行

题材关联落在**竖排诗行**与**印章**这两个具体形制上，不只是配色沾边：

- **外框是印章的边框**（朱文印的阳线框），不是承托徽标的圆角底板。它四角透明、
  框内大面积透明，系统托盘把它合成到自己背景上时不会出现一块硬边色块。
  这也是为什么框是**方**的而不是圆角矩形：圆角矩形正是 launcher 底板的形状语言。
- **框内三道竖条自顶端垂下、长度递减**（右起长/中/短）。竖排是汉字诗稿的书写方向；
  长度递减是「末行字数不足」的自然结果。**顶端必须齐平**：这是与柱状图的唯一可靠
  区分点——柱状图从底部基线向上生长、顶端参差；诗行从顶端悬垂、底端参差。探针里把
  三行改成齐底后立刻读成柱状图。

### 被 16 px 探针淘汰的候选（渲染成七档联系表后亲眼看，不是推理）

- **朱红圆底 + 米白月牙**（本任务上一版，已被否）：16 px 下月牙上下牙尖完全消失，
  退化成偏心白斑；红白高对比经抗锯齿混出大量粉色中间色，浅底上一圈粉雾；圆环左厚
  右薄、轮廓多边形化。且「月 + 圆」是通用夜间模式图标语言，与诗词无关。
  **根因是造型而非导出参数**：月牙的负空间在 16 px 细到亚像素级，换重采样算法救不回来。
- **实心朱红圆角矩形 + 竖行**：16 px 可辨，但那个矩形正是被禁止的自绘底板。
- **实心朱印 + 竖行**（框内填满）：等价于一整块红方块，同上。
- **立轴（上下轴头出挑）**：16 px 读成希腊神庙立柱，或工字钢。
- **对开册页 / 中缝镂空**：读成暂停按钮。
- **实心印 + 二横（阳文简化）**：读成汉堡菜单或等号。
- **云形（一至三瓣）**：瓣间沟被下采样平均掉，读成面包。
- **云 + 月并置**：两个相邻物体合成一团，读成用户头像。
- **框内三行 + 落款方点**：1024 下好看，但 16/24 px 下款点与最短竖条（或与下框）
  并成一团 L 形色块。**第四个元素在 16 px 一律不存在**，故设计里没有它。

### 16 px 可辨性如何反向决定造型

四条硬约束，全部由「16 px 下必须认得出」推出，而不是先画好再检查：

1. **一切几何落在整数像素边界上、逐尺寸原生渲染**，因此 16 px 渲出来**只有 2 种
   颜色**（朱砂与全透明），抗锯齿中间色数为 0。上一版的粉雾不是调参数调掉的，
   是让边缘不再需要抗锯齿。
2. **笔画最细 2 px**（16 px 档）。1 px 笔画在缩放与 DPI 取整下会时隐时现。
3. **负空间最窄 1 px 且必须是直线**。曲线负空间（上一版月牙的凹口）在 16 px 必然糊。
4. **元素数 ≤ 2 类**（框 + 竖条），无渐变、无阴影、无内部小细节、无第四个物体。

### 为什么用「相对比例 + 逐尺寸取整」而不是缩放母图

`RELATIVE` 是以 32 单位网格表达的比例，`layout()` 对每个目标尺寸**各取整一次**，
三道竖条随后由整数运算落位，因此任何尺寸下三条必然等宽——这是构造保证，不是碰巧。
`MINIMUM` 给出小尺寸的下限（框线 2 px、竖条 2 px、间隙 1 px），`assert_uniform`
在生成时逐档校验「三竖条等宽、左右框线等宽、颜色数 ≤ 2、四角透明」，不满足即失败。

实测过的反例：先用 16 单位网格按浮点比例算各列边界，20 px 得到笔画
`[2,3,2,3,2]`、24 px 得到 `[3,3,4,3,3]`——同一张图里笔画忽粗忽细，而这**不会报错**。

## 颜色

朱砂 `#C0362C` 单色。WCAG 对比度（`_contrast` 可复算）：

| 组合                               | 比值   |
| ---------------------------------- | ------ |
| 朱砂 vs Windows 浅色托盘 `#F3F3F3` | 4.97:1 |
| 朱砂 vs Windows 深色托盘 `#202020` | 2.95:1 |
| 朱砂 vs 纯白 `#FFFFFF`             | 5.52:1 |

浅深两种系统托盘都 ≥ 2.5:1，所以**一套图标同时服务两种主题**，不发两套。
单色也顺带消掉了「两色相邻处产生抗锯齿中间色」这个上一版的失效模式。

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
import pathlib
import struct
import tempfile

from PIL import Image, ImageDraw

CINNABAR = (192, 54, 44, 255)
TRANSPARENT = (0, 0, 0, 0)

# 以 32 单位网格表达的比例。`layout()` 对每个目标尺寸各取整一次。
RELATIVE = {
    "margin": 2 / 32,
    "thick": 3 / 32,
    "col_w": 4 / 32,
    "gap": 2 / 32,
    "top": 6 / 32,
    "heights": (19 / 32, 15 / 32, 10 / 32),
}

# 小尺寸下限。低于这些值的笔画在缩放与 DPI 取整下会时隐时现。
MINIMUM = {"margin": 1, "thick": 2, "col_w": 2, "gap": 1, "height": 2}

SOURCE_SIZE = 1024
# Windows 通知区域与多数 Linux 面板按 32 px 取托盘图（高 DPI 下再放大）。
TRAY_SIZE = 32
# **顺序即写入顺序**：32 px 在最前，因为开发工具与任务栏取第一层。
ICO_LAYER_ORDER = (32, 16, 24, 48, 64, 256)


def layout(size: int) -> dict:
    """把 `RELATIVE` 在给定尺寸上取整成一组整数几何。

    最长竖条与下框内沿之间强制留至少 1 px：贴上去会让竖条与框并成一体，
    读成一个实心块。触发时按比例压缩三条高度而不是只截最长那条，否则长短节奏会失真。
    """
    margin = max(MINIMUM["margin"], round(RELATIVE["margin"] * size))
    thick = max(MINIMUM["thick"], round(RELATIVE["thick"] * size))
    col_w = max(MINIMUM["col_w"], round(RELATIVE["col_w"] * size))
    gap = max(MINIMUM["gap"], round(RELATIVE["gap"] * size))
    top = max(margin + thick + 1, round(RELATIVE["top"] * size))
    heights = tuple(max(MINIMUM["height"], round(f * size)) for f in RELATIVE["heights"])

    limit = size - margin - thick - 1 - top
    if max(heights) > limit:
        scale = limit / max(heights)
        heights = tuple(max(MINIMUM["height"], int(h * scale)) for h in heights)

    return {
        "margin": margin,
        "thick": thick,
        "col_w": col_w,
        "gap": gap,
        "top": top,
        "heights": heights,
    }


def render(size: int) -> Image.Image:
    """按目标尺寸原生渲染，全部几何落在整数像素边界上。

    没有超采样、没有降采样：两者都会引入抗锯齿中间色，而上一版正是死在中间色渗色上。
    """
    spec = layout(size)
    margin, thick = spec["margin"], spec["thick"]
    col_w, gap, top = spec["col_w"], spec["gap"], spec["top"]

    img = Image.new("RGBA", (size, size), TRANSPARENT)
    pen = ImageDraw.Draw(img)

    # 印框画成四条独立实心矩形。用 `rectangle(outline=, width=)` 的话，线宽对边界的
    # 取整在不同 Pillow 版本上不一致，会让上下边与左右边差 1 px。
    outer0 = margin
    outer1 = size - margin - 1
    pen.rectangle([outer0, outer0, outer1, outer0 + thick - 1], fill=CINNABAR)
    pen.rectangle([outer0, outer1 - thick + 1, outer1, outer1], fill=CINNABAR)
    pen.rectangle([outer0, outer0, outer0 + thick - 1, outer1], fill=CINNABAR)
    pen.rectangle([outer1 - thick + 1, outer0, outer1, outer1], fill=CINNABAR)

    inner0 = outer0 + thick
    inner1 = outer1 - thick
    span = 3 * col_w + 2 * gap
    pad = (inner1 - inner0 + 1 - span) // 2
    right_edge = inner1 - pad
    for index, height in enumerate(spec["heights"]):
        x1 = right_edge - index * (col_w + gap)
        x0 = x1 - col_w + 1
        pen.rectangle([x0, top, x1, top + height - 1], fill=CINNABAR)

    return img


def _runs(image: Image.Image, row: int) -> list[int]:
    """`row` 这一行上各段连续不透明像素的宽度。"""
    widths: list[int] = []
    current = 0
    for x in range(image.width):
        if image.getpixel((x, row))[3] > 0:
            current += 1
        elif current:
            widths.append(current)
            current = 0
    if current:
        widths.append(current)
    return widths


def assert_uniform(image: Image.Image, size: int) -> None:
    """校验这一档真的达到了设计意图，而不是「看起来生成成功了」。

    四条断言各对应一种已实际发生过的失效：颜色数 > 2 是抗锯齿中间色（上一版 16 px
    的粉雾）；竖条不等宽是按浮点比例算各列边界（20/24 px 实测过）；角落不透明是混进
    了底板；段数不为 5 是竖条与框粘连。
    """
    colors = {image.getpixel((x, y)) for y in range(size) for x in range(size)}
    assert colors <= {TRANSPARENT, CINNABAR}, f"{size} px 出现中间色：{sorted(colors)}"

    corners = [
        image.getpixel(p)
        for p in ((0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1))
    ]
    assert all(px[3] == 0 for px in corners), f"{size} px 四角必须透明：{corners}"

    spec = layout(size)
    row = spec["top"] + 1
    widths = _runs(image, row)
    # 期望 5 段：左框、三道竖条、右框。
    assert len(widths) == 5, f"{size} px 第 {row} 行应为 5 段（左框+三竖条+右框），实得 {widths}"
    assert widths[0] == widths[4] == spec["thick"], f"{size} px 左右框线不等宽：{widths}"
    assert len(set(widths[1:4])) == 1, f"{size} px 三道竖条不等宽：{widths}"


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
