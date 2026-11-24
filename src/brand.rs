pub const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

/// Decode the app icon into RGBA pixels (width, height, data).
#[cfg(not(target_os = "linux"))]
pub fn icon_rgba() -> Option<(u32, u32, Vec<u8>)> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(ICON_PNG)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    Some((info.width, info.height, buf))
}

const TRAY_CANVAS: usize = 64;
const TRAY_GLYPH: usize = 48;

/// Tray glyph: the white "Z" of the app icon on a transparent background,
/// filled with `rgb`, cropped and scaled to sit small inside a square canvas.
#[cfg(not(target_os = "linux"))]
pub fn tray_rgba(rgb: [u8; 3]) -> Option<(u32, u32, Vec<u8>)> {
    let (w, h, px) = icon_rgba()?;
    let (w, h) = (w as usize, h as usize);
    let min_ch = |p: &[u8]| p[0].min(p[1]).min(p[2]);
    let lo = px.chunks_exact(4).filter(|p| p[3] > 250).map(min_ch).min()? as f32;
    let alpha: Vec<f32> = px
        .chunks_exact(4)
        .map(|p| {
            let t = ((min_ch(p) as f32 - lo) / (255.0 - lo)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t) * p[3] as f32 / 255.0
        })
        .collect();

    // Bounding box of the glyph.
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if alpha[y * w + x] > 0.3 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (gw, gh) = (x1 - x0, y1 - y0);
    let scale = TRAY_GLYPH as f32 / gw.max(gh) as f32;
    let (ow, oh) = ((gw as f32 * scale).round() as usize, (gh as f32 * scale).round() as usize);
    let (ox, oy) = ((TRAY_CANVAS - ow) / 2, (TRAY_CANVAS - oh) / 2);

    // Box-filter downscale of the cropped alpha into the centred slot.
    let mut out = vec![0u8; TRAY_CANVAS * TRAY_CANVAS * 4];
    for y in 0..oh {
        for x in 0..ow {
            let sx0 = x0 as f32 + x as f32 / scale;
            let sx1 = x0 as f32 + (x + 1) as f32 / scale;
            let sy0 = y0 as f32 + y as f32 / scale;
            let sy1 = y0 as f32 + (y + 1) as f32 / scale;
            let (mut sum, mut n) = (0.0, 0.0);
            for sy in (sy0 as usize)..(sy1.ceil() as usize).min(h) {
                for sx in (sx0 as usize)..(sx1.ceil() as usize).min(w) {
                    sum += alpha[sy * w + sx];
                    n += 1.0;
                }
            }
            let a = if n > 0.0 { sum / n } else { 0.0 };
            let i = ((oy + y) * TRAY_CANVAS + ox + x) * 4;
            out[i..i + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], (a * 255.0) as u8]);
        }
    }
    Some((TRAY_CANVAS as u32, TRAY_CANVAS as u32, out))
}
