//! Color resolution and pixel-blitting helpers.

use rio_vt::config::colors::term::TermColors;
use rio_vt::config::colors::{AnsiColor, NamedColor};
pub(crate) fn resolve_named(colors: &TermColors, name: NamedColor, dr: u8, dg: u8, db: u8) -> u32 {
    if let Some(a) = &colors[name] {
        arr_to_u32(*a)
    } else {
        rgb_u32(dr, dg, db)
    }
}

pub(crate) fn indexed_color(idx: u8) -> (u8, u8, u8) {
    match idx {
        0 => (0x00, 0x00, 0x00),
        1 => (0xcd, 0x00, 0x00),
        2 => (0x00, 0xcd, 0x00),
        3 => (0xcd, 0xcd, 0x00),
        4 => (0x00, 0x00, 0xee),
        5 => (0xcd, 0x00, 0xcd),
        6 => (0x00, 0xcd, 0xcd),
        7 => (0xe5, 0xe5, 0xe5),
        8 => (0x7f, 0x7f, 0x7f),
        9 => (0xff, 0x00, 0x00),
        10 => (0x00, 0xff, 0x00),
        11 => (0xff, 0xff, 0x00),
        12 => (0x5c, 0x5c, 0xff),
        13 => (0xff, 0x00, 0xff),
        14 => (0x00, 0xff, 0xff),
        15 => (0xff, 0xff, 0xff),
        16..=231 => {
            let i = idx - 16;
            let r = i / 36;
            let g = (i / 6) % 6;
            let b = i % 6;
            let cv = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            (cv(r), cv(g), cv(b))
        }
        232..=255 => {
            let v = 8 + 10 * (idx - 232);
            (v, v, v)
        }
    }
}

/// Fallback RGB for a named color. Foreground and its dim/light aliases all
/// use the foreground slot, as in rio's palette.
fn named_default(name: NamedColor) -> (NamedColor, (u8, u8, u8)) {
    let rgb = match name {
        NamedColor::Foreground | NamedColor::LightForeground | NamedColor::DimForeground => {
            (0xd8, 0xd8, 0xd8)
        }
        NamedColor::Background => (0x10, 0x10, 0x10),
        NamedColor::Cursor => (0xf0, 0xf0, 0xf0),
        NamedColor::Black | NamedColor::DimBlack => (0, 0, 0),
        NamedColor::Red => (0xcd, 0, 0),
        NamedColor::Green => (0, 0xcd, 0),
        NamedColor::Yellow => (0xcd, 0xcd, 0),
        NamedColor::Blue => (0, 0, 0xee),
        NamedColor::Magenta => (0xcd, 0, 0xcd),
        NamedColor::Cyan => (0, 0xcd, 0xcd),
        NamedColor::White => (0xe5, 0xe5, 0xe5),
        NamedColor::LightBlack => (0x7f, 0x7f, 0x7f),
        NamedColor::LightRed => (0xff, 0, 0),
        NamedColor::LightGreen => (0, 0xff, 0),
        NamedColor::LightYellow => (0xff, 0xff, 0),
        NamedColor::LightBlue => (0x5c, 0x5c, 0xff),
        NamedColor::LightMagenta => (0xff, 0, 0xff),
        NamedColor::LightCyan => (0, 0xff, 0xff),
        NamedColor::LightWhite => (0xff, 0xff, 0xff),
        NamedColor::DimRed => (0x88, 0, 0),
        NamedColor::DimGreen => (0, 0x88, 0),
        NamedColor::DimYellow => (0x88, 0x88, 0),
        NamedColor::DimBlue => (0, 0, 0x88),
        NamedColor::DimMagenta => (0x88, 0, 0x88),
        NamedColor::DimCyan => (0, 0x88, 0x88),
        NamedColor::DimWhite => (0x99, 0x99, 0x99),
    };
    let name = match name {
        NamedColor::LightForeground | NamedColor::DimForeground => NamedColor::Foreground,
        other => other,
    };
    (name, rgb)
}

pub(crate) fn resolve_ansi_color(color: AnsiColor, colors: &TermColors) -> u32 {
    match color {
        AnsiColor::Spec(rgb) => rgb_u32(rgb.r, rgb.g, rgb.b),
        AnsiColor::Indexed(i) => {
            if let Some(a) = &colors[i as usize] {
                arr_to_u32(*a)
            } else {
                let (r, g, b) = indexed_color(i);
                rgb_u32(r, g, b)
            }
        }
        AnsiColor::Named(n) => {
            let (name, (r, g, b)) = named_default(n);
            resolve_named(colors, name, r, g, b)
        }
    }
}

