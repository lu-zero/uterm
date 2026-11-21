use super::AppState;
use crate::draw::{
    blit_glyph, fill_rect, fill_rect_alpha, is_selected, resolve_ansi_color, resolve_named, rgb_u32,
};
use fontdue::Font;
use rio_vt::ansi::CursorShape;
use rio_vt::config::colors::NamedColor;
use rio_vt::crosswords::grid::Dimensions;
use rio_vt::crosswords::Mode;
use std::num::NonZeroU32;

struct CellRender {
    ch: char,
    fg: u32,
    bg: u32,
}

/// First face that has `ch` and paints ink. The index is that face in `fonts`.
pub(crate) fn rasterize_from(
    fonts: &[Font],
    px: f32,
    ch: char,
) -> Option<(usize, fontdue::Metrics, Vec<u8>)> {
    let (index, font) = fonts
        .iter()
        .enumerate()
        .find(|(_, font)| font.has_glyph(ch))?;
    let (metrics, bitmap) = font.rasterize(ch, px);
    if metrics.advance_width > 0.0 && metrics.height > 0 && bitmap.iter().any(|&a| a > 0) {
        Some((index, metrics, bitmap))
    } else {
        None
    }
}

impl AppState {
    fn rasterize_char(&self, ch: char) -> Option<(fontdue::Metrics, Vec<u8>)> {
        let (_face, metrics, bitmap) = rasterize_from(&self.fonts, self.font_px, ch)?;
        Some((metrics, bitmap))
    }

    pub(crate) fn ensure_glyph(&mut self, ch: char) {
        if self.glyph_cache.contains_key(&ch) {
            return;
        }
        if let Some(raster) = self.rasterize_char(ch) {
            self.glyph_cache.insert(ch, raster);
        }
    }

    pub(crate) fn render(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        self.clamp_active();
        let size = self.window.inner_size();
        let (width, height) = (size.width, size.height);
        if width == 0 || height == 0 {
            return;
        }
        let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
            return;
        };
        if self.surface.resize(w, h).is_err() {
            return;
        }

        let (
            cells,
            ncols,
            nrows,
            cursor_row,
            cursor_col,
            show_cursor,
            cursor_shape,
            default_bg,
            cur_u,
            selection,
        ) = {
            let Some(tab) = self.tabs.get(self.active) else {
                return;
            };
            let mut term = tab.terminal.lock();
            let rows_vec: Vec<_> = term.visible_rows();
            let ncols = term.grid.columns();
            let nrows = rows_vec.len();
            let c = term.grid.cursor.pos;
            let show = term.mode().contains(Mode::SHOW_CURSOR);
            let shape = term.cursor_shape;
            let default_fg = resolve_named(&term.colors, NamedColor::Foreground, 0xd8, 0xd8, 0xd8);
            let default_bg = resolve_named(&term.colors, NamedColor::Background, 0x10, 0x10, 0x10);
            let cur_u = resolve_named(&term.colors, NamedColor::Cursor, 0xf0, 0xf0, 0xf0);
            let selection = term.selection.as_ref().and_then(|s| s.to_range(&term));

            let mut cells = Vec::new();
            if let Some(n) = nrows.checked_mul(ncols) {
                let _ = cells.try_reserve(n);
            }
            for row in &rows_vec {
                for col in 0..ncols {
                    let Some(square) = row.inner.get(col) else {
                        cells.push(CellRender {
                            ch: '\0',
                            fg: default_fg,
                            bg: default_bg,
                        });
                        continue;
                    };
                    let style = term.grid.style_of(square);
                    let mut fg = resolve_ansi_color(style.fg, &term.colors, default_fg);
                    let mut bg = resolve_ansi_color(style.bg, &term.colors, default_bg);
                    if style
                        .flags
                        .contains(rio_vt::crosswords::style::StyleFlags::INVERSE)
                    {
                        std::mem::swap(&mut fg, &mut bg);
                    }
                    cells.push(CellRender {
                        ch: square.c(),
                        fg,
                        bg,
                    });
                }
            }
            term.reset_damage();
            term.damage_event_in_flight = false;
            (
                cells, ncols, nrows, c.row.0, c.col.0, show, shape, default_bg, cur_u, selection,
            )
        };

