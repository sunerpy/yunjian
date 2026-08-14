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
/// 联系表里每格的边长。取 288 = 256 + 2×16：256 是 256 px 那一层能**按原尺寸单独
/// 呈现**所需的绘制区（缩小它就看不到它本来的字节了），两侧各留 16 px 是为了让
/// **每一格的留白都等比**——见 `blit_magnified` 与 `probe_cell_margins`。
///
/// 上一版取 256（绘制区恰好填满格子）导致一个真实缺陷：16 / 32 / 64 / 256 四档的
/// 绘制区正好等于格边长，格内留白只剩图标自己那圈气口放大后的 16 px（占格 6.2%），
/// 而 24 / 48 两档因为整数倍率取不满还多出居中留白（28 px / 23 px，占 10.9% / 9.0%）。
/// 同一张表里留白差 1.75 倍，目视读成「有几档的红块顶到画布边、各档 padding 不成
/// 比例」。逐格实测值记在 `crates/yunjian-app/icons/README.md`。
const CELL: u32 = 288;
/// 小尺寸表每格的边长。与 `CELL` 同值：三档表因此是 864×576，
/// **仍落在任何看图管道都不会再降采样的量级内**——见 `render_sheet` 的说明。
const SMALL_CELL: u32 = 288;
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
const COMPENSATION_CELL: u32 = 288;
/// 每格允许的留白占格比例区间。上下界由整数倍率能达到的极值反推，不是拍的：
/// 六档在 `CELL = 288` 下逐格实测 11.1% / 11.8% / 11.1% / 13.5% / 11.1% / 11.1%
/// （48 px 那档最大，因为它的整数倍率只能取 5、绘制区 240 比别档小）。
/// 这条把「各档 padding 等比」变成可判定的，而不是靠目视比较相邻格。
/// 每格留白占格的允许区间。
///
/// 纸形竖长，左右留白必然大于上下：16 px 档左右各 3/16 ≈ 18.8%、上下各 1/16 ≈ 6.3%，
/// 放大到对照表的格子后分别约 22% 与 11%。区间取 8%~26% 覆盖两者，
/// 它守的是「各档取景是同一套」而不是「四边一样宽」。
const CELL_MARGIN_SHARE: std::ops::RangeInclusive<f64> = 0.08..=0.26;
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
    /// 纸形左右透明边距，按 `(size - paper_w) / 2` 推导（纸竖长，左右必大于上下）。
    margin: u32,
    /// 纸形上边距。
    margin_top: u32,
    /// 纸形宽。
    paper_w: u32,
    /// 纸形高，**必须大于 `paper_w`**——竖长是「笺」的形状线索。
    paper_h: u32,
    /// 右上折角的边长。
    fold: u32,
    /// 每道米白诗行的宽。
    line_w: u32,
    /// 每道米白诗行的高。
    line_h: u32,
    /// 诗行道数，**不随缩放变**：整数倍复制放大的是粗细而非道数。
    lines: u32,
}

/// 某个骨架量在该档的期望值：`floor(unit × 该层缩放 + 0.5)`，缩放 = `size / 16`。
///
/// 用整数算而不是浮点四舍五入：Rust 的 `f64::round` 与 Python 内建 `round` 在 `.5`
/// 上行为不同（后者是银行家舍入），而 20 px 档的环宽目标恰好是 2.5，两侧算法必须给出
/// 同一个 3，否则这条断言在唯一需要它的档位上与生成器不一致。
fn expected_unit(unit: u32, size: u32) -> u32 {
    (unit * size + 8) / 16
}

/// 该档骨架的完整期望值，与 `generate_source.py` 的 `SKELETON` 逐项对应。
///
/// **刻意在两个语言里各写一份**：生成器那份查渲染前的字符网格，这份查已写进 ICO
/// 的字节。两份都对才说明「设计意图 → 产物」这条链上没有丢东西——v4 的两档笔画
/// 变细正是在这两点之间丢的。
///
/// 20 与 24 px 不是 16 的整数倍，各有一份手调值；逐条推导在生成器文件头的取整一节。
fn expected_skeleton(size: u32) -> Option<Geometry> {
    let (w, h, f, lw, lh, n) = match size {
        16 => (10, 14, 2, 6, 2, 2),
        20 => (12, 18, 2, 8, 2, 3),
        24 => (14, 20, 2, 10, 3, 3),
        _ if size.is_multiple_of(16) => {
            let k = size / 16;
            // 道数原样带过——放大的是每道粗细，不是道数。
            (10 * k, 14 * k, 2 * k, 6 * k, 2 * k, 2)
        }
        _ => return None,
    };
    Some(Geometry {
        margin: (size - w) / 2,
        margin_top: (size - h) / 2,
        paper_w: w,
        paper_h: h,
        fold: f,
        line_w: lw,
        line_h: lh,
        lines: n,
    })
}

