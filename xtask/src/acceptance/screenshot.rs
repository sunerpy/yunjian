//! X11 截图。
//!
//! # 为什么截图是必需的证据，而不是装饰
//!
//! 断言「窗口没有窗口管理器绘制的边框」可以由 `_NET_FRAME_EXTENTS` 回答，但断言
//! 「界面上确实画出了自绘标题栏」不能——属性只说明窗口管理器做了什么，说明不了应用
//! 画了什么。本项目在图标那一轮已经吃过同一个教训：字节断言全绿而 16 px 下的设计
//! 不可用。所以每条 UI 断言都要留一张图。
//!
//! # 为什么不调 `scrot` / `import`
//!
//! 那会把「有没有截到图」变成宿主机装了哪些工具的函数，而 X11 的 `GetImage` 本身就是
//! 一次请求。同时 `png` crate 已经在 xtask 的依赖树里（`verify-icons` 用它），编码是
//! 现成的。少一个外部依赖，少一种「本机能跑 CI 不能」的形态。

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use anyhow::{Context, Result, bail};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat, Window};

/// 一次窗口像素读取的度量结果。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Paint {
    /// 最常见那一种颜色。
    pub(crate) dominant: [u8; 3],
    /// 它占了多少比例。
    pub(crate) dominant_share: f64,
}

impl Paint {
    /// 窗口内容是否读得到。
    ///
    /// 判据是**主色占比**而不是颜色种数。种数会被噪声骗过：本机实测一块 99.996% 纯黑的
    /// 窗口里也有 21 种颜色（21 个像素的杂色），按「种数 > 2」判会被当成画好了。
    /// 主色占比不会——纯黑那一次是 0.99996。
    ///
    /// 阈值取 0.95：真实界面里最大的一块是内容区底色，而标题栏（40 px）与导航条
    /// （41 px）各有自己的底色，合起来已占 800 px 高度的一成，所以主色到不了 95%。
    pub(crate) fn painted(self) -> bool {
        self.dominant_share < 0.95
    }
}

/// 读一次窗口像素，**同时**写出 PNG 并度量它。
///
/// # 为什么必须是同一次读取
///
/// 分两次读会有竞态：本次开发中先数出 21 种颜色、紧接着写出的图却只有 1 种，
/// 于是「依据」和「证据」互相矛盾——而报告的价值全在两者一致。合成一次之后，
/// 图就是那个数字的出处。
///
/// # 这个度量用来回答一个必须先回答的问题
///
/// 「点在最大化按钮上，窗口最大化了吗」这句话只有在**按钮所在的像素读得到**时才有意义。
/// 一块几乎单色的窗口意味着界面内容不在这块 X 窗口上（没渲染，或被合成到了一块读不到的
/// GL 表面），此时点击落在空处、什么都不会发生，而那**不一定是产品缺陷**——
/// 把它记成 FAIL 是在报告一个可能不存在的故障。
///
/// # Errors
///
/// X11 请求失败、像素格式不是 32 位，或写文件失败。
pub(crate) fn capture_and_measure(
    conn: &impl Connection,
    window: Window,
    path: &Path,
) -> Result<Paint> {
    let (width, height, rgba) = read_size_and_rgba(conn, window)?;
    write_png(path, width, height, &rgba)?;

    // 直方图开在 5 位色深上（32³ = 32768 桶）：要回答的是「这一大片是不是同一个颜色」，
    // 而抗锯齿会让同一片底色散成若干个相差 1 的值，按全 24 位统计反而把它拆开。
    let mut histogram = vec![0u32; 32 * 32 * 32];
    let mut total = 0u32;
    for chunk in rgba.chunks_exact(4) {
        let index = (usize::from(chunk[0] >> 3) << 10)
            | (usize::from(chunk[1] >> 3) << 5)
            | usize::from(chunk[2] >> 3);
        histogram[index] += 1;
        total += 1;
    }
    let (index, count) = histogram
        .iter()
        .enumerate()
        .max_by_key(|(_, count)| **count)
        .map(|(index, count)| (index, *count))
        .unwrap_or((0, 0));
    let dominant = [
        ((index >> 10) as u8) << 3,
        (((index >> 5) & 0x1F) as u8) << 3,
        ((index & 0x1F) as u8) << 3,
    ];
    Ok(Paint {
        dominant,
        dominant_share: if total == 0 {
            1.0
        } else {
            f64::from(count) / f64::from(total)
        },
    })
}

/// 把 `window` 的当前像素写成 PNG。`window` 传 root 即整屏。
///
/// # Errors
///
/// X11 请求失败、窗口已销毁、像素格式不是 32 位，或写文件失败。
pub(crate) fn capture(conn: &impl Connection, window: Window, path: &Path) -> Result<()> {
    let (width, height, rgba) = read_size_and_rgba(conn, window)?;
    write_png(path, width, height, &rgba)
}

/// 把 RGBA 像素写成 PNG。
///
/// # Errors
///
/// 建目录、建文件或 PNG 编码失败。
fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建 {} 失败", parent.display()))?;
    }
    let file = File::create(path).with_context(|| format!("创建 {} 失败", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .with_context(|| format!("写 {} 的 PNG 头失败", path.display()))?;
    writer
        .write_image_data(rgba)
        .with_context(|| format!("写 {} 的 PNG 数据失败", path.display()))?;
    Ok(())
}

/// 读窗口像素并转成 RGBA，返回（宽，高，像素）。
///
/// X11 的 `Z_PIXMAP` 在 32 位小端视觉下是 BGRA；这里换成 RGBA 并把 alpha 一律写满：
/// 读到的是**已合成到屏幕上**的像素，透明度在这里没有意义，保留它只会让看图的人
/// 以为某块区域是透明的。
///
/// # Errors
///
/// X11 请求失败、窗口尺寸为 0，或像素格式不是 32 位。
fn read_size_and_rgba(conn: &impl Connection, window: Window) -> Result<(u32, u32, Vec<u8>)> {
    let geometry = conn
        .get_geometry(window)
        .context("请求窗口几何失败")?
        .reply()
        .context("读取窗口几何失败")?;
    if geometry.width == 0 || geometry.height == 0 {
        bail!("窗口尺寸为 0，没有可读的像素");
    }
    let image = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            window,
            0,
            0,
            geometry.width,
            geometry.height,
            !0,
        )
        .context("请求窗口像素失败")?
        .reply()
        .context("读取窗口像素失败")?;

    let width = u32::from(geometry.width);
    let height = u32::from(geometry.height);
    let pixels = (width as usize) * (height as usize);
    let expected = pixels * 4;
    if image.data.len() < expected {
        bail!(
            "像素数据只有 {} 字节，按 {width}×{height} 的 32 位格式应有 {expected} 字节；\
             本函数只支持 32 位深度的视觉",
            image.data.len()
        );
    }
    let mut rgba = Vec::with_capacity(expected);
    for chunk in image.data.chunks_exact(4).take(pixels) {
        rgba.extend_from_slice(&[chunk[2], chunk[1], chunk[0], 0xFF]);
    }
    Ok((width, height, rgba))
}