        let tab_layouts = self.tab_layouts(width as usize);
        self.ensure_glyph('×');
        for (label, _, _, _) in &tab_layouts {
            for ch in label.chars() {
                self.ensure_glyph(ch);
            }
        }
        if let Some((_, text)) = self.resize_overlay.clone() {
            for ch in text.chars() {
                self.ensure_glyph(ch);
            }
        }
        for cell in &cells {
            if cell.ch != '\0' && cell.ch != ' ' {
                self.ensure_glyph(cell.ch);
            }
        }

        let mut buffer = match self.surface.buffer_mut() {
            Ok(b) => b,
            Err(_) => return,
        };
        buffer.fill(default_bg);

        // Draw into the buffer's own size so a failed or stale resize cannot
        // index past the allocation.
        let bw = buffer.width().get() as usize;
        let bh = buffer.height().get() as usize;
        let bar_h = self.cell_h.min(bh);

        let bar_bg = rgb_u32(0x1f, 0x23, 0x29);
        let bar_border = rgb_u32(0x33, 0x3a, 0x44);
        let inactive_bg = rgb_u32(0x27, 0x2c, 0x34);
        let inactive_fg = rgb_u32(0xb8, 0xc0, 0xcc);
        let active_bg = rgb_u32(0x2f, 0x5f, 0x8f);
        let active_fg = rgb_u32(0xff, 0xff, 0xff);
        let close_fg = rgb_u32(0xd0, 0xd6, 0xde);
        fill_rect(&mut buffer, bw, bh, 0, 0, bw, bar_h, bar_bg);
        if bar_h >= 1 {
            fill_rect(
                &mut buffer,
                bw,
                bh,
                0,
                bar_h.saturating_sub(1),
                bw,
                1,
                bar_border,
            );
        }

        for (i, (label, x0, w, close_x0)) in tab_layouts.iter().enumerate() {
            let is_active = i == self.active;
            let (fg, bgc) = if is_active {
                (active_fg, active_bg)
            } else {
                (inactive_fg, inactive_bg)
            };
            fill_rect(
                &mut buffer,
                bw,
                bh,
                *x0,
                1,
                *w,
                bar_h.saturating_sub(2),
                bgc,
            );
            if is_active {
                fill_rect(
                    &mut buffer,
                    bw,
                    bh,
                    *x0,
                    0,
                    *w,
                    2.min(bar_h),
                    rgb_u32(0x6b, 0xb6, 0xff),
                );
            }
            // tab text
            let mut cx = (*x0 as i32) + (self.cell_w as i32 / 2);
            let max_x = (*x0 + *w).saturating_sub(self.cell_w) as i32;
            for ch in label.chars() {
                if cx >= max_x {
                    break;
                }
                if let Some((metrics, bitmap)) = self.glyph_cache.get(&ch) {
                    let gx = cx + metrics.xmin;
                    let gy = self.ascent as i32 - metrics.ymin - metrics.height as i32;
                    blit_glyph(&mut buffer, bw, bh, gx, gy, metrics, bitmap, fg);
                    cx += metrics.advance_width.ceil() as i32;
                } else {
                    cx += self.cell_w as i32;
                }
            }
            // close button "×"
            if let Some(cx0) = close_x0 {
                if let Some((metrics, bitmap)) = self.glyph_cache.get(&'×') {
                    let gx = (*cx0 as i32) + metrics.xmin;
                    let gy = self.ascent as i32 - metrics.ymin - metrics.height as i32;
                    blit_glyph(
                        &mut buffer,
                        bw,
                        bh,
                        gx,
                        gy,
                        metrics,
                        bitmap,
                        if is_active { active_fg } else { close_fg },
                    );
                }
            }
        }