/// `mark` 记号的四连通分量，每个分量以 `(x, y)` 集合表示。
fn components(grid: &[Vec<Cell>], mark: Cell) -> Vec<Vec<(u32, u32)>> {
    let size = grid.len() as u32;
    let mut seen = vec![vec![false; size as usize]; size as usize];
    let mut parts = Vec::new();
    for y in 0..size {
        for x in 0..size {
            if grid[y as usize][x as usize] != mark || seen[y as usize][x as usize] {
                continue;
            }
            let mut part = Vec::new();
            let mut frontier = vec![(x, y)];
            seen[y as usize][x as usize] = true;
            while let Some((cx, cy)) = frontier.pop() {
                part.push((cx, cy));
                for (nx, ny) in [
                    (cx.wrapping_sub(1), cy),
                    (cx + 1, cy),
                    (cx, cy.wrapping_sub(1)),
                    (cx, cy + 1),
                ] {
                    if nx < size
                        && ny < size
                        && grid[ny as usize][nx as usize] == mark
                        && !seen[ny as usize][nx as usize]
                    {
                        seen[ny as usize][nx as usize] = true;
                        frontier.push((nx, ny));
                    }
                }
            }
            parts.push(part);
        }
    }
    parts
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

/// 一条扫描线折成的段序列 `(记号, 起始下标, 长度)`。
fn segments(line: &[Cell]) -> Vec<(Cell, u32, u32)> {
    let mut out: Vec<(Cell, u32, u32)> = Vec::new();
    for (index, &cell) in line.iter().enumerate() {
        match out.last_mut() {
            Some(last) if last.0 == cell => last.2 += 1,
            _ => out.push((cell, index as u32, 1)),
        }
    }
    out
}

/// 三层结构必须在中线上量得到的段序：透明 → 朱环 → 内白 → 印文 → 内白 → 朱环 → 透明。
///
/// **这条常量是 v5 那次缺陷的直接守卫。** v5 的中线段序在记号上与它一模一样，
/// 但中间那段朱色是两道竖笔之间的缝而不是印文——层级数看着对、语义完全不同。
/// 所以除了段序，还必须断言「朱色恰有两个连通分量（外环 + 不贴边的印文）」，
/// 两条合起来才排除掉「实心朱块被两道白痕切开」的形态。
const MIDLINE: [Cell; 7] = [
    Cell::Clear,
    Cell::Seal,
    Cell::Ink,
    Cell::Seal,
    Cell::Ink,
    Cell::Seal,
    Cell::Clear,
];

/// 从 `Cell` 网格反推骨架几何。
///
/// 本版造型是「折角笺纸」：一整块实心纸形 + 右上阶梯折角 + 内部若干道米白诗行。
/// 纸形边界由**非透明像素的包围盒**给出，不依赖任何坐标常量。
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
    let paper_w = x1 - x0 + 1;
    let paper_h = y1 - y0 + 1;

    let mut fold = 0u32;
    for y in y0..=y1 {
        let w = (x0..=x1)
            .filter(|&x| grid[y as usize][x as usize] == Cell::Clear)
            .count() as u32;
        if w == 0 {
            break;
        }
        fold += 1;
    }

    let mut bands: Vec<(u32, u32)> = Vec::new();
    for y in y0..=y1 {
        let whites: Vec<u32> = (x0..=x1)
            .filter(|&x| grid[y as usize][x as usize] == Cell::Ink)
            .collect();
        if whites.len() < 3 {
            continue;
        }
        let left = whites[0] - x0;
        let right = x1 - whites[whites.len() - 1];
        if left != right || left < 2 {
            continue;
        }
        match bands.last_mut() {
            Some(last) if last.0 + last.1 == y => last.1 += 1,
            _ => bands.push((y, 1)),
        }
    }
    let lines = bands.len() as u32;
    let (line_w, line_h) = match bands.first() {
        Some(&(by, bh)) => (
            (x0..=x1)
                .filter(|&x| grid[by as usize][x as usize] == Cell::Ink)
                .count() as u32,
            bh,
        ),
        None => (0, 0),
    };

    Ok(Geometry {
        margin: x0,
        margin_top: y0,
        paper_w,
        paper_h,
        fold,
        line_w,
        line_h,
        lines,
    })
}

