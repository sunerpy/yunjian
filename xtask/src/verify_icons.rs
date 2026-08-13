//! `xtask verify-icons`：图标集验收门禁。
//!
//! # 为什么这个门禁必须存在
//!
//! `cargo tauri icon` 退出 0 并打印一长串 `Creating ...`，这**不是**验收。它可能：
//!
//! - 把 `icons/icon.png` **覆写成 512×512**（本机实测确实如此），于是「源图是
//!   1024×1024」这条要求悄悄不再成立；
//! - 按某个内部顺序写 ICO 的层目录。Windows 开发工具展示的是**第一层**，若第一层
//!   不是 32 px，任务栏里的图标就是错的，而构建、测试、`ls` 全都正常。
//!
//! 因此这里一律**解析字节**下结论，而不是读生成器的成功消息。
//!
//! # 为什么还要写一张六档联系表
//!
//! 字节断言能证明「16 px 那一层存在」，证不了「16 px 下还认得出这是什么」。后者只能
//! 由人眼裁决，而本项目的图标设计已被 16 px 那一档推翻过多轮：朱红圆底加米白月牙在
//! 16 px 下牙尖消失、退化成偏心白斑；云形读成面包；云与月并置读成用户头像。逐条见
//! `crates/yunjian-app/icons/generate_source.py` 的文件头。
//!
//! 六档而非四档：`SHEET_SIZES` 与 `REQUIRED_ICO_SIZES` 一一对应，少一列就等于有一层
//! 从没被人眼看过，而那一层完全可能正是失效的那层。
//!
//! 联系表刻意**渲染 ICO 里真实那几层的字节**并按整数倍最近邻放大，而不是自己把 1024
//! 缩到 16：后者检验的是「我这次缩得糊不糊」，前者检验的是「用户实际看到的那几个
//! 字节糊不糊」。同时铺浅色与深色两块底板，因为托盘背景两种都有。

use std::fmt::Write as _;
use std::fs;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::verify_sources::emit;

/// 图标目录。
const ICONS_DIR: &str = "crates/yunjian-app/icons";
/// 喂给 `cargo tauri icon` 的源图。**不叫 `icon.png`**：见 `generate_source.py` 文件头，
/// `cargo tauri icon` 会把 `icon.png` 覆写成 512×512。
const SOURCE: &str = "source-1024.png";
/// 托盘图标。
const TRAY: &str = "tray.png";
/// Windows 图标容器。
const ICO: &str = "icon.ico";
/// 源图必须的边长。
const SOURCE_SIDE: u32 = 1024;
/// ICO 必须齐备的层尺寸。
const REQUIRED_ICO_SIZES: [u32; 6] = [16, 24, 32, 48, 64, 256];
/// ICO 第一层必须是这个尺寸——开发工具与任务栏取的是第一层。
const ICO_FIRST_SIZE: u32 = 32;
/// 六档联系表输出路径。
const CONTACT_SHEET: &str = "docs/reports/icon-contact-sheet.png";
/// 小尺寸专用表输出路径。见 `render_sheet` 上方那段「为什么要两张表」。
const SMALL_SHEET: &str = "docs/reports/icon-small-sizes.png";
/// 联系表里每格的边长。取 256 是为了让 256 px 那一层能**按原尺寸单独呈现**，
/// 而不是被塞进一个更小的格子里降采样——那样看到的就不是它本来的字节了。
const CELL: u32 = 256;
/// 小尺寸表每格的边长。取 240 是为了让整张表宽 720、高 480，
/// **落在任何看图管道都不会再降采样的量级内**——见 `render_sheet` 的说明。
const SMALL_CELL: u32 = 240;
/// 联系表要展示的尺寸。**六档齐备**，与 `REQUIRED_ICO_SIZES` 一致：
/// 少一档就等于有一档从没被人眼看过，而 16 px 恰恰是最容易失效的那一档。
const SHEET_SIZES: [u32; 6] = [16, 24, 32, 48, 64, 256];
/// 小尺寸表要展示的尺寸：造型被推翻过的四轮，问题全部只出在这三档。
const SMALL_SHEET_SIZES: [u32; 3] = [16, 24, 32];
/// 两块底板。取 Windows 浅色与深色托盘的实际底色。
const BACKDROPS: [[u8; 3]; 2] = [[243, 243, 243], [32, 32, 32]];
/// 深色补偿对照图输出路径。见 `render_compensation_sheet` 上方那段说明。
const COMPENSATION_SHEET: &str = "docs/reports/icon-dark-compensation.png";
/// 深色补偿对照图每格的边长，取值理由同 `SMALL_CELL`。
const COMPENSATION_CELL: u32 = 240;
/// 深色补偿对照图用哪一层做样本。颜色对比与尺寸无关，取最大的清晰层即可。
const COMPENSATION_SAMPLE_SIZE: u32 = 32;
/// 朱砂。取值来自「浅深两侧对比相等」的解，推导在 `generate_source.py` 的深色补偿一节。
const SEAL: [u8; 4] = [219, 70, 52, 255];
/// 米白刻痕。
const INK: [u8; 4] = [250, 246, 238, 255];
/// 上一版（v4）的配色。**只用于深色补偿对照图的参照列**，不参与任何产物断言。
const LEGACY_SEAL: [u8; 4] = [216, 69, 47, 255];
const LEGACY_INK: [u8; 4] = [245, 239, 225, 255];
/// 小尺寸层允许的最大颜色数（含全透明）。
///
/// 这条断言守的是一个**已实际发生过**的失效：`cargo tauri icon` 生成 ICO 的方式是把
/// 1024 源图**降采样**，于是手工按整数像素网格画出来的清晰小层根本进不了 ICO。
/// 实测对照——降采样的 32 px 有 **30** 种颜色（大量抗锯齿中间色，在 16 px 上表现为
/// 高对比边缘渗出的一圈杂色雾），原生渲染的同一层只有 **2** 种。
/// 因此层数与层序都正确、构建也全绿时，这条是唯一能发现「忘了用原生层覆盖 ICO」的检查。
const MAX_SMALL_LAYER_COLORS: usize = 4;
/// 受上面那条颜色数约束的层。48 px 以上不约束：那些尺寸本来就有余量表达更多层次。
const CRISP_SIZES: [u32; 3] = [16, 24, 32];

/// 一层图标的三种像素。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cell {
    Clear,
    Seal,
    Ink,
}

