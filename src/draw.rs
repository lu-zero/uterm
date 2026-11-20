//! Color resolution and pixel-blitting helpers.

use rio_vt::config::colors::term::TermColors;
use rio_vt::config::colors::{AnsiColor, NamedColor};
use rio_vt::selection::SelectionRange;

pub(crate) fn is_selected(selection: Option<SelectionRange>, row: i32, col: usize) -> bool {
    let Some(sel) = selection else {
        return false;
    };
    if sel.is_block {
        let c0 = sel.start.col.0.min(sel.end.col.0);
        let c1 = sel.start.col.0.max(sel.end.col.0);
        row >= sel.start.row.0 && row <= sel.end.row.0 && col >= c0 && col <= c1
    } else {
        if row < sel.start.row.0 || row > sel.end.row.0 {
            return false;
        }
        if sel.start.row.0 == sel.end.row.0 {
            col >= sel.start.col.0 && col <= sel.end.col.0
        } else if row == sel.start.row.0 {
            col >= sel.start.col.0
        } else if row == sel.end.row.0 {
            col <= sel.end.col.0
        } else {
            true
        }
    }
}

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

pub(crate) fn resolve_ansi_color(color: AnsiColor, colors: &TermColors, _default: u32) -> u32 {
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
        AnsiColor::Named(n) => match n {
            NamedColor::Foreground | NamedColor::LightForeground | NamedColor::DimForeground => {
                resolve_named(colors, NamedColor::Foreground, 0xd8, 0xd8, 0xd8)
            }
            NamedColor::Background => {
                resolve_named(colors, NamedColor::Background, 0x10, 0x10, 0x10)
            }
            NamedColor::Cursor => resolve_named(colors, NamedColor::Cursor, 0xf0, 0xf0, 0xf0),
            NamedColor::Black => colors[NamedColor::Black]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0, 0, 0)),
            NamedColor::Red => colors[NamedColor::Red]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0xcd, 0, 0)),
            NamedColor::Green => colors[NamedColor::Green]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0, 0xcd, 0)),
            NamedColor::Yellow => colors[NamedColor::Yellow]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0xcd, 0xcd, 0)),
            NamedColor::Blue => colors[NamedColor::Blue]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0, 0, 0xee)),
            NamedColor::Magenta => colors[NamedColor::Magenta]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0xcd, 0, 0xcd)),
            NamedColor::Cyan => colors[NamedColor::Cyan]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0, 0xcd, 0xcd)),
            NamedColor::White => colors[NamedColor::White]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0xe5, 0xe5, 0xe5)),
            NamedColor::LightBlack => colors[NamedColor::LightBlack]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0x7f, 0x7f, 0x7f)),
            NamedColor::LightRed => colors[NamedColor::LightRed]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0xff, 0, 0)),
            NamedColor::LightGreen => colors[NamedColor::LightGreen]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0, 0xff, 0)),
            NamedColor::LightYellow => colors[NamedColor::LightYellow]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0xff, 0xff, 0)),
            NamedColor::LightBlue => colors[NamedColor::LightBlue]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0x5c, 0x5c, 0xff)),
            NamedColor::LightMagenta => colors[NamedColor::LightMagenta]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0xff, 0, 0xff)),
            NamedColor::LightCyan => colors[NamedColor::LightCyan]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0, 0xff, 0xff)),
            NamedColor::LightWhite => colors[NamedColor::LightWhite]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0xff, 0xff, 0xff)),
            NamedColor::DimBlack => colors[NamedColor::DimBlack]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0, 0, 0)),
            NamedColor::DimRed => colors[NamedColor::DimRed]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0x88, 0, 0)),
            NamedColor::DimGreen => colors[NamedColor::DimGreen]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0, 0x88, 0)),
            NamedColor::DimYellow => colors[NamedColor::DimYellow]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0x88, 0x88, 0)),
            NamedColor::DimBlue => colors[NamedColor::DimBlue]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0, 0, 0x88)),
            NamedColor::DimMagenta => colors[NamedColor::DimMagenta]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0x88, 0, 0x88)),
            NamedColor::DimCyan => colors[NamedColor::DimCyan]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0, 0x88, 0x88)),
            NamedColor::DimWhite => colors[NamedColor::DimWhite]
                .as_ref()
                .copied()
                .map(arr_to_u32)
                .unwrap_or(rgb_u32(0x99, 0x99, 0x99)),
        },
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

pub(crate) fn blit_glyph(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    x0: i32,
    y0: i32,
    metrics: &fontdue::Metrics,
    bitmap: &[u8],
    color: u32,
) {
    for y in 0..metrics.height {
        let py = y0 + y as i32;
        if py < 0 || py >= bh as i32 {
            continue;
        }
        for x in 0..metrics.width {
            let px = x0 + x as i32;
            if px < 0 || px >= bw as i32 {
                continue;
            }
            let Some(offset) = y
                .checked_mul(metrics.width)
                .and_then(|off| off.checked_add(x))
            else {
                continue;
            };
            let Some(alpha) = bitmap.get(offset).map(|sample| u32::from(*sample)) else {
                continue;
            };
            if alpha == 0 {
                continue;
            }
            let Some(idx) = pixel_index(bw, px as usize, py as usize) else {
                continue;
            };
            let ia = 255 - alpha;
            if let Some(pixel) = buf.get_mut(idx) {
                *pixel = blend(*pixel, color, alpha, ia);
            }
        }
    }
}
