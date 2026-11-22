use super::AppState;
use crate::draw::{
    blend, blit_atlas, blit_atlas_sheared, fill_rect, fill_rect_alpha, resolve_ansi_color,
    resolve_named, rgb_u32,
};
use rio_vt::ansi::CursorShape;
use rio_vt::config::colors::NamedColor;
use rio_vt::crosswords::grid::Dimensions;
use rio_vt::crosswords::pos::{Column, Line, Pos};
use rio_vt::crosswords::square::Wide;
use rio_vt::crosswords::style::StyleFlags;
use std::collections::HashMap;
use std::num::NonZeroU32;
use term_atlas::GlyphRect;

pub(crate) struct CellRender {
    ch: char,
    /// Base code point plus combining marks / variation selectors, only when present.
    cluster: Option<String>,
    fg: u32,
    bg: u32,
    /// 1 for a normal glyph, 2 for a wide glyph's leading cell, 0 for a spacer.
    cells: u8,
    flags: StyleFlags,
}

impl AppState {
    pub(crate) fn render(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        if self.atlas.needs_reset() {
            self.atlas.reclaim();
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

        // Reuse the frame buffers; clearing keeps their allocated capacity.
        let mut rows = std::mem::take(&mut self.rows);
        let mut cells = std::mem::take(&mut self.cells);
        cells.clear();

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
            display_offset,
        ) = {
            let Some(tab) = self.tabs.get(self.active) else {
                return;
            };
            let mut term = tab.terminal.lock();
            term.fill_visible_rows(&mut rows);
            let ncols = term.grid.columns();
            let nrows = rows.len();
            // rio-vt's cursor() handles hidden cursors, scrollback and spacers.
            let cursor = term.cursor();
            let show = cursor.is_visible();
            let cursor_row = cursor.pos.row.0 - term.display_offset() as i32;
            let cursor_col = cursor.pos.col.0;
            let shape = cursor.content;
            let default_fg = resolve_named(&term.colors, NamedColor::Foreground, 0xd8, 0xd8, 0xd8);
            let default_bg = resolve_named(&term.colors, NamedColor::Background, 0x10, 0x10, 0x10);
            let cur_u = resolve_named(&term.colors, NamedColor::Cursor, 0xf0, 0xf0, 0xf0);
            let selection = term.selection.as_ref().and_then(|s| s.to_range(&term));
            let display_offset = term.display_offset();

            for (r, row) in rows.iter().enumerate() {
                for col in 0..ncols {
                    let Some(square) = row.inner.get(col) else {
                        cells.push(CellRender {
                            ch: '\0',
                            cluster: None,
                            fg: default_fg,
                            bg: default_bg,
                            cells: 1,
                            flags: StyleFlags::empty(),
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
                    let cells_wide = match square.wide() {
                        Wide::Wide => 2,
                        Wide::Spacer | Wide::LeadingSpacer => 0,
                        Wide::Narrow => 1,
                    };
                    // Only allocate a cluster string when the cell actually
                    // has zero-width extras to composite.
                    let cluster = if square.extras_id_checked().is_some() {
                        Some(
                            term.grid
                                .cell_text(Pos::new(
                                    Line(r as i32 - display_offset as i32),
                                    Column(col),
                                ))
                                .collect(),
                        )
                    } else {
                        None
                    };
                    cells.push(CellRender {
                        ch: square.c(),
                        cluster,
                        fg,
                        bg,
                        cells: cells_wide,
                        flags: style.flags,
                    });
                }
            }
            term.reset_damage();
            term.damage_event_in_flight = false;
            (
                cells,
                ncols,
                nrows,
                cursor_row,
                cursor_col,
                show,
                shape,
                default_bg,
                cur_u,
                selection,
                display_offset,
            )
        };
        self.rows = rows;

        let tab_layouts = self.tab_layouts(width as usize);
        // Rasterize every glyph before borrowing the framebuffer from self.
        let mut glyphs: HashMap<(char, u8), GlyphRect> = HashMap::new();
        let mut need = |ch: char, cells: u8| {
            if let Some(rect) = self.atlas.ensure(ch, cells) {
                glyphs.insert((ch, cells), rect);
            }
        };
        need('×', 1);
        for (label, _, _, _) in &tab_layouts {
            for ch in label.chars() {
                need(ch, 1);
            }
        }
        if let Some((_, text)) = &self.resize_overlay {
            for ch in text.chars() {
                need(ch, 1);
            }
        }
        let key_help: Vec<(&str, &str)> = if self.key_help {
            crate::keybindings::bindings()
                .map(|binding| (binding.label, binding.description))
                .collect()
        } else {
            Vec::new()
        };
        for (keys, action) in &key_help {
            for ch in keys.chars().chain(action.chars()) {
                need(ch, 1);
            }
        }
        drop(need);
        let mut cell_rects = std::mem::take(&mut self.cell_rects);
        cell_rects.clear();
        cell_rects.extend(cells.iter().map(|cell| {
            if cell.cells == 0 || cell.ch == '\0' || cell.ch == ' ' {
                None
            } else if let Some(text) = &cell.cluster {
                self.atlas.ensure_str(text, cell.cells)
            } else {
                self.atlas.ensure(cell.ch, cell.cells)
            }
        }));

        let mut buffer = match self.surface.buffer_mut() {
            Ok(b) => b,
            Err(_) => {
                self.cells = cells;
                self.cell_rects = cell_rects;
                return;
            }
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
                if let Some(&rect) = glyphs.get(&(ch, 1)) {
                    blit_atlas(
                        &mut buffer,
                        bw,
                        bh,
                        &self.atlas,
                        rect,
                        cx.max(0) as usize,
                        1,
                        fg,
                    );
                }
                cx += self.cell_w as i32;
            }
            // close button "×"
            if let Some(cx0) = close_x0 {
                if let Some(&rect) = glyphs.get(&('×', 1)) {
                    blit_atlas(
                        &mut buffer,
                        bw,
                        bh,
                        &self.atlas,
                        rect,
                        *cx0,
                        1,
                        if is_active { active_fg } else { close_fg },
                    );
                }
            }
        }

        let y_origin = bar_h;
        // Paint all backgrounds first: a wide glyph from one cell may spill
        // into the next, whose background must not cover that ink.
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
                let selected = selection.is_some_and(|sel| {
                    sel.contains(Pos::new(Line(r as i32 - display_offset as i32), Column(c)))
                });
                let bg = if selected { cell.fg } else { cell.bg };
                if bg != default_bg || selected {
                    fill_rect(&mut buffer, bw, bh, px, py, self.cell_w, self.cell_h, bg);
                }
            }
        }

        // Then glyphs and cursor, on top of both cells' backgrounds.
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

                let selected = selection.is_some_and(|sel| {
                    sel.contains(Pos::new(Line(r as i32 - display_offset as i32), Column(c)))
                });
                let mut fg = if selected { cell.bg } else { cell.fg };
                if cell.flags.contains(StyleFlags::DIM) {
                    fg = blend(cell.bg, fg, 128, 127);
                }
                let width = self.cell_w * cell.cells.max(1) as usize;
                if !cell.flags.contains(StyleFlags::HIDDEN) {
                    if let Some(Some(rect)) = cell_rects.get(idx) {
                        if cell.flags.contains(StyleFlags::BOLD) && self.cell_w > 1 {
                            blit_atlas(&mut buffer, bw, bh, &self.atlas, *rect, px, py, fg);
                            blit_atlas(&mut buffer, bw, bh, &self.atlas, *rect, px + 1, py, fg);
                        } else if cell.flags.contains(StyleFlags::ITALIC) {
                            blit_atlas_sheared(
                                &mut buffer,
                                bw,
                                bh,
                                &self.atlas,
                                *rect,
                                px,
                                py,
                                fg,
                                -0.25,
                            );
                        } else {
                            blit_atlas(&mut buffer, bw, bh, &self.atlas, *rect, px, py, fg);
                        }
                    }
                    if cell.flags.contains(StyleFlags::STRIKEOUT) {
                        fill_rect(&mut buffer, bw, bh, px, py + self.cell_h / 2, width, 1, fg);
                    }
                    let uy = py + self.cell_h.saturating_sub(2);
                    if cell.flags.contains(StyleFlags::UNDERLINE) {
                        fill_rect(&mut buffer, bw, bh, px, uy, width, 1, fg);
                    } else if cell.flags.contains(StyleFlags::DOUBLE_UNDERLINE) {
                        fill_rect(&mut buffer, bw, bh, px, uy.saturating_sub(2), width, 1, fg);
                        fill_rect(&mut buffer, bw, bh, px, uy, width, 1, fg);
                    } else if cell.flags.contains(StyleFlags::UNDERCURL) {
                        for x in 0..width {
                            let y = uy - usize::from(x % 4 == 1 || x % 4 == 2);
                            fill_rect(&mut buffer, bw, bh, px + x, y, 1, 1, fg);
                        }
                    } else if cell.flags.contains(StyleFlags::DOTTED_UNDERLINE) {
                        for x in (0..width).step_by(2) {
                            fill_rect(&mut buffer, bw, bh, px + x, uy, 1, 1, fg);
                        }
                    } else if cell.flags.contains(StyleFlags::DASHED_UNDERLINE) {
                        for x in (0..width).step_by(4) {
                            fill_rect(&mut buffer, bw, bh, px + x, uy, 2.min(width - x), 1, fg);
                        }
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
            let mut cx = x0 + 3;
            for ch in text.chars() {
                if let Some(&rect) = glyphs.get(&(ch, 1)) {
                    blit_atlas(
                        &mut buffer,
                        bw,
                        bh,
                        &self.atlas,
                        rect,
                        cx,
                        y0 + 2,
                        rgb_u32(0xf0, 0xf4, 0xfa),
                    );
                }
                cx += self.cell_w;
            }
        }

        if !key_help.is_empty() {
            let keys_w = key_help
                .iter()
                .map(|(keys, _)| keys.chars().count())
                .max()
                .unwrap_or(0);
            let action_w = key_help
                .iter()
                .map(|(_, action)| action.chars().count())
                .max()
                .unwrap_or(0);
            let box_w = ((keys_w + 2 + action_w + 4) * self.cell_w).min(bw.saturating_sub(4));
            let box_h = ((key_help.len() + 3) * self.cell_h).min(bh.saturating_sub(bar_h + 4));
            let x0 = bw.saturating_sub(box_w) / 2;
            let y0 = bar_h + (bh.saturating_sub(bar_h + box_h)) / 2;
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
            for (row, (keys, action)) in key_help.iter().enumerate() {
                let ty = y0 + 2 + row * self.cell_h;
                if ty + self.cell_h > y0 + box_h {
                    break;
                }
                let mut cx = x0 + 2;
                for ch in keys.chars() {
                    if let Some(&rect) = glyphs.get(&(ch, 1)) {
                        blit_atlas(
                            &mut buffer,
                            bw,
                            bh,
                            &self.atlas,
                            rect,
                            cx,
                            ty,
                            rgb_u32(0x8b, 0xc5, 0xff),
                        );
                    }
                    cx += self.cell_w;
                }
                cx = x0 + 2 + (keys_w + 2) * self.cell_w;
                for ch in action.chars() {
                    if let Some(&rect) = glyphs.get(&(ch, 1)) {
                        blit_atlas(
                            &mut buffer,
                            bw,
                            bh,
                            &self.atlas,
                            rect,
                            cx,
                            ty,
                            rgb_u32(0xe6, 0xed, 0xf5),
                        );
                    }
                    cx += self.cell_w;
                }
            }
        }

        self.cells = cells;
        self.cell_rects = cell_rects;
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
    fn atlas() -> term_atlas::Atlas {
        term_atlas::Atlas::with_face_format(
            term_atlas::Face::bundled(),
            10,
            18,
            term_atlas::PixelFormat::Argb,
        )
    }

    #[test]
    fn atlas_draws_latin_cyrillic_and_fallback_cjk() {
        let mut atlas = atlas();
        for ch in ['A', 'Z', 'ї', 'є', 'і', 'ґ', 'ш', 'Є'] {
            assert!(atlas.ensure(ch, 1).is_some(), "{ch} has no glyph");
        }
        for ch in ['日', '码'] {
            assert!(atlas.ensure(ch, 2).is_some(), "{ch} has no fallback glyph");
        }
    }

    #[test]
    fn emoji_glyph_blits_into_the_framebuffer() {
        let mut atlas = atlas();
        let rect = atlas.ensure('😀', 2).expect("emoji fallback");
        let mut framebuffer = vec![0u32; 40 * 18];
        crate::draw::blit_atlas(&mut framebuffer, 40, 18, &atlas, rect, 0, 0, 0x00ffffff);
        assert!(
            framebuffer.iter().any(|&pixel| pixel != 0),
            "emoji produced no pixels"
        );
    }

    #[test]
    fn flag_cluster_is_rasterized() {
        let mut atlas = atlas();
        let rect = atlas.ensure_str("🇺🇸", 2).expect("flag cluster");
        assert!(rect.w > 0 && rect.h > 0);
    }

    #[test]
    fn wide_glyph_occupies_two_cells() {
        let mut atlas = atlas();
        let rect = atlas.ensure('日', 2).expect("wide glyph");
        assert_eq!(rect.w, 20);
        assert_eq!(rect.h, 18);
    }
}