/// 从**已写进 ICO 的字节**反推出来的骨架几何。
///
/// 全部字段单位是该层的像素。`generate_source.py` 在渲染前也断言同一组量，
/// 但那只能证明「生成器自己自洽」；v4 的六档里有两档笔画变细、印面偏小，恰恰是
/// 生成之后、写进 ICO 的路上丢的。这个结构存在的唯一理由就是把那一段补上。
#[derive(Debug, PartialEq, Eq)]
struct Geometry {
    margin: u32,
    mark: u32,
    border: u32,
    stroke_w: u32,
    gap: u32,
    stroke_h: u32,
}

/// 笔宽的期望值：`floor(2 × 该层缩放 + 0.5)`，缩放 = `size / 16`。
///
/// 用整数算而不是浮点四舍五入：Rust 的 `f64::round` 与 Python 内建 `round` 在 `.5`
/// 上行为不同（后者是银行家舍入），而 20 px 档的目标值恰好是 2.5，两侧算法必须给出
/// 同一个 3，否则这条断言在唯一需要它的档位上与生成器不一致。
fn expected_stroke_width(size: u32) -> u32 {
    (2 * size + 8) / 16
}

/// 各段连续 `true` 的 (起始下标, 长度)。
fn runs(values: &[bool]) -> Vec<(u32, u32)> {
    let mut spans = Vec::new();
    let mut start: Option<u32> = None;
    for (index, on) in values.iter().enumerate() {
        let index = index as u32;
        match (on, start) {
            (true, None) => start = Some(index),
            (false, Some(from)) => {
                spans.push((from, index - from));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        spans.push((from, values.len() as u32 - from));
    }
    spans
}

/// 把一层位图判成 `Cell` 网格。出现第三种实色即失败——那说明这层是降采样产物。
fn classify(image: &Rgba) -> Result<Vec<Vec<Cell>>> {
    let mut grid = Vec::with_capacity(image.height as usize);
    for y in 0..image.height {
        let mut row = Vec::with_capacity(image.width as usize);
        for x in 0..image.width {
            let px = image.pixel(x, y);
            row.push(match px {
                _ if px[3] == 0 => Cell::Clear,
                p if p == SEAL => Cell::Seal,
                p if p == INK => Cell::Ink,
                other => bail!(
                    "{}×{} 层在 ({x},{y}) 出现既非朱砂 {SEAL:?} 也非米白 {INK:?} 的实色 {other:?}",
                    image.width,
                    image.height
                ),
            });
        }
        grid.push(row);
    }
    Ok(grid)
}

/// 从 `Cell` 网格反推骨架几何。
///
/// 「笔行」的判据是**该行恰有两段米白**：印面行有零段，所以这个判据不需要知道
/// 任何坐标常量，改了设计也不会悄悄量到别的东西上。
fn measure(grid: &[Vec<Cell>]) -> Result<Geometry> {
    let size = grid.len() as u32;
    let occupied = |x: u32, y: u32| grid[y as usize][x as usize] != Cell::Clear;
    let xs: Vec<u32> = (0..size)
        .filter(|&x| (0..size).any(|y| occupied(x, y)))
        .collect();
    let ys: Vec<u32> = (0..size)
        .filter(|&y| (0..size).any(|x| occupied(x, y)))
        .collect();
    let (&x0, &x1) = (
        xs.first().context("整层全透明")?,
        xs.last().context("整层全透明")?,
    );
    let (&y0, &y1) = (
        ys.first().context("整层全透明")?,
        ys.last().context("整层全透明")?,
    );

    let ink_row = |y: u32| -> Vec<bool> {
        (0..size)
            .map(|x| grid[y as usize][x as usize] == Cell::Ink)
            .collect()
    };
    let stroke_rows: Vec<u32> = (0..size)
        .filter(|&y| runs(&ink_row(y)).len() == 2)
        .collect();
    let (&first, &last) = (
        stroke_rows
            .first()
            .context("找不到任何「恰有两段米白」的行——刻痕不是两道竖笔")?,
        stroke_rows.last().context("找不到笔行")?,
    );
    if last - first + 1 != stroke_rows.len() as u32 {
        bail!("笔行不连续：{stroke_rows:?}");
    }
    let spans = runs(&ink_row(first));
    let (left_x, left_w) = spans[0];
    let (right_x, right_w) = spans[1];
    if left_w != right_w {
        bail!("左笔宽 {left_w} != 右笔宽 {right_w}，两笔必须等宽");
    }
    if x1 - (right_x + right_w - 1) != left_x - x0 {
        bail!(
            "两笔在印身内左右不等距：左 {} 右 {}",
            left_x - x0,
            x1 - (right_x + right_w - 1)
        );
    }
    if first - y0 != y1 - last {
        bail!("两笔在印身内上下不等距：上 {} 下 {}", first - y0, y1 - last);
    }
    if x1 - x0 != y1 - y0 {
        bail!("印身不是正方形：{}×{}", x1 - x0 + 1, y1 - y0 + 1);
    }
    Ok(Geometry {
        margin: x0,
        mark: x1 - x0 + 1,
        border: left_x - x0,
        stroke_w: left_w,
        gap: right_x - (left_x + left_w),
        stroke_h: stroke_rows.len() as u32,
    })
}

/// 逐层核对骨架：笔宽逐档取整、两笔等宽等高、四边净距相等、全网格双向镜像。
///
/// 这一组断言的存在理由是 v4 的一次真实失效：**六档里有两档的中竖笔明显变细、
/// 印面偏小**，而当时的门禁只查层数、层序、四角 alpha 与颜色数，四项全绿。
/// 换句话说，那次失效发生在字节里，却没有任何字节断言看着它。
fn check_geometry(failures: &mut Failures, size: u32, image: &Rgba) {
    let grid = match classify(image).and_then(|grid| measure(&grid).map(|got| (grid, got))) {
        Ok(pair) => pair,
        Err(err) => {
            failures.check(false, ICO, || format!("{size} px 层无法量出骨架：{err:#}"));
            return;
        }
    };
    let (grid, got) = grid;
    let what = ICO;

    let want_stroke = expected_stroke_width(size);
    failures.check(got.stroke_w == want_stroke, what, || {
        format!(
            "{size} px 笔宽 {} != floor(2 × {}/16 + 0.5) = {want_stroke}——该档没有按自己的\
             网格取整，而是浮点缩放来的（v4 就是这样丢了两档笔宽）",
            got.stroke_w, size
        )
    });
    failures.check(got.stroke_h == 2 * got.stroke_w + got.gap, what, || {
        format!(
            "{size} px 笔高 {} != 2×{} + {} ，两笔加缝的外接框不是正方形",
            got.stroke_h, got.stroke_w, got.gap
        )
    });
    failures.check(got.border >= 2 && got.gap >= 2, what, || {
        format!(
            "{size} px 净间距或缝宽小于 2 px：朱边 {} 缝 {}",
            got.border, got.gap
        )
    });
    let seal_share = f64::from(got.mark) / f64::from(size);
    failures.check((0.83..=0.90).contains(&seal_share), what, || {
        format!(
            "{size} px 印身占整格 {:.1}%（印身 {}），应落在 83%~90%，否则各档取景不是一套",
            seal_share * 100.0,
            got.mark
        )
    });

    let mirrored_x = (0..size).all(|y| {
        (0..size).all(|x| grid[y as usize][x as usize] == grid[y as usize][(size - 1 - x) as usize])
    });
    let mirrored_y = (0..size).all(|y| grid[y as usize] == grid[(size - 1 - y) as usize]);
    failures.check(mirrored_x && mirrored_y, what, || {
        format!("{size} px 层不是双向镜像对称：左右={mirrored_x} 上下={mirrored_y}")
    });

    emit(&format!(
        "  骨架     {size:>3} px  气口={:<3} 朱边={:<3} 笔宽={:<3}(期望 {want_stroke}) \
         缝宽={:<3} 笔高={:<4} 印身={}({:.1}%)  双向镜像={}",
        got.margin,
        got.border,
        got.stroke_w,
        got.gap,
        got.stroke_h,
        got.mark,
        seal_share * 100.0,
        mirrored_x && mirrored_y
    ));
}

/// 一张解码后的 RGBA8 位图。
struct Rgba {
    width: u32,
    height: u32,
    /// 长度为 `width * height * 4`。
    pixels: Vec<u8>,
}

impl Rgba {
    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ]
    }

    /// 四个角的像素。透明性判定只看角：中心当然应该有颜色。
    fn corners(&self) -> [[u8; 4]; 4] {
        let (w, h) = (self.width - 1, self.height - 1);
        [
            self.pixel(0, 0),
            self.pixel(w, 0),
            self.pixel(0, h),
            self.pixel(w, h),
        ]
    }
}