pub(crate) fn arr_to_u32(a: [f32; 4]) -> u32 {
    let r = (a[0].clamp(0.0, 1.0) * 255.0) as u8;
    let g = (a[1].clamp(0.0, 1.0) * 255.0) as u8;
    let b = (a[2].clamp(0.0, 1.0) * 255.0) as u8;
    rgb_u32(r, g, b)
}

pub(crate) fn rgb_u32(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

fn pixel_index(bw: usize, x: usize, y: usize) -> Option<usize> {
    y.checked_mul(bw)?.checked_add(x)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn fill_rect(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    color: u32,
) {
    let x2 = x.saturating_add(w).min(bw);
    let y2 = y.saturating_add(h).min(bh);
    for yy in y.min(bh)..y2 {
        for xx in x.min(bw)..x2 {
            if let Some(idx) = pixel_index(bw, xx, yy) {
                if let Some(px) = buf.get_mut(idx) {
                    *px = color;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn fill_rect_alpha(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    color: u32,
    a: u8,
) {
    let x2 = x.saturating_add(w).min(bw);
    let y2 = y.saturating_add(h).min(bh);
    let a = a as u32;
    let ia = 255 - a;
    for yy in y.min(bh)..y2 {
        for xx in x.min(bw)..x2 {
            let Some(idx) = pixel_index(bw, xx, yy) else {
                continue;
            };
            if let Some(px) = buf.get_mut(idx) {
                *px = blend(*px, color, a, ia);
            }
        }
    }
}

pub(crate) fn blend(dst: u32, src: u32, a: u32, ia: u32) -> u32 {
    let dr = (dst >> 16) & 0xff;
    let dg = (dst >> 8) & 0xff;
    let db = dst & 0xff;
    let sr = (src >> 16) & 0xff;
    let sg = (src >> 8) & 0xff;
    let sb = src & 0xff;
    let r = (sr * a + dr * ia) / 255;
    let g = (sg * a + dg * ia) / 255;
    let b = (sb * a + db * ia) / 255;
    (r << 16) | (g << 8) | b
}

/// Blit one atlas cell sprite into a softbuffer framebuffer (0x00RRGGBB).
///
/// The gray atlas is one coverage byte per pixel; the color atlas holds
/// premultiplied ARGB8888 bytes when `PixelFormat::Argb` is selected.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blit_atlas(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    atlas: &term_atlas::Atlas,
    rect: term_atlas::GlyphRect,
    x0: usize,
    y0: usize,
    color: u32,
) {
    blit_atlas_impl(buf, bw, bh, atlas, rect, x0, y0, color, 0.0);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn blit_atlas_sheared(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    atlas: &term_atlas::Atlas,
    rect: term_atlas::GlyphRect,
    x0: usize,
    y0: usize,
    color: u32,
    shear: f32,
) {
    blit_atlas_impl(buf, bw, bh, atlas, rect, x0, y0, color, shear);
}

#[allow(clippy::too_many_arguments)]
fn blit_atlas_impl(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    atlas: &term_atlas::Atlas,
    rect: term_atlas::GlyphRect,
    x0: usize,
    y0: usize,
    color: u32,
    shear: f32,
) {
    let gray_stride = term_atlas::ATLAS_SIZE as usize;
    let color_stride = term_atlas::COLOR_ATLAS_SIZE as usize;
    let gray = atlas.pixels();
    let color_pixels = atlas.color_pixels();
    for y in 0..rect.h as usize {
        let py = y0 + y;
        if py >= bh {
            continue;
        }
        let shift = (y as f32 * shear).round() as i32;
        for x in 0..rect.w as usize {
            let px = x0 as i32 + x as i32 + shift;
            if px < 0 || px >= bw as i32 {
                continue;
            }
            let Some(idx) = pixel_index(bw, px as usize, py) else {
                continue;
            };
            let Some(dst) = buf.get_mut(idx) else {
                continue;
            };
            if rect.atlas == 0 {
                let src_idx = (rect.y as usize + y) * gray_stride + rect.x as usize + x;
                let Some(&alpha) = gray.get(src_idx) else {
                    continue;
                };
                if alpha != 0 {
                    *dst = blend(*dst, color, alpha as u32, (255 - alpha) as u32);
                }
            } else if let Some(pixels) = color_pixels {
                let src_idx = (rect.y as usize + y) * color_stride + rect.x as usize + x;
                let offset = src_idx * 4;
                let Some(bytes) = pixels.get(offset..offset + 4) else {
                    continue;
                };
                let a = bytes[3] as u32;
                if a == 0 {
                    continue;
                }
                let src = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0]);
                let ia = 255 - a;
                let channel = |shift: u32| -> u32 {
                    let s = (src >> shift) & 0xff;
                    let d = ((*dst >> shift) & 0xff) * ia / 255;
                    (s + d).min(255)
                };
                *dst = (channel(16) << 16) | (channel(8) << 8) | channel(0);
            }
        }
    }
}