        let y_origin = bar_h;
        for r in 0..nrows {
            let py = y_origin + r * self.cell_h;
            if py >= bh {
                break;
            }
            for c in 0..ncols {
                let Some(idx) = r.checked_mul(ncols).and_then(|i| i.checked_add(c)) else {
                    continue;
                };
                let Some(cell) = cells.get(idx) else {
                    continue;
                };
                let px = c * self.cell_w;
                if px >= bw {
                    break;
                }

                let mut fg = cell.fg;
                let mut bg = cell.bg;
                if is_selected(selection, r as i32, c) {
                    std::mem::swap(&mut fg, &mut bg);
                }

                if bg != default_bg || is_selected(selection, r as i32, c) {
                    fill_rect(&mut buffer, bw, bh, px, py, self.cell_w, self.cell_h, bg);
                }

                let ch = cell.ch;
                if ch != '\0' && ch != ' ' {
                    if let Some((metrics, bitmap)) = self.glyph_cache.get(&ch) {
                        let gx = px as i32 + metrics.xmin;
                        let gy =
                            py as i32 + self.ascent as i32 - metrics.ymin - metrics.height as i32;
                        blit_glyph(&mut buffer, bw, bh, gx, gy, metrics, bitmap, fg);
                    }
                }

                if show_cursor && cursor_row == r as i32 && cursor_col == c {
                    match cursor_shape {
                        CursorShape::Beam => {
                            fill_rect(&mut buffer, bw, bh, px, py, 2, self.cell_h, cur_u)
                        }
                        CursorShape::Underline => {
                            let uy = py + self.cell_h.saturating_sub(2);
                            fill_rect(&mut buffer, bw, bh, px, uy, self.cell_w, 2, cur_u);
                        }
                        _ => fill_rect_alpha(
                            &mut buffer,
                            bw,
                            bh,
                            px,
                            py,
                            self.cell_w,
                            self.cell_h,
                            cur_u,
                            110,
                        ),
                    }
                }
            }
        }

        // Transient "cols x rows" overlay shown after a resize.
        if let Some((_, text)) = &self.resize_overlay {
            let n = text.chars().count();
            let box_w = (n * self.cell_w + 6).min(bw);
            let box_h = (self.cell_h + 4).min(bh);
            let x0 = bw.saturating_sub(box_w + 6);
            let y0 = bh.saturating_sub(box_h + 6).max(bar_h);
            fill_rect(
                &mut buffer,
                bw,
                bh,
                x0,
                y0,
                box_w,
                box_h,
                rgb_u32(0x14, 0x18, 0x1f),
            );
            fill_rect(
                &mut buffer,
                bw,
                bh,
                x0,
                y0,
                box_w,
                1,
                rgb_u32(0x6b, 0xb6, 0xff),
            );
            let mut cx = (x0 + 3) as i32;
            for ch in text.chars() {
                if let Some((metrics, bitmap)) = self.glyph_cache.get(&ch) {
                    let gx = cx + metrics.xmin;
                    let gy =
                        y0 as i32 + 2 + self.ascent as i32 - metrics.ymin - metrics.height as i32;
                    blit_glyph(
                        &mut buffer,
                        bw,
                        bh,
                        gx,
                        gy,
                        metrics,
                        bitmap,
                        rgb_u32(0xf0, 0xf4, 0xfa),
                    );
                    cx += metrics.advance_width.ceil() as i32;
                } else {
                    cx += self.cell_w as i32;
                }
            }
        }