/// 解码一张 PNG 成 RGBA8。
///
/// `png` crate 的 `Transformations::ALPHA | EXPAND` 把灰度、调色板与无 alpha 的输入
/// 统一抬成 RGBA8，于是下面的角落 alpha 判定对任何合法 PNG 都成立——**包括**
/// 被压平成不透明 RGB 的那种，那正是失败场景要抓的形态。
fn decode_png(bytes: &[u8], what: &str) -> Result<Rgba> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::ALPHA | png::Transformations::EXPAND);
    let mut reader = decoder
        .read_info()
        .with_context(|| format!("{what} 不是可解析的 PNG"))?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader
        .next_frame(&mut buf)
        .with_context(|| format!("{what} 解码失败"))?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        bail!(
            "{what} 归一化后仍不是 RGBA8：color_type={:?} bit_depth={:?}",
            info.color_type,
            info.bit_depth
        );
    }
    buf.truncate((info.width * info.height * 4) as usize);
    Ok(Rgba {
        width: info.width,
        height: info.height,
        pixels: buf,
    })
}

/// 一张位图里出现过的全部 RGBA 取值。
///
/// 用来区分「该尺寸的原生渲染」与「1024 源图的降采样」：前者所有几何都落在整数像素
/// 边界上，颜色数等于用到的实色数；后者每条边缘都会混出一串中间色。实测差距是 2 对 30。
fn distinct_colors(image: &Rgba) -> std::collections::BTreeSet<[u8; 4]> {
    let mut seen = std::collections::BTreeSet::new();
    for y in 0..image.height {
        for x in 0..image.width {
            seen.insert(image.pixel(x, y));
        }
    }
    seen
}

/// ICO 目录里的一条。
#[derive(Debug)]
struct IcoEntry {
    size: u32,
    bits_per_pixel: u16,
    offset: usize,
    length: usize,
}

/// 手工解析 ICO 的目录结构。
///
/// 刻意**不**用图像库的 `sizes()` 之类接口：那类接口返回的是尺寸**集合**，而这里要断言的
/// 恰恰是**顺序**（32 px 必须在最前），集合把顺序信息丢掉了，断言就永远变不了红。
///
/// 格式：6 字节头（reserved=0、type=1、count），随后每层一条 16 字节目录项，
/// 其中宽/高字段为 1 字节，取值 `0` 表示 256。
fn parse_ico(bytes: &[u8]) -> Result<Vec<IcoEntry>> {
    if bytes.len() < 6 {
        bail!("ICO 太短，连 6 字节头都不完整");
    }
    let reserved = u16::from_le_bytes([bytes[0], bytes[1]]);
    let kind = u16::from_le_bytes([bytes[2], bytes[3]]);
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    if reserved != 0 || kind != 1 {
        bail!("ICO 头不合法：reserved={reserved} type={kind}（应为 0 与 1）");
    }
    if count == 0 {
        bail!("ICO 目录为空");
    }
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let at = 6 + i * 16;
        let raw = bytes
            .get(at..at + 16)
            .with_context(|| format!("ICO 第 {i} 条目录项越界"))?;
        let width = u32::from(raw[0]);
        let height = u32::from(raw[1]);
        // `0` 编码 256：单字节装不下 256。
        let width = if width == 0 { 256 } else { width };
        let height = if height == 0 { 256 } else { height };
        if width != height {
            bail!("ICO 第 {i} 层不是正方形：{width}x{height}");
        }
        let bits_per_pixel = u16::from_le_bytes([raw[6], raw[7]]);
        let length = u32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]) as usize;
        let offset = u32::from_le_bytes([raw[12], raw[13], raw[14], raw[15]]) as usize;
        entries.push(IcoEntry {
            size: width,
            bits_per_pixel,
            offset,
            length,
        });
    }
    Ok(entries)
}

/// 失败累积器。一次跑完报出全部问题，而不是第一条就中止——修图标是要看全貌的。
#[derive(Default)]
struct Failures(Vec<String>);

impl Failures {
    fn check(&mut self, ok: bool, what: &str, why: impl FnOnce() -> String) {
        if !ok {
            self.0.push(format!("{what}: {}", why()));
        }
    }

    fn into_result(self) -> Result<()> {
        if self.0.is_empty() {
            return Ok(());
        }
        let mut msg = format!("图标验收失败，共 {} 项：\n", self.0.len());
        for f in &self.0 {
            let _ = writeln!(msg, "  FAIL  {f}");
        }
        bail!(msg)
    }
}