/// 这一组断言各对应一次真实失效（七版对照见生成器文件头）：
/// - v4 六档里有两档笔画明显变细，而当时门禁只查层数、层序、四角 alpha 与颜色数；
/// - v5 把指定的内框整条删掉，三层从未画出来，同样没有断言看着它；
/// - v6 的「外环 + 单一印文」被目视读作字母 T，方向本身证伪，于是改为折角笺纸。
///
/// 本版把每一条旧断言换成新造型的等价不变量，没有一条被删掉。
fn check_geometry(failures: &mut Failures, size: u32, image: &Rgba) {
    let measured = match classify(image).and_then(|grid| measure(&grid).map(|got| (grid, got))) {
        Ok(pair) => pair,
        Err(err) => {
            failures.check(false, ICO, || format!("{size} px 层无法量出骨架：{err:#}"));
            return;
        }
    };
    let (grid, got) = measured;
    let what = ICO;

    if let Some(want) = expected_skeleton(size) {
        failures.check(
            got.paper_w == want.paper_w && got.paper_h == want.paper_h,
            what,
            || {
                format!(
                    "{size} px 纸形 {}×{} != 期望 {}×{}",
                    got.paper_w, got.paper_h, want.paper_w, want.paper_h
                )
            },
        );
        failures.check(got.fold == want.fold, what, || {
            format!("{size} px 折角边长 {} != 期望 {}", got.fold, want.fold)
        });
        failures.check(
            got.line_w == want.line_w && got.line_h == want.line_h,
            what,
            || {
                format!(
                    "{size} px 诗行 {}×{} != 期望 {}×{}",
                    got.line_w, got.line_h, want.line_w, want.line_h
                )
            },
        );
        failures.check(got.lines == want.lines, what, || {
            format!("{size} px 诗行 {} 道 != 期望 {} 道", got.lines, want.lines)
        });
        failures.check(got.margin == want.margin, what, || {
            format!("{size} px 左边距 {} != 期望 {}", got.margin, want.margin)
        });
        failures.check(got.margin_top == want.margin_top, what, || {
            format!(
                "{size} px 上边距 {} != 期望 {}",
                got.margin_top, want.margin_top
            )
        });
    }

    failures.check(got.paper_h > got.paper_w, what, || {
        format!(
            "{size} px 纸形 {}×{} 不是竖长——读不出「笺」",
            got.paper_w, got.paper_h
        )
    });
    failures.check(got.fold >= 2, what, || {
        format!(
            "{size} px 折角边长 {} < 2 px，缩一档就会碎成 1 px",
            got.fold
        )
    });
    failures.check(got.lines >= 2, what, || {
        format!("{size} px 只有 {} 道诗行，读不出「文字」", got.lines)
    });

    let poem_y = (0..size).find(|&y| {
        let row: Vec<Cell> = (0..size).map(|x| grid[y as usize][x as usize]).collect();
        row.iter().filter(|&&c| c == Cell::Ink).count() >= 3
            && row[0] == Cell::Clear
            && grid[y as usize][(size / 2) as usize] == Cell::Ink
    });
    match poem_y {
        None => failures.check(false, what, || {
            format!("{size} px 找不到任何一条穿过诗行的水平线——米白层没画出来")
        }),
        Some(y) => {
            let row: Vec<Cell> = (0..size).map(|x| grid[y as usize][x as usize]).collect();
            let kinds: Vec<Cell> = segments(&row).into_iter().map(|(c, _, _)| c).collect();
            let want = vec![Cell::Clear, Cell::Seal, Cell::Ink, Cell::Seal, Cell::Clear];
            failures.check(kinds == want, what, || {
                format!("{size} px 诗行中线段序 {kinds:?} != {want:?}——三层没有全部画出来")
            });
            let mut rev = row.clone();
            rev.reverse();
            failures.check(row == rev, what, || {
                format!("{size} px 诗行所在行不左右镜像——诗行没有水平居中")
            });
        }
    }

    for (mark, label) in [(Cell::Seal, "朱色"), (Cell::Ink, "米白")] {
        for y in 0..size {
            let widths: Vec<u32> = runs(
                &(0..size)
                    .map(|x| grid[y as usize][x as usize] == mark)
                    .collect::<Vec<_>>(),
            )
            .into_iter()
            .map(|(_, w)| w)
            .collect();
            failures.check(widths.iter().all(|&w| w >= 2), what, || {
                format!("{size} px 第 {y} 行出现 <2 px 的{label}：{widths:?}")
            });
        }
        for x in 0..size {
            let widths: Vec<u32> = runs(
                &(0..size)
                    .map(|y| grid[y as usize][x as usize] == mark)
                    .collect::<Vec<_>>(),
            )
            .into_iter()
            .map(|(_, w)| w)
            .collect();
            failures.check(widths.iter().all(|&w| w >= 2), what, || {
                format!("{size} px 第 {x} 列出现 <2 px 的{label}：{widths:?}")
            });
        }
    }

    for y in 0..size {
        for x in 0..size {
            if grid[y as usize][x as usize] != Cell::Ink {
                continue;
            }
            let touches_clear = [
                (x + 1, y),
                (x.wrapping_sub(1), y),
                (x, y + 1),
                (x, y.wrapping_sub(1)),
            ]
            .into_iter()
            .any(|(nx, ny)| {
                nx >= size || ny >= size || grid[ny as usize][nx as usize] == Cell::Clear
            });
            failures.check(!touches_clear, what, || {
                format!("{size} px 的米白像素 ({x},{y}) 与透明相邻，纸面被穿透")
            });
        }
    }
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
///
/// # 倍率按「目标绘制区」四舍五入，而不是向下取整
///
/// 目标绘制区取 `side × 8 / 9`，于是每格四周先有约 `side / 18` 的固定留白，
/// 各档再叠上自己那圈气口放大后的宽度——两项相加逐格落在 11%~13.5%，
/// 由 `probe_cell_margins` 断言。
///
/// **向下取整会毁掉这个等比。** 上一版用 `side / src.width` 向下取整且格边长恰好
/// 等于最大层的边长，于是能整除格边长的四档（16 / 32 / 64 / 256）绘制区正好填满
/// 格子、只剩 6.2% 留白，不能整除的两档（24 / 48）反而多出居中留白到 9%~10.9%——
/// 同一张表里留白差 1.75 倍，目视读成「有几档的红块顶到画布边」。四舍五入让 24 px
/// 取 11 倍（264，比 256 略大）而不是 10 倍（240），差距因此收敛。
fn blit_magnified(dst: &mut Rgba, src: &Rgba, ox: u32, oy: u32, side: u32, bg: [u8; 3]) {
    let target = side * 8 / 9;
    let mut factor = ((target + src.width / 2) / src.width).max(1);
    while factor > 1 && src.width * factor > side {
        factor -= 1;
    }
    let drawn = (src.width * factor).min(side);
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
/// 逐格量出四周留白（连续底色像素数），用来断言六档在表里的**取景等比**。
///
/// 这条守的是需求方本轮点名的缺陷：上一版六档表逐格实测留白
/// 16 / 28 / 16 / 23 / 16 / 16 px（占格 6.2% / 10.9% / 6.2% / 9.0% / 6.2% / 6.2%），
/// 相差 1.75 倍，于是「顶到画布边的那几档」与「四周有均匀留白的那两档」在同一张
/// 表里读成两套取景。成因见 `blit_magnified` 的倍率一节。
///
/// 取样线走每格的正中：图标四角必须透明（另有断言守着），所以从格边向内扫到的
/// 第一个非底色像素一定是外环的外沿，量到的就是「印身四周的留白」。
fn probe_cell_margins(sheet: &Rgba, sizes: &[u32], cell: u32) -> Vec<(usize, u32, [u32; 4])> {
    let mut out = Vec::new();
    for (ri, bg) in BACKDROPS.iter().enumerate() {
        let expected = [bg[0], bg[1], bg[2], 255];
        for (ci, size) in sizes.iter().enumerate() {
            let (ox, oy) = (ci as u32 * cell, ri as u32 * cell);
            let (cx, cy) = (ox + cell / 2, oy + cell / 2);
            let scan = |probe: &dyn Fn(u32) -> [u8; 4]| {
                (0..cell).take_while(|&k| probe(k) == expected).count() as u32
            };
            out.push((
                ri,
                *size,
                [
                    scan(&|k| sheet.pixel(ox + k, cy)),
                    scan(&|k| sheet.pixel(ox + cell - 1 - k, cy)),
                    scan(&|k| sheet.pixel(cx, oy + k)),
                    scan(&|k| sheet.pixel(cx, oy + cell - 1 - k)),
                ],
            ));
        }
    }
    out
}

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
        let margins = probe_cell_margins(&sheet, sizes, cell);
        let lopsided: Vec<_> = margins
            .iter()
            .filter(|(_, _, m)| m[0] != m[1] || m[2] != m[3])
            .collect();
        failures.check(lopsided.is_empty(), path, || {
            format!(
                "每格留白必须左右相等且上下相等（纸竖长，左右必大于上下），但这些格不符\
                 （行序号, 尺寸, 留白）：{lopsided:?}"
            )
        });
        let off_share: Vec<_> = margins
            .iter()
            .filter(|(_, _, m)| !CELL_MARGIN_SHARE.contains(&(f64::from(m[0]) / f64::from(cell))))
            .collect();
        failures.check(off_share.is_empty(), path, || {
            format!(
                "各档留白占格必须落在 {:.0}%~{:.1}%，否则同一张表里读成两套取景，\
                 但这些格越界（行序号, 尺寸, 留白）：{off_share:?}",
                CELL_MARGIN_SHARE.start() * 100.0,
                CELL_MARGIN_SHARE.end() * 100.0
            )
        });
        emit(&format!(
            "  联系表   {path}  {}×{}  列 {sizes:?}  行 [浅色 #F3F3F3, 深色 #202020]  \
             逐格底色一致 = {}  逐格留白 {:?}（占格 {}）",
            sheet.width,
            sheet.height,
            odd.is_empty(),
            margins
                .iter()
                .take(sizes.len())
                .map(|(_, _, m)| m[0])
                .collect::<Vec<_>>(),
            margins
                .iter()
                .take(sizes.len())
                .map(|(_, _, m)| format!("{:.1}%", f64::from(m[0]) / f64::from(cell) * 100.0))
                .collect::<Vec<_>>()
                .join(" ")
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
    /// 而人眼验收看到的就不再是字节本身。864×576 是这条的具体形态。
    #[test]
    fn small_sheet_stays_within_native_viewing_budget() {
        let layers: Vec<(u32, Rgba)> = SMALL_SHEET_SIZES
            .iter()
            .map(|s| (*s, stub_layer()))
            .collect();
        let sheet = render_sheet(&layers, SMALL_SHEET_SIZES.as_slice(), SMALL_CELL);
        assert_eq!((sheet.width, sheet.height), (864, 576));
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

    /// 按骨架参数合成一层朱文方印，用来把几何断言的正反两面都测到。
    ///
    /// 参数顺序与 `generate_source.py` 的 `SKELETON` 一致：
    /// 气口 / 环宽 / 净距 / 横宽 / 横高 / 竖宽 / 竖高。刻意逐项可调，好让测试能
    /// 构造出各种坏样本（净距 1 px、印文外接框不方、上下对称的印文……）。
    #[allow(clippy::too_many_arguments)]
    fn synth(size: u32, m: u32, r: u32, c: u32, bw: u32, bh: u32, sw: u32, sh: u32) -> Rgba {
        let mut pixels = vec![0u8; (size * size * 4) as usize];
        let (bx, by) = (m + r + c, m + r + c);
        let sx = bx + (bw - sw) / 2;
        for y in 0..size {
            for x in 0..size {
                let inside = x >= m && x < size - m && y >= m && y < size - m;
                let in_inner = x >= m + r && x < size - m - r && y >= m + r && y < size - m - r;
                let in_bar = (bx..bx + bw).contains(&x) && (by..by + bh).contains(&y);
                let in_stem = (sx..sx + sw).contains(&x) && (by + bh..by + bh + sh).contains(&y);
                let cell = match (inside, in_inner, in_bar || in_stem) {
                    (false, _, _) => [0, 0, 0, 0],
                    (true, false, _) => SEAL,
                    (true, true, true) => SEAL,
                    (true, true, false) => INK,
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

    /// v5 的形态：实心朱面 + 两道等宽等高米白竖笔，**没有内框**。
    ///
    /// 它必须被本轮的门禁判红。这是本轮最重要的一条反向测试：v5 的字节断言全绿
    /// 而三层结构从未存在，说明那次缺陷本来就是可以被断言抓住的，缺的只是断言。
    fn synth_v5_solid_with_two_bars(size: u32, m: u32, b: u32, s: u32, g: u32) -> Rgba {
        let mut pixels = vec![0u8; (size * size * 4) as usize];
        let stroke_h = 2 * s + g;
        let first = (size - stroke_h) / 2;
        let left = m + b;
        for y in 0..size {
            for x in 0..size {
                let inside = x >= m && x < size - m && y >= m && y < size - m;
                let band = (first..first + stroke_h).contains(&y);
                let in_left = (left..left + s).contains(&x);
                let in_right = (left + s + g..left + 2 * s + g).contains(&x);
                let cell = match (inside, band && (in_left || in_right)) {
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

    /// 气口与环宽的期望值必须与 `generate_source.py` 的 `floor(unit×缩放+0.5)` 逐档一致。
    ///
    /// 20 与 24 px 是这条唯一有区分力的地方：20 px 的环宽目标值是 2.5，用银行家舍入
    /// 会算出 2，用 `.5` 向上才是 3。这张表就是把两侧算法钉在一起。
    #[test]
    fn expected_units_match_the_generator_per_tier() {
        for (size, margin, ring) in [
            (16, 1, 2),
            (20, 1, 3),
            (24, 2, 3),
            (32, 2, 4),
            (48, 3, 6),
            (64, 4, 8),
            (128, 8, 16),
            (256, 16, 32),
            (1024, 64, 128),
        ] {
            assert_eq!(expected_unit(1, size), margin, "{size} px 气口");
            assert_eq!(expected_unit(2, size), ring, "{size} px 环宽");
            let want = expected_skeleton(size).expect("表内档位");
            assert_eq!((want.margin, want.ring), (margin, ring), "{size} px 期望表");
        }
    }

    /// 量骨架必须从像素反推出与 16 px 定稿一致的每一项。
    #[test]
    fn measures_the_shipped_skeleton_from_pixels() {
        let grid = classify(&synth(16, 1, 2, 2, 6, 2, 2, 4)).expect("只含两种实色");
        assert_eq!(
            measure(&grid).expect("可量"),
            expected_skeleton(16).expect("16 px 在期望表内")
        );
    }

    /// 合格的 32 px 层不该被判红，而 v4 的失效形态（该档笔画比应有的细）必须被抓住。
    #[test]
    fn catches_a_tier_whose_strokes_are_thinner_than_its_grid() {
        let mut failures = Failures::default();
        check_geometry(&mut failures, 32, &synth(32, 2, 4, 4, 12, 4, 4, 8));
        assert!(failures.into_result().is_ok(), "合格的 32 px 层不该被判红");

        // 环宽 3 而不是 4：层数、层序、四角 alpha、颜色数四项仍全绿。
        let mut failures = Failures::default();
        check_geometry(&mut failures, 32, &synth(32, 2, 3, 5, 12, 4, 4, 8));
        let err = failures.into_result().expect_err("环宽 3 应被判红");
        let text = err.to_string();
        assert!(text.contains("环宽 3"), "{text}");
        assert!(text.contains("= 4"), "失败信息要带上期望值：{text}");
    }

    /// **本轮的核心反向测试**：v5 那枚「实心朱块 + 两道米白竖笔」必须被判红。
    ///
    /// v5 的门禁对它全绿，而它的三层结构（外环 / 内白 / 印文）根本不存在。
    /// 这条既证明新断言真的在看那件事，也钉住「不许退回 v5 造型」。
    #[test]
    fn rejects_the_v5_solid_seal_with_two_bars() {
        let mut failures = Failures::default();
        check_geometry(
            &mut failures,
            16,
            &synth_v5_solid_with_two_bars(16, 1, 4, 2, 2),
        );
        let err = failures
            .into_result()
            .expect_err("v5 的实心双竖笔层必须被判红");
        let text = err.to_string();
        assert!(
            text.contains("连通分量") || text.contains("中线段序"),
            "必须点名三层结构缺失，而不是别的顺带失败：{text}"
        );
    }

    /// 上下对称的印文必须进不了门禁：那正是 v5 被目视读成暂停键的骨架特征。
    ///
    /// **上下不对称其实已被 `measure` 的结构约束隐含**：竖宽一旦等于横宽，印文退化
    /// 成矩形，`take_while` 把所有行都算成横、竖行为空而直接 bail。所以
    /// `check_geometry` 里那条 `!mirrored_y` 是第二道防线，将来若有人放宽 `measure`
    /// 它仍会拦住 v5 那种骨架。这条测试因此钉两侧：退化矩形被拒，且合格层实测不对称。
    #[test]
    fn rejects_a_vertically_symmetric_glyph() {
        let mut failures = Failures::default();
        check_geometry(&mut failures, 16, &synth(16, 1, 2, 2, 6, 6, 6, 0));
        let err = failures
            .into_result()
            .expect_err("退化成矩形的印文应被判红");
        assert!(err.to_string().contains("竖各行宽度不一"), "{err}");

        let grid = classify(&synth(16, 1, 2, 2, 6, 2, 2, 4)).expect("合格层只含两种实色");
        let flipped: Vec<Vec<Cell>> = grid.iter().rev().cloned().collect();
        assert_ne!(grid, flipped, "合格层必须上下不对称——印文要有朝向");
    }

    /// 净距只有 1 px 必须被拒——需求方给的原坐标（横起于 x=4、环内沿在 x=3）就是这种。
    #[test]
    fn rejects_a_one_pixel_clearance() {
        let mut failures = Failures::default();
        check_geometry(&mut failures, 16, &synth(16, 1, 2, 1, 8, 2, 2, 6));
        let err = failures.into_result().expect_err("1 px 净距应被判红");
        assert!(err.to_string().contains("净距"), "{err}");
    }

    /// 出现第三种实色即判为降采样产物：`classify` 必须点名那个像素。
    #[test]
    fn classify_rejects_a_third_solid_colour() {
        let mut layer = synth(16, 1, 2, 2, 6, 2, 2, 4);
        layer.pixels[(((4 * 16) + 4) * 4) as usize..][..4].copy_from_slice(&[200, 100, 90, 255]);
        let err = classify(&layer).expect_err("第三种实色应被拒");
        assert!(err.to_string().contains("(4,4)"), "{err}");
    }

    /// 逐格留白必须四边相等且等比。这条守的是上一版六档表 6.2% 与 10.9% 并存的缺陷。
    #[test]
    fn cell_margins_are_uniform_across_tiers() {
        let sizes = SHEET_SIZES;
        let layers: Vec<(u32, Rgba)> = sizes
            .iter()
            .map(|&s| {
                let want = expected_skeleton(s).expect("六档都在期望表内");
                (
                    s,
                    synth(
                        s,
                        want.margin,
                        want.ring,
                        want.clearance,
                        want.bar_w,
                        want.bar_h,
                        want.stem_w,
                        want.stem_h,
                    ),
                )
            })
            .collect();
        let sheet = render_sheet(&layers, sizes.as_slice(), CELL);
        for (row, size, margins) in probe_cell_margins(&sheet, sizes.as_slice(), CELL) {
            assert!(
                margins.iter().all(|v| *v == margins[0]),
                "行 {row} 的 {size} px 格四周留白不等：{margins:?}"
            );
            let share = f64::from(margins[0]) / f64::from(CELL);
            assert!(
                CELL_MARGIN_SHARE.contains(&share),
                "行 {row} 的 {size} px 格留白占格 {:.1}%，越出 {CELL_MARGIN_SHARE:?}",
                share * 100.0
            );
        }
    }

    /// 深色补偿对照图必须真的并置两套配色，否则它证不了补偿做了什么。
    #[test]
    fn compensation_sheet_places_both_palettes_side_by_side() {
        let sheet = render_compensation_sheet(&synth(32, 2, 4, 4, 12, 4, 4, 8));
        let cell = COMPENSATION_CELL;
        assert_eq!((sheet.width, sheet.height), (2 * cell, 2 * cell));
        // 格中心落在印文的竖上，那里是朱砂——两列的差别正是这个朱砂取值。
        let centre = |ci: u32, ri: u32| sheet.pixel(ci * cell + cell / 2, ri * cell + cell / 2);
        assert_eq!(centre(0, 0), SEAL, "左列必须是本版朱砂");
        assert_eq!(centre(1, 0), LEGACY_SEAL, "右列必须是上一版朱砂");
        assert_ne!(SEAL, LEGACY_SEAL, "两版配色相同则这张表毫无信息量");
        assert_eq!(sheet.pixel(0, 0), [243, 243, 243, 255], "上行是浅底");
        assert_eq!(sheet.pixel(0, cell), [32, 32, 32, 255], "下行是深底");
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