        let _ = buffer.present();
    }

    pub(crate) fn tab_label_text(&self, idx: usize) -> String {
        let title = self
            .tabs
            .get(idx)
            .map(|t| t.title.as_str())
            .unwrap_or("shell");
        let mut t = title.trim().replace(['\n', '\r', '\t'], " ");
        if t.is_empty() {
            t = "shell".to_string();
        }
        // truncate to keep tabs reasonable
        let max_chars = 18usize;
        if t.chars().count() > max_chars {
            t = t
                .chars()
                .take(max_chars.saturating_sub(1))
                .collect::<String>()
                + ".";
        }
        format!(" {} ", t)
    }

    pub(crate) fn tab_layouts(&self, bw: usize) -> Vec<(String, usize, usize, Option<usize>)> {
        let mut layouts = Vec::new();
        let mut xoff = 4usize;
        let gap = 4usize;
        let close_pad = self.cell_w.max(8);
        for i in 0..self.tabs.len() {
            let label = self.tab_label_text(i);
            let text_w = label.chars().count() * self.cell_w;
            let w =
                (text_w + close_pad + self.cell_w / 2).min(bw.saturating_sub(xoff + gap).max(1));
            let close_x0 = if xoff + w > self.cell_w {
                Some(xoff + w.saturating_sub(close_pad))
            } else {
                None
            };
            layouts.push((label, xoff, w, close_x0));
            xoff = xoff.saturating_add(w + gap);
            if xoff >= bw {
                break;
            }
        }
        layouts
    }

    pub(crate) fn tab_bar_hit(&self, x: f64, y: f64) -> Option<(usize, bool)> {
        let bar_h = self.cell_h as f64;
        if y < 0.0 || y >= bar_h {
            return None;
        }
        let layouts = self.tab_layouts(self.window.inner_size().width as usize);
        for (i, (_, x0, w, close_x0)) in layouts.iter().enumerate() {
            let start = *x0 as f64;
            let end = (*x0 + *w) as f64;
            if x >= start && x < end {
                if let Some(cx0) = close_x0 {
                    let cstart = *cx0 as f64;
                    if x >= cstart {
                        return Some((i, true));
                    }
                }
                return Some((i, false));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::rasterize_from;
    use crate::config::bundled_font;
    use crate::draw::blit_glyph;
    use fontdue::{Font, Metrics};

    fn cell_width(font: &Font, px: f32) -> (usize, f32) {
        let (metrics, _) = font.rasterize('M', px);
        let width = metrics.advance_width.ceil().max(1.0) as usize;
        (width, metrics.advance_width)
    }

    #[test]
    fn cascadia_paints_latin_and_ukrainian_and_skips_japanese() {
        let font = bundled_font().unwrap();
        let fonts = [font];
        let px = 32.0;
        let (_cell_w, m_advance) = cell_width(&fonts[0], px);
        for ch in ['A', 'Z', 'ї', 'є', 'і', 'ґ', 'ш', 'Є'] {
            let (face, metrics, _) = rasterize_from(&fonts, px, ch).unwrap_or_else(|| {
                panic!("U+{:04X} {ch} has no ink in Cascadia", ch as u32);
            });
            assert_eq!(face, 0);
            assert!(
                (metrics.advance_width - m_advance).abs() < 0.01,
                "{ch} advance {} vs M {m_advance}",
                metrics.advance_width
            );
        }
        assert!(rasterize_from(&fonts, px, '日').is_none());
        assert!(rasterize_from(&fonts, px, 'あ').is_none());
    }

    #[test]
    fn a_wide_bitmap_continues_into_the_next_cell() {
        let font = bundled_font().unwrap();
        let (cell_w, _) = cell_width(&font, 32.0);
        let metrics = Metrics {
            width: cell_w * 3,
            height: cell_w,
            advance_width: (cell_w * 2) as f32,
            ..Metrics::default()
        };
        let bitmap = vec![255u8; metrics.width * metrics.height];
        let (bw, bh) = (cell_w * 5, cell_w);
        let mut buf = vec![0u32; bw * bh];
        // Column 2 starts here; a three-cell-wide glyph spills into columns 3 and 4.
        blit_glyph(
            &mut buf,
            bw,
            bh,
            (cell_w * 2) as i32,
            0,
            &metrics,
            &bitmap,
            0x00ff_ffff,
        );
        let has_ink = |from: usize, to: usize| {
            buf.chunks(bw)
                .any(|row| row[from..to].iter().any(|&p| p != 0))
        };
        assert!(has_ink(cell_w * 2, cell_w * 3), "ink in its own cell");
        assert!(has_ink(cell_w * 3, cell_w * 4), "ink in the next cell");
        assert!(
            has_ink(cell_w * 4, cell_w * 5),
            "ink in the cell after that"
        );
    }
}