/// 仓库根。`CARGO_MANIFEST_DIR` 是 `xtask/`，向上一级即根。
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask 必须有父目录")
        .to_path_buf()
}

fn read(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).with_context(|| format!("读取 {} 失败", path.display()))
}

/// 把 `src` 按**整数倍**最近邻放大铺进 `dst` 的 (`ox`, `oy`) 处，底色为 `bg`。
///
/// 最近邻而非插值是刻意的：放大只为让人眼看清**那几个字节本来的样子**，插值会替
/// 生成器把糊掉的地方抹匀，正好毁掉这张表的用处。
///
/// 倍率必须是整数，否则源像素会被放成大小不一的方块（例如 24 px 铺进 128 px 的格子
/// 是 5.33 倍），人眼看到的粗细差异就分不清是图标本身的还是放大引入的。
/// 因此这里按 `CELL / src.width` 取整倍并**居中**，剩余边缘留底色。
fn blit_magnified(dst: &mut Rgba, src: &Rgba, ox: u32, oy: u32, side: u32, bg: [u8; 3]) {
    let factor = (side / src.width).max(1);
    let drawn = src.width * factor;
    let pad = (side - drawn) / 2;
    for dy in 0..side {
        for dx in 0..side {
            let inside = dx >= pad && dx < pad + drawn && dy >= pad && dy < pad + drawn;
            let [r, g, b, a] = if inside {
                let sx = (dx - pad) / factor;
                let sy = (dy - pad) / factor;
                src.pixel(sx.min(src.width - 1), sy.min(src.height - 1))
            } else {
                [0, 0, 0, 0]
            };
            // 源像素按 alpha 合成到底板上，得到「在这种背景的托盘上实际长什么样」。
            let mix = |fg: u8, back: u8| -> u8 {
                let fg = u32::from(fg) * u32::from(a);
                let back = u32::from(back) * (255 - u32::from(a));
                ((fg + back) / 255) as u8
            };
            let i = (((oy + dy) * dst.width + ox + dx) * 4) as usize;
            dst.pixels[i] = mix(r, bg[0]);
            dst.pixels[i + 1] = mix(g, bg[1]);
            dst.pixels[i + 2] = mix(b, bg[2]);
            dst.pixels[i + 3] = 255;
        }
    }
}

fn write_png(path: &Path, image: &Rgba) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("创建 {} 失败", parent.display()))?;
    }
    let file = fs::File::create(path).with_context(|| format!("创建 {} 失败", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .with_context(|| format!("写 {} 的 PNG 头失败", path.display()))?;
    writer
        .write_image_data(&image.pixels)
        .with_context(|| format!("写 {} 的像素失败", path.display()))?;
    Ok(())
}

/// 渲染联系表：一行浅色底板、一行深色底板，列为 `sizes`。
///
/// 每格的像素来自 **ICO 里同尺寸的那一层**，即 Windows 真正会取的那份字节。
///
/// # 为什么每一行是一整片连续底色，而不是带间隙的格子
///
/// 上一版在格与格之间铺了中灰间隙。整数倍率放大后各档的绘制区大小不一
/// （16 px 铺进 256 的格子是 16 倍恰好填满，24 px 只能取 10 倍得到 240，两侧各留 8 px），
/// 于是**有的格子四周有一圈底色、有的格子直接顶到中灰**——同一行里各格呈现的底色
/// 面积因此不同，人眼读成「各格明度不一致」，而字节层其实完全相同。
/// 这是一次真实的误判来源：评审据此判定「对比图生成流程有 bug，无法公平比较」。
/// 去掉间隙后，一行就是一整片均匀底色，每个图标的周围环境完全相同。
/// `probe_backdrops` 逐格取样复核这一点，把「底色一致」变成可判定的断言而不是承诺。
///
/// # 为什么要两张表
///
/// 六档表为了让 256 px 那层按原尺寸呈现，格边长必须是 256，整张表宽约 1.5 K。
/// 看图管道一旦把它整体缩到 1 K 以内，缩放本身会在红白边界混出粉雾，
/// 而**造型被推翻的四轮，问题全部只出在 16 / 24 / 32 这三档**。
/// 所以另出一张只含这三档、尺寸 720×480 的小表：它小到不会被任何管道再缩一次，
/// 看到的就是字节本身。
/// 把一层的配色替换成上一版（v4）的，用来做深色补偿的参照列。
fn retint_to_legacy(image: &Rgba) -> Rgba {
    let mut pixels = image.pixels.clone();
    for px in pixels.chunks_exact_mut(4) {
        let current: [u8; 4] = [px[0], px[1], px[2], px[3]];
        if current == SEAL {
            px.copy_from_slice(&LEGACY_SEAL);
        } else if current == INK {
            px.copy_from_slice(&LEGACY_INK);
        }
    }
    Rgba {
        width: image.width,
        height: image.height,
        pixels,
    }
}

/// 渲染深色补偿对照图：左列本版配色，右列上一版配色，上行浅底、下行深底。
///
/// # 为什么需要单独一张
///
/// 六档表与小尺寸表都只呈现**当前**这一版，看不出补偿到底改了什么。而本版的深色
/// 补偿是把朱砂移到「浅深两侧对比相等」的那个点（3.83:1 / 3.83:1），取代 v4 的
/// 3.93:1 / 3.73:1——深色侧净增只有 **2.7%**，因为 3.83:1 是任何单色填充在这两种
/// 托盘底色之间能取到的**最大下界**（推导在 `generate_source.py` 的深色补偿一节）。
///
/// 这张表的用处正是让这个「弱」可被目视核验：如果并置之后仍看不出差别，那是算术
/// 上限的结论，不是补偿没做。声称做了补偿却不给对照，与没做无法区分。
fn render_compensation_sheet(sample: &Rgba) -> Rgba {
    let legacy = retint_to_legacy(sample);
    let cell = COMPENSATION_CELL;
    let width = 2 * cell;
    let height = BACKDROPS.len() as u32 * cell;
    let mut sheet = Rgba {
        width,
        height,
        pixels: vec![0; (width * height * 4) as usize],
    };
    for (ri, bg) in BACKDROPS.iter().enumerate() {
        let band = ri as u32 * cell;
        for y in band..band + cell {
            for x in 0..width {
                let i = ((y * width + x) * 4) as usize;
                sheet.pixels[i..i + 4].copy_from_slice(&[bg[0], bg[1], bg[2], 255]);
            }
        }
        for (ci, variant) in [sample, &legacy].into_iter().enumerate() {
            blit_magnified(&mut sheet, variant, ci as u32 * cell, band, cell, *bg);
        }
    }
    sheet
}

fn render_sheet(layers: &[(u32, Rgba)], sizes: &[u32], cell: u32) -> Rgba {
    let width = sizes.len() as u32 * cell;
    let height = BACKDROPS.len() as u32 * cell;
    let mut sheet = Rgba {
        width,
        height,
        pixels: vec![0; (width * height * 4) as usize],
    };
    for (ri, bg) in BACKDROPS.iter().enumerate() {
        let band = ri as u32 * cell;
        for y in band..band + cell {
            for x in 0..width {
                let i = ((y * width + x) * 4) as usize;
                sheet.pixels[i..i + 4].copy_from_slice(&[bg[0], bg[1], bg[2], 255]);
            }
        }
        for (ci, size) in sizes.iter().enumerate() {
            let Some((_, layer)) = layers.iter().find(|(s, _)| s == size) else {
                continue;
            };
            blit_magnified(&mut sheet, layer, ci as u32 * cell, band, cell, *bg);
        }
    }
    sheet
}

/// 逐格取样每格左上角的像素，用来复核同一行各格的基准底色**逐字节相同**。
///
/// 取左上角是有依据的：图标四角必须透明（另有断言守着），所以该位置合成出来的
/// 一定是底板色本身。凡取样值与该行底板不等，就说明表里出现了第二种底色——
/// 那正是让「各档发色」无法公平比较的那个缺陷。
fn probe_backdrops(sheet: &Rgba, sizes: &[u32], cell: u32) -> Vec<(usize, u32, [u8; 4])> {
    let mut odd = Vec::new();
    for (ri, bg) in BACKDROPS.iter().enumerate() {
        let expected = [bg[0], bg[1], bg[2], 255];
        for (ci, size) in sizes.iter().enumerate() {
            let sample = sheet.pixel(ci as u32 * cell, ri as u32 * cell);
            if sample != expected {
                odd.push((ri, *size, sample));
            }
        }
    }
    odd
}

pub(crate) fn run() -> Result<()> {
    let root = repo_root();
    let icons = root.join(ICONS_DIR);
    let mut failures = Failures::default();

    emit("== 图标验收（逐字节，不采信生成器的成功消息）==");

    // --- 1. 源图规格 ---
    let source_path = icons.join(SOURCE);
    let source = decode_png(&read(&source_path)?, SOURCE)?;
    failures.check(
        source.width == SOURCE_SIDE && source.height == SOURCE_SIDE,
        SOURCE,
        || {
            format!(
                "源图必须 {SOURCE_SIDE}×{SOURCE_SIDE}，实际 {}×{}",
                source.width, source.height
            )
        },
    );
    let source_corners = source.corners();
    let opaque_corners: Vec<usize> = source_corners
        .iter()
        .enumerate()
        .filter(|(_, px)| px[3] != 0)
        .map(|(i, _)| i)
        .collect();
    failures.check(opaque_corners.is_empty(), SOURCE, || {
        format!(
            "源图必须是透明背景（不透明方块会产出带底色方块的托盘图标），\
             但第 {opaque_corners:?} 个角不透明：{source_corners:?}"
        )
    });
    emit(&format!(
        "  源图     {SOURCE}  {}×{}  四角 alpha {:?}",
        source.width,
        source.height,
        source_corners.map(|px| px[3])
    ));

    // --- 2. ICO 层与层序 ---
    let ico_bytes = read(&icons.join(ICO))?;
    let entries = parse_ico(&ico_bytes)?;
    let sizes: Vec<u32> = entries.iter().map(|e| e.size).collect();
    for required in REQUIRED_ICO_SIZES {
        failures.check(sizes.contains(&required), ICO, || {
            format!("缺少 {required} px 层，实际层序 {sizes:?}")
        });
    }
    failures.check(sizes.first() == Some(&ICO_FIRST_SIZE), ICO, || {
        format!("第一层必须是 {ICO_FIRST_SIZE} px（开发工具与任务栏取第一层），实际层序 {sizes:?}")
    });
    let mut layers = Vec::new();
    for entry in &entries {
        let slice = ico_bytes
            .get(entry.offset..entry.offset + entry.length)
            .with_context(|| format!("ICO {} px 层的数据范围越界", entry.size))?;
        // Tauri 写的是 PNG 内嵌层；BMP 内嵌层这里不解码，只在联系表里跳过。
        if slice.starts_with(b"\x89PNG\r\n\x1a\n") {
            let layer = decode_png(slice, &format!("{ICO} 的 {} px 层", entry.size))?;
            failures.check(
                layer.width == entry.size && layer.height == entry.size,
                ICO,
                || {
                    format!(
                        "{} px 目录项与内嵌 PNG 实际尺寸不符：{}×{}",
                        entry.size, layer.width, layer.height
                    )
                },
            );
            if CRISP_SIZES.contains(&entry.size) {
                let colors = distinct_colors(&layer);
                failures.check(colors.len() <= MAX_SMALL_LAYER_COLORS, ICO, || {
                    format!(
                        "{} px 层有 {} 种颜色（上限 {MAX_SMALL_LAYER_COLORS}），\
                         说明它是 1024 源图的降采样而不是该尺寸的原生渲染——\
                         `cargo tauri icon` 之后漏跑了 `generate_source.py --ico-only`。\
                         那些抗锯齿中间色在 16 px 上就是高对比边缘的一圈杂色雾。",
                        entry.size,
                        colors.len()
                    )
                });
            }
            check_geometry(&mut failures, entry.size, &layer);
            layers.push((entry.size, layer));
        }
        emit(&format!(
            "  ICO 层   {:>3} px  {} bpp  {} 字节",
            entry.size, entry.bits_per_pixel, entry.length
        ));
    }
    emit(&format!(
        "  ICO 层序 {sizes:?}  32 px 在最前 = {}",
        sizes.first() == Some(&ICO_FIRST_SIZE)
    ));

    // --- 3. 托盘图标透明背景 ---
    let tray = decode_png(&read(&icons.join(TRAY))?, TRAY)?;
    let tray_corners = tray.corners();
    let tray_opaque: Vec<usize> = tray_corners
        .iter()
        .enumerate()
        .filter(|(_, px)| px[3] != 0)
        .map(|(i, _)| i)
        .collect();
    failures.check(tray_opaque.is_empty(), TRAY, || {
        format!(
            "托盘图标必须真的透明、且没有自绘圆角底板（系统会把它直接合成到自己的\
             背景上，底板会读成一块硬边色块），但第 {tray_opaque:?} 个角不透明：{tray_corners:?}"
        )
    });
    emit(&format!(
        "  托盘     {TRAY}  {}×{}  四角 alpha {:?}",
        tray.width,
        tray.height,
        tray_corners.map(|px| px[3])
    ));

    // --- 4. 写联系表 ---
    // 无论断言结果如何都写：图标不合格时，最需要的恰恰是那张能看出哪儿不对的表。
    for (path, sizes, cell) in [
        (CONTACT_SHEET, SHEET_SIZES.as_slice(), CELL),
        (SMALL_SHEET, SMALL_SHEET_SIZES.as_slice(), SMALL_CELL),
    ] {
        let sheet = render_sheet(&layers, sizes, cell);
        write_png(&root.join(path), &sheet)?;
        let odd = probe_backdrops(&sheet, sizes, cell);
        failures.check(odd.is_empty(), path, || {
            format!(
                "同一行各格的基准底色必须逐字节相同，否则各档发色无法公平比较，\
                 但这些格取样到了别的颜色（行序号, 尺寸, 实测像素）：{odd:?}"
            )
        });
        emit(&format!(
            "  联系表   {path}  {}×{}  列 {sizes:?}  行 [浅色 #F3F3F3, 深色 #202020]  \
             逐格底色一致 = {}",
            sheet.width,
            sheet.height,
            odd.is_empty()
        ));
    }
    if let Some((_, sample)) = layers
        .iter()
        .find(|(size, _)| *size == COMPENSATION_SAMPLE_SIZE)
    {
        let sheet = render_compensation_sheet(sample);
        write_png(&root.join(COMPENSATION_SHEET), &sheet)?;
        emit(&format!(
            "  补偿表   {COMPENSATION_SHEET}  {}×{}  列 [本版 #DB4634, 上一版 #D8452F]  \
             行 [浅色, 深色]  样本 {COMPENSATION_SAMPLE_SIZE} px",
            sheet.width, sheet.height
        ));
    } else {
        failures.check(false, ICO, || {
            format!("缺少 {COMPENSATION_SAMPLE_SIZE} px 层，深色补偿对照图无法生成")
        });
    }
    emit("  ↑ 这几张表必须由人亲眼看：字节断言证不了「16 px 下还认得出是什么」。");

    failures.into_result()?;
    emit("== 全部通过 ==");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 逐字节解析一份手工拼的最小 ICO，确认目录项的宽高、`0`→256 编码与顺序都读对了。
    ///
    /// 用合成字节而不是仓库里的真文件：这条测的是**解析器**，若拿真文件测，
    /// 解析器写错了同时真文件也恰好不合格时，两个错误可能互相掩盖。
    #[test]
    fn parses_ico_directory_preserving_order_and_256_encoding() {
        let mut bytes = vec![0u8, 0, 1, 0, 2, 0]; // reserved=0 type=1 count=2
        // 第一条：32 px。
        bytes.extend_from_slice(&[32, 32, 0, 0, 1, 0, 32, 0]);
        bytes.extend_from_slice(&4u32.to_le_bytes()); // length
        bytes.extend_from_slice(&38u32.to_le_bytes()); // offset
        // 第二条：宽高字段为 0，表示 256 px。
        bytes.extend_from_slice(&[0, 0, 0, 0, 1, 0, 32, 0]);
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&42u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);

        let entries = parse_ico(&bytes).expect("合成 ICO 应可解析");
        let sizes: Vec<u32> = entries.iter().map(|e| e.size).collect();
        assert_eq!(sizes, vec![32, 256], "顺序与 0→256 编码都必须保留");
        assert_eq!(entries[0].bits_per_pixel, 32);
        assert_eq!(entries[0].offset, 38);
        assert_eq!(entries[0].length, 4);
    }

    #[test]
    fn rejects_ico_with_bad_header() {
        let bytes = vec![0u8, 0, 2, 0, 1, 0]; // type=2 是 CUR，不是 ICO
        let err = parse_ico(&bytes).expect_err("type=2 应被拒");
        assert!(err.to_string().contains("type=2"), "{err}");
    }

    /// 解析器必须把**顺序**暴露出来，否则「32 px 在最前」这条断言永远变不了红。
    ///
    /// 这条正是不用图像库 `sizes()` 接口的理由：那类接口返回集合，
    /// 而下面两份字节的集合完全相同、顺序相反。
    #[test]
    fn layer_order_is_observable_not_just_the_set() {
        let entry = |size: u8| {
            let mut e = vec![size, size, 0, 0, 1, 0, 32, 0];
            e.extend_from_slice(&0u32.to_le_bytes());
            e.extend_from_slice(&38u32.to_le_bytes());
            e
        };
        let build = |a: u8, b: u8| {
            let mut bytes = vec![0u8, 0, 1, 0, 2, 0];
            bytes.extend(entry(a));
            bytes.extend(entry(b));
            bytes
        };
        let first = parse_ico(&build(32, 16)).expect("可解析");
        let second = parse_ico(&build(16, 32)).expect("可解析");
        assert_eq!(first[0].size, 32);
        assert_eq!(second[0].size, 16);
    }

    /// 无 alpha 通道的 PNG 必须被抬成 RGBA8，且角落 alpha 读出来是 255（不透明）——
    /// 这正是「把源图压平到白底」时应当让透明度断言变红的那条路径。
    #[test]
    fn decodes_rgb_png_as_opaque_rgba_so_flattened_sources_are_caught() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("写头");
            writer
                .write_image_data(&[255; 12])
                .expect("写 2×2 RGB 像素");
        }
        let image = decode_png(&bytes, "合成 RGB PNG").expect("应可解码");
        assert_eq!(image.width, 2);
        assert!(
            image.corners().iter().all(|px| px[3] == 255),
            "无 alpha 的输入抬成 RGBA 后角落必须是不透明：{:?}",
            image.corners()
        );
    }

    /// 一层 4×4、四角透明、中央 2×2 不透明红的假层，用于联系表相关的几条测试。
    ///
    /// **四角必须透明**，因为真实图标就是这样（另有断言守着），而 `probe_backdrops`
    /// 正是依赖这个不变量在格子左上角取到纯底板色。夹具违反不变量会让那条测试假红。
    fn stub_layer() -> Rgba {
        let clear = [0u8, 0, 0, 0];
        let red = [255u8, 0, 0, 255];
        let rows = [
            [clear, clear, clear, clear],
            [clear, red, red, clear],
            [clear, red, red, clear],
            [clear, clear, clear, clear],
        ];
        Rgba {
            width: 4,
            height: 4,
            pixels: rows.iter().flatten().flatten().copied().collect(),
        }
    }

    /// 联系表必须真的把每格画出来：放大后的格子不能全是底板色，
    /// 否则一张「看起来生成成功」的空白表会让人眼验收退化成形式。
    #[test]
    fn contact_sheet_cells_contain_icon_pixels_not_just_backdrop() {
        let sheet = render_sheet(&[(16, stub_layer())], &[16], CELL);
        // 4×4 的源铺进 256 的格子是 64 倍，于是中央 2×2 落在 [64, 192) 区间。
        assert_eq!(sheet.pixel(CELL / 2, CELL / 2), [255, 0, 0, 255]);
        // 角落是透明源像素，应当合成出浅色底板。
        assert_eq!(sheet.pixel(0, 0), [243, 243, 243, 255]);
        // 第二行同一位置的底板是深色。
        assert_eq!(sheet.pixel(0, CELL), [32, 32, 32, 255]);
    }

    /// 同一行各格的基准底色必须逐字节相同，否则各档发色无法公平比较。
    ///
    /// 这条守的是一次**真实误判**：上一版在格间铺中灰，而整数倍率让各档绘制区
    /// 大小不一，于是有的格子四周多一圈底色、有的直接顶到中灰，人眼读成
    /// 「各格明度不一致」并据此判定生成流程有 bug。取样器必须能抓住这种不一致，
    /// 所以这里先证它对合格的表不误报，再手工污染一格证它会变红。
    #[test]
    fn backdrop_probe_catches_a_single_polluted_cell() {
        let layers = [(16, stub_layer()), (24, stub_layer())];
        let mut sheet = render_sheet(&layers, &[16, 24], CELL);
        assert!(
            probe_backdrops(&sheet, &[16, 24], CELL).is_empty(),
            "连续底色的表不该被判为不一致"
        );

        // 把第二格左上角改成差 4 级的近似底色：肉眼几乎看不出，取样器必须看出来。
        let i = ((CELL) * 4) as usize;
        sheet.pixels[i..i + 4].copy_from_slice(&[239, 239, 239, 255]);
        let odd = probe_backdrops(&sheet, &[16, 24], CELL);
        assert_eq!(odd.len(), 1, "只污染了一格：{odd:?}");
        assert_eq!(odd[0].1, 24, "被污染的是 24 px 那一列：{odd:?}");
    }

    /// 小尺寸表必须小到不会被看图管道再缩一次，否则缩放会在红白边界混出粉雾，
    /// 而人眼验收看到的就不再是字节本身。720×480 是这条的具体形态。
    #[test]
    fn small_sheet_stays_within_native_viewing_budget() {
        let layers: Vec<(u32, Rgba)> = SMALL_SHEET_SIZES
            .iter()
            .map(|s| (*s, stub_layer()))
            .collect();
        let sheet = render_sheet(&layers, SMALL_SHEET_SIZES.as_slice(), SMALL_CELL);
        assert_eq!((sheet.width, sheet.height), (720, 480));
        assert!(
            SMALL_SHEET_SIZES.iter().all(|s| SHEET_SIZES.contains(s)),
            "小表的每一档都必须也是六档表里的一档，否则两张表在验收同一件事时会不一致"
        );
    }

    /// 放大倍率必须是整数并居中，否则源像素会被放成大小不一的方块，
    /// 人眼就分不清粗细差异是图标本身的还是放大引入的。
    ///
    /// 用 3×3 源铺进边长 8 的格子：整数倍率是 2，画出 6×6 并左右各留 1 px 底色。
    /// 若实现改回 `dx * src.width / side` 的非整数映射，中心那 6×6 的分块会不均，
    /// 下面对边缘留白的断言会立刻变红。
    #[test]
    fn magnification_factor_is_integral_and_centred() {
        let src = Rgba {
            width: 3,
            height: 3,
            pixels: [255, 0, 0, 255].repeat(9),
        };
        let mut dst = Rgba {
            width: 8,
            height: 8,
            pixels: vec![0; 8 * 8 * 4],
        };
        blit_magnified(&mut dst, &src, 0, 0, 8, [243, 243, 243]);
        assert_eq!(dst.pixel(0, 0), [243, 243, 243, 255], "左上应是留白");
        assert_eq!(dst.pixel(1, 1), [255, 0, 0, 255], "6×6 图像区左上角");
        assert_eq!(dst.pixel(6, 6), [255, 0, 0, 255], "6×6 图像区右下角");
        assert_eq!(dst.pixel(7, 7), [243, 243, 243, 255], "右下应是留白");
    }

    /// 颜色数上限必须能区分「原生渲染」与「1024 降采样」。
    ///
    /// 实测数字：`cargo tauri icon` 降采样的 32 px 层有 30 种颜色，原生渲染的只有 2 种。
    /// 这里用两张合成图钉住判据本身——降采样那份的中间色数量远超上限。
    #[test]
    fn colour_count_separates_native_render_from_downscale() {
        let native = Rgba {
            width: 4,
            height: 4,
            pixels: [
                [192u8, 54, 44, 255],
                [0, 0, 0, 0],
                [0, 0, 0, 0],
                [192, 54, 44, 255],
            ]
            .repeat(4)
            .concat(),
        };
        assert!(
            distinct_colors(&native).len() <= MAX_SMALL_LAYER_COLORS,
            "原生渲染只有实色与全透明，必须在上限内"
        );

        // 模拟降采样：每个像素带一个不同的 alpha 过渡值。
        let mut blurred = Vec::new();
        for i in 0..16u8 {
            blurred.extend_from_slice(&[192, 54, 44, 16 * i]);
        }
        let downscaled = Rgba {
            width: 4,
            height: 4,
            pixels: blurred,
        };
        assert!(
            distinct_colors(&downscaled).len() > MAX_SMALL_LAYER_COLORS,
            "降采样引入的一串中间色必须超过上限，否则这条断言抓不到漏跑 --ico-only"
        );
    }

    /// 六档必须与 ICO 的必需层完全一致：少一档就等于有一档从没被人眼看过。
    #[test]
    fn contact_sheet_covers_every_required_ico_size() {
        let mut sheet = SHEET_SIZES;
        let mut required = REQUIRED_ICO_SIZES;
        sheet.sort_unstable();
        required.sort_unstable();
        assert_eq!(sheet, required, "联系表列必须覆盖每一个必需的 ICO 层尺寸");
    }

    /// 按骨架参数合成一层，用来把几何断言的正反两面都测到。
    ///
    /// 参数顺序与 `generate_source.py` 的 `SKELETON` 一致：气口 / 朱边 / 笔宽 / 缝宽。
    /// `stroke_h` 单独给出，好让测试能构造一个「笔高不等于 2×笔宽+缝宽」的坏样本。
    fn synth(size: u32, m: u32, b: u32, s: u32, g: u32, stroke_h: u32) -> Rgba {
        let mut pixels = vec![0u8; (size * size * 4) as usize];
        let first = (size - stroke_h) / 2;
        let left = m + b;
        for y in 0..size {
            for x in 0..size {
                let inside = x >= m && x < size - m && y >= m && y < size - m;
                let in_stroke_band = y >= first && y < first + stroke_h;
                let in_left = x >= left && x < left + s;
                let in_right = x >= left + s + g && x < left + 2 * s + g;
                let cell = match (inside, in_stroke_band && (in_left || in_right)) {
                    (false, _) => [0, 0, 0, 0],
                    (true, true) => INK,
                    (true, false) => SEAL,
                };
                let i = ((y * size + x) * 4) as usize;
                pixels[i..i + 4].copy_from_slice(&cell);
            }
        }
        Rgba {
            width: size,
            height: size,
            pixels,
        }
    }

    /// 笔宽期望值必须与 `generate_source.py` 的 `floor(2×缩放+0.5)` 逐档一致。
    ///
    /// 20 与 24 px 是这条唯一有区分力的地方：20 px 的目标值是 2.5，用银行家舍入
    /// 会算出 2，用 `.5` 向上才是 3。这张表就是把两侧算法钉在一起。
    #[test]
    fn expected_stroke_width_matches_the_generator_per_tier() {
        for (size, want) in [
            (16, 2),
            (20, 3),
            (24, 3),
            (32, 4),
            (48, 6),
            (64, 8),
            (128, 16),
            (256, 32),
            (1024, 128),
        ] {
            assert_eq!(expected_stroke_width(size), want, "{size} px");
        }
    }

    /// 量骨架必须从像素反推出与 16 px 定稿一致的每一项。
    #[test]
    fn measures_the_shipped_skeleton_from_pixels() {
        let grid = classify(&synth(16, 1, 4, 2, 2, 6)).expect("只含两种实色");
        assert_eq!(
            measure(&grid).expect("可量"),
            Geometry {
                margin: 1,
                mark: 14,
                border: 4,
                stroke_w: 2,
                gap: 2,
                stroke_h: 6,
            }
        );
    }

    /// v4 的真实失效：某一档的笔画比该档应有的细。这条断言必须抓住它。
    ///
    /// 用 32 px 构造一个笔宽 3 而不是 4 的层——层数、层序、四角 alpha、颜色数
    /// 四项全绿，只有笔宽是错的，正是当时漏过去的那种形态。
    #[test]
    fn catches_a_tier_whose_stroke_is_thinner_than_its_grid() {
        let mut failures = Failures::default();
        check_geometry(&mut failures, 32, &synth(32, 2, 8, 4, 4, 12));
        assert!(failures.into_result().is_ok(), "合格的 32 px 层不该被判红");

        let mut failures = Failures::default();
        check_geometry(&mut failures, 32, &synth(32, 2, 9, 3, 4, 10));
        let err = failures.into_result().expect_err("笔宽 3 应被判红");
        let text = err.to_string();
        assert!(text.contains("笔宽 3"), "{text}");
        assert!(text.contains("= 4"), "失败信息要带上期望值：{text}");
    }

    /// 两笔不等宽必须在量骨架阶段就被拒——那是 v2「最右竖条顶穿右框线」的同类形态。
    #[test]
    fn rejects_layers_whose_two_strokes_differ() {
        let mut layer = synth(16, 1, 4, 2, 2, 6);
        // 把右笔的最右一列改回朱砂，右笔就只剩 1 px。
        for y in 5..11 {
            let i = ((y * 16 + 10) * 4) as usize;
            layer.pixels[i..i + 4].copy_from_slice(&SEAL);
        }
        let grid = classify(&layer).expect("仍只含两种实色");
        let err = measure(&grid).expect_err("两笔不等宽应被拒");
        assert!(err.to_string().contains("等宽"), "{err}");
    }

    /// 出现第三种实色即判为降采样产物：`classify` 必须点名那个像素。
    #[test]
    fn classify_rejects_a_third_solid_colour() {
        let mut layer = synth(16, 1, 4, 2, 2, 6);
        layer.pixels[(((7 * 16) + 7) * 4) as usize..][..4].copy_from_slice(&[200, 100, 90, 255]);
        let err = classify(&layer).expect_err("第三种实色应被拒");
        assert!(err.to_string().contains("(7,7)"), "{err}");
    }

    /// 深色补偿对照图必须真的并置两套配色，否则它证不了补偿做了什么。
    #[test]
    fn compensation_sheet_places_both_palettes_side_by_side() {
        let sheet = render_compensation_sheet(&synth(32, 2, 8, 4, 4, 12));
        assert_eq!((sheet.width, sheet.height), (480, 480));
        // 每格 240 / 源 32 = 7 倍居中，格中心必然落在印面上。
        let centre = |ci: u32, ri: u32| sheet.pixel(ci * 240 + 120, ri * 240 + 120);
        assert_eq!(centre(0, 0), SEAL, "左列必须是本版朱砂");
        assert_eq!(centre(1, 0), LEGACY_SEAL, "右列必须是上一版朱砂");
        assert_ne!(SEAL, LEGACY_SEAL, "两版配色相同则这张表毫无信息量");
        assert_eq!(sheet.pixel(0, 0), [243, 243, 243, 255], "上行是浅底");
        assert_eq!(sheet.pixel(0, 240), [32, 32, 32, 255], "下行是深底");
    }

    /// 门禁必须一次报出全部失败项，而不是第一条就中止。
    #[test]
    fn failures_report_every_item() {
        let mut failures = Failures::default();
        failures.check(false, "a", || "第一条".to_owned());
        failures.check(true, "b", || unreachable!("通过项不应求值 why"));
        failures.check(false, "c", || "第二条".to_owned());
        let err = failures.into_result().expect_err("应失败");
        let text = err.to_string();
        assert!(text.contains("共 2 项"), "{text}");
        assert!(
            text.contains("a: 第一条") && text.contains("c: 第二条"),
            "{text}"
        );
    }
}
