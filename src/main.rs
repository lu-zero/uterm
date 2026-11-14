use fontdue::Font;
use rio_vt::ansi::CursorShape;
use rio_vt::clipboard::{Clipboard, ClipboardType};
use rio_vt::config::colors::term::TermColors;
use rio_vt::config::colors::{AnsiColor, NamedColor};
use rio_vt::crosswords::grid::Dimensions;
use rio_vt::crosswords::pos::{Column, Line, Pos, Side};
use rio_vt::crosswords::{Crosswords, Mode};
use rio_vt::event::sync::FairMutex;
use rio_vt::event::{EventListener, Msg, RioEvent, WindowId, WindowSize};
use rio_vt::performer::Machine;
use rio_vt::selection::{Selection, SelectionRange, SelectionType};
use softbuffer::{Context, Surface};
use std::borrow::Cow;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Modifiers, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::raw_window_handle::HasDisplayHandle;
use winit::window::{Window, WindowId as WinitWindowId};

const CASCADIA_TTF: &[u8] = include_bytes!("../CascadiaCodeNF.ttf");

#[derive(Clone)]
enum UiEvent {
    Wakeup,
    TabExited(usize),
    TitleChanged(usize, String),
    ClipboardStore(ClipboardType, String),
    ClipboardLoad(usize, ClipboardType, Arc<dyn Fn(&str) -> String + Sync + Send + 'static>),
}

#[derive(Clone)]
struct Listener {
    proxy: EventLoopProxy<UiEvent>,
}

impl EventListener for Listener {
    fn send_event(&self, event: RioEvent, _id: WindowId) {
        match event {
            RioEvent::CloseTerminal(route) | RioEvent::ChildExited(route, _) => {
                let _ = self.proxy.send_event(UiEvent::TabExited(route));
            }
            RioEvent::Title(route, title) => {
                let _ = self.proxy.send_event(UiEvent::TitleChanged(route, title));
            }
            RioEvent::ClipboardStore(ty, text) => {
                let _ = self.proxy.send_event(UiEvent::ClipboardStore(ty, text));
            }
            RioEvent::ClipboardLoad(route, ty, formatter) => {
                let _ = self.proxy.send_event(UiEvent::ClipboardLoad(route, ty, formatter));
            }
            RioEvent::ResetTitle | RioEvent::SyncWindowTitle => {
                let _ = self.proxy.send_event(UiEvent::Wakeup);
            }
            _ => {
                let _ = self.proxy.send_event(UiEvent::Wakeup);
            }
        }
    }
}

#[derive(Clone, Copy)]
struct TermSize {
    cols: usize,
    rows: usize,
    cell_w: f32,
    cell_h: f32,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize { self.rows }
    fn screen_lines(&self) -> usize { self.rows }
    fn columns(&self) -> usize { self.cols }
    fn square_width(&self) -> f32 { self.cell_w }
    fn square_height(&self) -> f32 { self.cell_h }
}

struct PtyHandle {
    channel: rio_vt::corcovado::channel::Sender<Msg>,
    _thread: std::thread::JoinHandle<(
        Machine<rio_vt::teletypewriter::Pty, Listener>,
        rio_vt::performer::State,
    )>,
}

struct Tab {
    id: usize,
    title: String,
    terminal: Arc<FairMutex<Crosswords<Listener>>>,
    pty: PtyHandle,
}

impl Tab {
    fn new(id: usize, cols: usize, rows: usize, cell_w: usize, cell_h: usize, listener: Listener) -> Self {
        let terminal = Arc::new(FairMutex::new(Crosswords::new(
            TermSize { cols, rows, cell_w: cell_w as f32, cell_h: cell_h as f32 },
            CursorShape::Block,
            listener.clone(),
            WindowId::from(0),
            id,
            10_000,
        )));
        let cwd = std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned());
        let pty = rio_vt::teletypewriter::create_pty_with_spawn(
            None,
            Vec::new(),
            &cwd,
            None,
            cols as u16,
            rows as u16,
            (cols * cell_w) as u16,
            (rows * cell_h) as u16,
        )
        .expect("create pty");
        let machine = Machine::new(terminal.clone(), pty, listener, WindowId::from(0), id).expect("machine");
        let channel = machine.channel();
        let thread = machine.spawn();
        Tab { id, title: "shell".to_string(), terminal, pty: PtyHandle { channel, _thread: thread } }
    }

    fn resize(&self, cols: usize, rows: usize, cell_w: usize, cell_h: usize) {
        let size = TermSize { cols, rows, cell_w: cell_w as f32, cell_h: cell_h as f32 };
        {
            let mut term = self.terminal.lock();
            term.resize(size);
        }
        let _ = self.pty.channel.send(Msg::Resize(WindowSize {
            rows: rows as u16,
            cols: cols as u16,
            width: (cols * cell_w) as u16,
            height: (rows * cell_h) as u16,
        }));
    }
}

#[derive(Clone, Copy)]
struct CellRender {
    ch: char,
    fg: u32,
    bg: u32,
}

struct AppState {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    tabs: Vec<Tab>,
    active: usize,
    next_tab_id: usize,
    font: Font,
    font_px: f32,
    cell_w: usize,
    cell_h: usize,
    ascent: f32,
    modifiers: Modifiers,
    glyph_cache: HashMap<char, (fontdue::Metrics, Vec<u8>)>,
    proxy: EventLoopProxy<UiEvent>,
    cursor_pos: (f64, f64),
    mouse_buttons: [bool; 3], // left, middle, right
    selecting: bool,
    clipboard: Clipboard,
}

impl AppState {
    fn grid_dims(&self) -> (usize, usize) {
        let sz = self.window.inner_size();
        let cols = (sz.width as usize / self.cell_w.max(1)).max(2);
        let total_rows = (sz.height as usize / self.cell_h.max(1)).max(2);
        let content_rows = total_rows.saturating_sub(1).max(1);
        (cols, content_rows)
    }

    fn resize_all_tabs(&mut self) {
        let (cols, rows) = self.grid_dims();
        for tab in &self.tabs {
            tab.resize(cols, rows, self.cell_w, self.cell_h);
        }
    }

    fn new_tab(&mut self) {
        let (cols, rows) = self.grid_dims();
        let listener = Listener { proxy: self.proxy.clone() };
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        self.tabs.push(Tab::new(id, cols, rows, self.cell_w, self.cell_h, listener));
        self.active = self.tabs.len() - 1;
        self.sync_window_title();
    }

    fn sync_window_title(&self) {
        if let Some(tab) = self.tabs.get(self.active) {
            let title = if tab.title.trim().is_empty() { "uterm".to_string() } else { format!("{} — uterm", tab.title.trim()) };
            self.window.set_title(&title);
        } else {
            self.window.set_title("uterm");
        }
    }

    fn close_active_tab(&mut self, event_loop: &ActiveEventLoop) {
        if self.tabs.is_empty() { return; }
        self.active = self.active.min(self.tabs.len() - 1);
        self.tabs.remove(self.active);
        if self.tabs.is_empty() {
            self.active = 0;
            event_loop.exit();
            self.window.set_title("uterm");
            return;
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        self.sync_window_title();
    }

    fn close_tab_by_id(&mut self, id: usize, event_loop: &ActiveEventLoop) {
        if let Some(idx) = self.tabs.iter().position(|t| t.id == id) {
            let was_active = idx == self.active;
            self.tabs.remove(idx);
            if self.tabs.is_empty() {
                self.active = 0;
                event_loop.exit();
                self.window.set_title("uterm");
                return;
            }
            if was_active {
                self.active = idx.min(self.tabs.len() - 1);
            } else if idx < self.active {
                self.active = self.active.saturating_sub(1);
            }
            if self.active >= self.tabs.len() { self.active = self.tabs.len() - 1; }
            self.sync_window_title();
        }
    }

    fn close_tab_at(&mut self, idx: usize, event_loop: &ActiveEventLoop) {
        if idx >= self.tabs.len() { return; }
        let was_active = idx == self.active;
        self.tabs.remove(idx);
        if self.tabs.is_empty() {
            self.active = 0;
            event_loop.exit();
            self.window.set_title("uterm");
            return;
        }
        if was_active {
            self.active = idx.min(self.tabs.len() - 1);
        } else if idx < self.active {
            self.active = self.active.saturating_sub(1);
        }
        if self.active >= self.tabs.len() { self.active = self.tabs.len() - 1; }
        self.sync_window_title();
    }

    fn next_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active = (self.active + 1) % self.tabs.len();
            self.sync_window_title();
        }
    }

    fn prev_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
            self.sync_window_title();
        }
    }

    fn set_tab_title(&mut self, id: usize, title: String) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            let t = title.trim().to_string();
            tab.title = if t.is_empty() { "shell".to_string() } else { t };
        }
        self.sync_window_title();
    }

    fn _reset_tab_title(&mut self, id: usize) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            tab.title = "shell".to_string();
        }
        self.sync_window_title();
    }

    fn copy_selection(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else { return; };
        let text = {
            let term = tab.terminal.lock();
            let Some(range) = term.selection.as_ref().and_then(|s| s.to_range(&term)) else { return; };
            term.bounds_to_string(range.start, range.end)
        };
        if !text.is_empty() {
            self.clipboard.set(ClipboardType::Clipboard, text.clone());
            self.clipboard.set(ClipboardType::Selection, text);
        }
    }

    fn update_primary_selection(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else { return; };
        let text = {
            let term = tab.terminal.lock();
            term.selection
                .as_ref()
                .and_then(|s| s.to_range(&term))
                .map(|range| term.bounds_to_string(range.start, range.end))
        };
        if let Some(text) = text {
            if !text.is_empty() {
                self.clipboard.set(ClipboardType::Selection, text);
            }
        }
    }

    fn paste_from(&mut self, ty: ClipboardType) {
        let mut text = self.clipboard.get(ty);
        if text.is_empty() && matches!(ty, ClipboardType::Selection) {
            text = self.clipboard.get(ClipboardType::Clipboard);
        }
        if text.is_empty() { return; }
        let Some(tab) = self.tabs.get(self.active) else { return; };
        let bracketed = {
            let term = tab.terminal.lock();
            term.mode().contains(Mode::BRACKETED_PASTE)
        };
        let payload = if bracketed {
            format!("\x1b[200~{text}\x1b[201~")
        } else {
            text
        };
        let _ = tab.pty.channel.send(Msg::Input(Cow::Owned(payload.into_bytes())));
    }

    fn paste_clipboard(&mut self) {
        self.paste_from(ClipboardType::Clipboard);
    }

    fn ensure_glyph(&mut self, ch: char) {
        if !self.glyph_cache.contains_key(&ch) {
            let raster = self.font.rasterize(ch, self.font_px);
            self.glyph_cache.insert(ch, raster);
        }
    }

    fn render(&mut self) {
        if self.tabs.is_empty() { return; }
        if self.active >= self.tabs.len() { self.active = self.tabs.len() - 1; }
        let size = self.window.inner_size();
        let (width, height) = (size.width, size.height);
        if width == 0 || height == 0 { return; }
        if let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
            let _ = self.surface.resize(w, h);
        }

        let (cells, ncols, nrows, cursor_row, cursor_col, show_cursor, cursor_shape, default_bg, cur_u, selection) = {
            let tab = &self.tabs[self.active];
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

            let mut cells = Vec::with_capacity(nrows * ncols);
            for row in &rows_vec {
                for col in 0..ncols {
                    let square = &row[Column(col)];
                    let style = term.grid.style_of(square);
                    let mut fg = resolve_ansi_color(style.fg, &term.colors, default_fg);
                    let mut bg = resolve_ansi_color(style.bg, &term.colors, default_bg);
                    if style.flags.contains(rio_vt::crosswords::style::StyleFlags::INVERSE) {
                        std::mem::swap(&mut fg, &mut bg);
                    }
                    cells.push(CellRender { ch: square.c(), fg, bg });
                }
            }
            term.reset_damage();
            term.damage_event_in_flight = false;
            (cells, ncols, nrows, c.row.0, c.col.0, show, shape, default_bg, cur_u, selection)
        };

        let tab_layouts = self.tab_layouts(width as usize);
        self.ensure_glyph('×');
        for (label, _, _, _) in &tab_layouts {
            for ch in label.chars() { self.ensure_glyph(ch); }
        }
        for cell in &cells {
            if cell.ch != '\0' && cell.ch != ' ' { self.ensure_glyph(cell.ch); }
        }

        let mut buffer = match self.surface.buffer_mut() {
            Ok(b) => b,
            Err(_) => return,
        };
        buffer.fill(default_bg);

        let bw = width as usize;
        let bh = height as usize;
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
            fill_rect(&mut buffer, bw, bh, 0, bar_h.saturating_sub(1), bw, 1, bar_border);
        }

        for (i, (label, x0, w, close_x0)) in tab_layouts.iter().enumerate() {
            let is_active = i == self.active;
            let (fg, bgc) = if is_active { (active_fg, active_bg) } else { (inactive_fg, inactive_bg) };
            fill_rect(&mut buffer, bw, bh, *x0, 1, *w, bar_h.saturating_sub(2), bgc);
            if is_active {
                fill_rect(&mut buffer, bw, bh, *x0, 0, *w, 2.min(bar_h), rgb_u32(0x6b, 0xb6, 0xff));
            }
            // tab text
            let mut cx = (*x0 as i32) + (self.cell_w as i32 / 2);
            let max_x = (*x0 + *w).saturating_sub(self.cell_w) as i32;
            for ch in label.chars() {
                if cx >= max_x { break; }
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
                let mut cx = *cx0 as i32;
                if let Some((metrics, bitmap)) = self.glyph_cache.get(&'×') {
                    let gx = cx + metrics.xmin;
                    let gy = self.ascent as i32 - metrics.ymin - metrics.height as i32;
                    blit_glyph(&mut buffer, bw, bh, gx, gy, metrics, bitmap, if is_active { active_fg } else { close_fg });
                    let _ = &mut cx;
                }
            }
        }

        let y_origin = bar_h;
        for r in 0..nrows {
            let py = y_origin + r * self.cell_h;
            if py >= bh { break; }
            for c in 0..ncols {
                let idx = r * ncols + c;
                let Some(cell) = cells.get(idx) else { continue; };
                let px = c * self.cell_w;
                if px >= bw { break; }

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
                        let gy = py as i32 + self.ascent as i32 - metrics.ymin - metrics.height as i32;
                        blit_glyph(&mut buffer, bw, bh, gx, gy, metrics, bitmap, fg);
                    }
                }

                if show_cursor && cursor_row == r as i32 && cursor_col == c {
                    match cursor_shape {
                        CursorShape::Beam => fill_rect(&mut buffer, bw, bh, px, py, 2, self.cell_h, cur_u),
                        CursorShape::Underline => {
                            let uy = py + self.cell_h.saturating_sub(2);
                            fill_rect(&mut buffer, bw, bh, px, uy, self.cell_w, 2, cur_u);
                        }
                        _ => fill_rect_alpha(&mut buffer, bw, bh, px, py, self.cell_w, self.cell_h, cur_u, 110),
                    }
                }
            }
        }

        let _ = buffer.present();
    }

    fn tab_label_text(&self, idx: usize) -> String {
        let title = self.tabs.get(idx).map(|t| t.title.as_str()).unwrap_or("shell");
        let mut t = title.trim().replace(['\n', '\r', '\t'], " ");
        if t.is_empty() { t = "shell".to_string(); }
        // truncate to keep tabs reasonable
        let max_chars = 18usize;
        if t.chars().count() > max_chars {
            t = t.chars().take(max_chars.saturating_sub(1)).collect::<String>() + "…";
        }
        format!(" {} ", t)
    }

    fn tab_layouts(&self, bw: usize) -> Vec<(String, usize, usize, Option<usize>)> {
        let mut layouts = Vec::new();
        let mut xoff = 4usize;
        let gap = 4usize;
        let close_pad = self.cell_w.max(8);
        for i in 0..self.tabs.len() {
            let label = self.tab_label_text(i);
            let text_w = label.chars().count() * self.cell_w;
            let w = (text_w + close_pad + self.cell_w / 2).min(bw.saturating_sub(xoff + gap).max(1));
            let close_x0 = if xoff + w > self.cell_w { Some(xoff + w.saturating_sub(close_pad)) } else { None };
            layouts.push((label, xoff, w, close_x0));
            xoff = xoff.saturating_add(w + gap);
            if xoff >= bw { break; }
        }
        layouts
    }

    fn tab_bar_hit(&self, x: f64, y: f64) -> Option<(usize, bool)> {
        let bar_h = self.cell_h as f64;
        if y < 0.0 || y >= bar_h { return None; }
        let layouts = self.tab_layouts(self.window.inner_size().width as usize);
        for (i, (_, x0, w, close_x0)) in layouts.iter().enumerate() {
            let start = *x0 as f64;
            let end = (*x0 + *w) as f64;
            if x >= start && x < end {
                if let Some(cx0) = close_x0 {
                    let cstart = *cx0 as f64;
                    if x >= cstart { return Some((i, true)); }
                }
                return Some((i, false));
            }
        }
        None
    }

    fn terminal_cell_at(&self, x: f64, y: f64) -> Option<(usize, usize, Side)> {
        let bar_h = self.cell_h as f64;
        let ty = y - bar_h;
        if ty < 0.0 || x < 0.0 { return None; }
        let (cols, rows) = self.grid_dims();
        let col_f = x / self.cell_w as f64;
        let row_f = ty / self.cell_h as f64;
        let col = col_f.floor() as i64;
        let row = row_f.floor() as i64;
        if col < 0 || row < 0 { return None; }
        let col = (col as usize).min(cols.saturating_sub(1));
        let row = (row as usize).min(rows.saturating_sub(1));
        let side = if col_f - (col as f64) < 0.5 { Side::Left } else { Side::Right };
        Some((col, row, side))
    }

    fn mouse_report_mode(&self) -> bool {
        let Some(tab) = self.tabs.get(self.active) else { return false; };
        let term = tab.terminal.lock();
        term.mode().intersects(Mode::MOUSE_MODE)
    }

    fn send_mouse_report(&self, button: u8, col: u16, row: u16, pressed: bool) {
        let Some(tab) = self.tabs.get(self.active) else { return; };
        let (sgr, utf8, mouse_mode) = {
            let term = tab.terminal.lock();
            let mode = term.mode();
            (mode.contains(Mode::SGR_MOUSE), mode.contains(Mode::UTF8_MOUSE), mode.intersects(Mode::MOUSE_MODE))
        };
        if !mouse_mode { return; }
        let bytes = encode_mouse_report(button, col, row, pressed, sgr, utf8);
        let _ = tab.pty.channel.send(Msg::Input(Cow::Owned(bytes)));
    }

    fn send_mouse_motion(&self, col: u16, row: u16) {
        let Some(tab) = self.tabs.get(self.active) else { return; };
        let (sgr, utf8, report_motion, report_drag) = {
            let term = tab.terminal.lock();
            let mode = term.mode();
            (mode.contains(Mode::SGR_MOUSE), mode.contains(Mode::UTF8_MOUSE), mode.contains(Mode::MOUSE_MOTION), mode.contains(Mode::MOUSE_DRAG))
        };
        let any_button = self.mouse_buttons.iter().any(|&b| b);
        if !report_motion && !(report_drag && any_button) { return; }
        let mut base = if any_button {
            if self.mouse_buttons[0] { 32 } else if self.mouse_buttons[1] { 33 } else { 34 }
        } else {
            35
        };
        base += 32;
        let bytes = encode_mouse_report(base, col, row, true, sgr, utf8);
        let _ = tab.pty.channel.send(Msg::Input(Cow::Owned(bytes)));
    }

    fn handle_mouse_press(&mut self, button: MouseButton, state: ElementState, event_loop: &ActiveEventLoop) {
        let (x, y) = self.cursor_pos;
        if state == ElementState::Pressed && button == MouseButton::Left {
            if let Some((idx, close)) = self.tab_bar_hit(x, y) {
                if close {
                    self.close_tab_at(idx, event_loop);
                } else {
                    self.active = idx;
                    self.sync_window_title();
                }
                self.selecting = false;
                self.window.request_redraw();
                return;
            }
        }

        let (btn_code, idx) = match button {
            MouseButton::Left => (0u8, 0usize),
            MouseButton::Middle => (1u8, 1usize),
            MouseButton::Right => (2u8, 2usize),
            _ => return,
        };

        if self.mouse_report_mode() {
            if state == ElementState::Pressed { self.mouse_buttons[idx] = true; } else { self.mouse_buttons[idx] = false; }
            if let Some((col, row, _)) = self.terminal_cell_at(x, y) {
                self.send_mouse_report(btn_code, col as u16, row as u16, state == ElementState::Pressed);
            }
            self.selecting = false;
            return;
        }

        // Local selection handling (no mouse reporting)
        match (button, state) {
            (MouseButton::Left, ElementState::Pressed) => {
                self.mouse_buttons[0] = true;
                if let Some((col, row, side)) = self.terminal_cell_at(x, y) {
                    if let Some(tab) = self.tabs.get(self.active) {
                        let mut term = tab.terminal.lock();
                        let point = Pos::new(Line(row as i32), Column(col));
                        let ty = SelectionType::Simple;
                        term.selection = Some(Selection::new(ty, point, side));
                    }
                    self.selecting = true;
                }
            }
            (MouseButton::Left, ElementState::Released) => {
                self.mouse_buttons[0] = false;
                if self.selecting {
                    if let Some((col, row, side)) = self.terminal_cell_at(x, y) {
                        if let Some(tab) = self.tabs.get(self.active) {
                            let mut term = tab.terminal.lock();
                            if let Some(sel) = &mut term.selection {
                                sel.update(Pos::new(Line(row as i32), Column(col)), side);
                            }
                        }
                    }
                    self.update_primary_selection();
                }
                self.selecting = false;
            }
            (MouseButton::Middle, ElementState::Pressed) => {
                self.mouse_buttons[1] = true;
                if self.terminal_cell_at(x, y).is_some() {
                    self.paste_from(ClipboardType::Selection);
                }
            }
            (MouseButton::Middle, ElementState::Released) => {
                self.mouse_buttons[1] = false;
            }
            (_, ElementState::Pressed) => {
                self.mouse_buttons[idx] = true;
            }
            (_, ElementState::Released) => {
                self.mouse_buttons[idx] = false;
            }
        }
        self.window.request_redraw();
        let _ = event_loop;
    }
}

fn is_selected(selection: Option<SelectionRange>, row: i32, col: usize) -> bool {
    let Some(sel) = selection else { return false; };
    if sel.is_block {
        let c0 = sel.start.col.0.min(sel.end.col.0);
        let c1 = sel.start.col.0.max(sel.end.col.0);
        row >= sel.start.row.0 && row <= sel.end.row.0 && col >= c0 && col <= c1
    } else {
        if row < sel.start.row.0 || row > sel.end.row.0 { return false; }
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

fn resolve_named(colors: &TermColors, name: NamedColor, dr: u8, dg: u8, db: u8) -> u32 {
    if let Some(a) = &colors[name] { arr_to_u32(*a) } else { rgb_u32(dr, dg, db) }
}

fn indexed_color(idx: u8) -> (u8, u8, u8) {
    match idx {
        0 => (0x00, 0x00, 0x00), 1 => (0xcd, 0x00, 0x00), 2 => (0x00, 0xcd, 0x00), 3 => (0xcd, 0xcd, 0x00),
        4 => (0x00, 0x00, 0xee), 5 => (0xcd, 0x00, 0xcd), 6 => (0x00, 0xcd, 0xcd), 7 => (0xe5, 0xe5, 0xe5),
        8 => (0x7f, 0x7f, 0x7f), 9 => (0xff, 0x00, 0x00), 10 => (0x00, 0xff, 0x00), 11 => (0xff, 0xff, 0x00),
        12 => (0x5c, 0x5c, 0xff), 13 => (0xff, 0x00, 0xff), 14 => (0x00, 0xff, 0xff), 15 => (0xff, 0xff, 0xff),
        16..=231 => {
            let i = idx - 16;
            let r = i / 36; let g = (i / 6) % 6; let b = i % 6;
            let cv = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            (cv(r), cv(g), cv(b))
        }
        232..=255 => { let v = 8 + 10 * (idx - 232); (v, v, v) }
    }
}

fn resolve_ansi_color(color: AnsiColor, colors: &TermColors, _default: u32) -> u32 {
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
            NamedColor::Foreground | NamedColor::LightForeground | NamedColor::DimForeground => resolve_named(colors, NamedColor::Foreground, 0xd8, 0xd8, 0xd8),
            NamedColor::Background => resolve_named(colors, NamedColor::Background, 0x10, 0x10, 0x10),
            NamedColor::Cursor => resolve_named(colors, NamedColor::Cursor, 0xf0, 0xf0, 0xf0),
            NamedColor::Black => colors[NamedColor::Black].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0,0,0)),
            NamedColor::Red => colors[NamedColor::Red].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0xcd,0,0)),
            NamedColor::Green => colors[NamedColor::Green].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0,0xcd,0)),
            NamedColor::Yellow => colors[NamedColor::Yellow].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0xcd,0xcd,0)),
            NamedColor::Blue => colors[NamedColor::Blue].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0,0,0xee)),
            NamedColor::Magenta => colors[NamedColor::Magenta].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0xcd,0,0xcd)),
            NamedColor::Cyan => colors[NamedColor::Cyan].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0,0xcd,0xcd)),
            NamedColor::White => colors[NamedColor::White].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0xe5,0xe5,0xe5)),
            NamedColor::LightBlack => colors[NamedColor::LightBlack].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0x7f,0x7f,0x7f)),
            NamedColor::LightRed => colors[NamedColor::LightRed].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0xff,0,0)),
            NamedColor::LightGreen => colors[NamedColor::LightGreen].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0,0xff,0)),
            NamedColor::LightYellow => colors[NamedColor::LightYellow].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0xff,0xff,0)),
            NamedColor::LightBlue => colors[NamedColor::LightBlue].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0x5c,0x5c,0xff)),
            NamedColor::LightMagenta => colors[NamedColor::LightMagenta].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0xff,0,0xff)),
            NamedColor::LightCyan => colors[NamedColor::LightCyan].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0,0xff,0xff)),
            NamedColor::LightWhite => colors[NamedColor::LightWhite].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0xff,0xff,0xff)),
            NamedColor::DimBlack => colors[NamedColor::DimBlack].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0,0,0)),
            NamedColor::DimRed => colors[NamedColor::DimRed].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0x88,0,0)),
            NamedColor::DimGreen => colors[NamedColor::DimGreen].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0,0x88,0)),
            NamedColor::DimYellow => colors[NamedColor::DimYellow].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0x88,0x88,0)),
            NamedColor::DimBlue => colors[NamedColor::DimBlue].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0,0,0x88)),
            NamedColor::DimMagenta => colors[NamedColor::DimMagenta].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0x88,0,0x88)),
            NamedColor::DimCyan => colors[NamedColor::DimCyan].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0,0x88,0x88)),
            NamedColor::DimWhite => colors[NamedColor::DimWhite].as_ref().copied().map(arr_to_u32).unwrap_or(rgb_u32(0x99,0x99,0x99)),
        }
    }
}

fn encode_mouse_report(button: u8, col: u16, row: u16, pressed: bool, sgr: bool, utf8: bool) -> Vec<u8> {
    let x = col.saturating_add(1);
    let y = row.saturating_add(1);
    if sgr {
        let end = if pressed { 'M' } else { 'm' };
        return format!("\x1b[<{button};{x};{y}{end}").into_bytes();
    }
    let encoded = if pressed { button } else { button | 3 };
    let mut out = vec![0x1b, b'[', b'M', 32u8.saturating_add(encoded)];
    for value in [x, y] {
        if utf8 && value >= 95 {
            let encoded = char::from_u32(32 + value as u32).unwrap_or(' ');
            let mut buffer = [0u8; 4];
            out.extend_from_slice(encoded.encode_utf8(&mut buffer).as_bytes());
        } else {
            out.push(32u8.saturating_add(value.min(223) as u8));
        }
    }
    out
}

fn ctrl_char(ch: char) -> Option<u8> {
    match ch {
        'a'..='z' => Some((ch as u8) - b'a' + 1),
        'A'..='Z' => Some((ch as u8) - b'A' + 1),
        '@' => Some(0), '[' => Some(0x1b), '\\' => Some(0x1c), ']' => Some(0x1d),
        '^' | '~' => Some(0x1e), '_' => Some(0x1f), ' ' => Some(0), '?' => Some(0x7f),
        '2' => Some(0), '3'..='7' => Some((ch as u8) - b'0' + 0x1b), '/' => Some(0x1f),
        _ => None,
    }
}

fn key_to_bytes(k: &winit::event::KeyEvent, modifiers: Modifiers) -> Option<Vec<u8>> {
    let ctrl = modifiers.state().control_key();
    let alt = modifiers.state().alt_key();
    if ctrl {
        if let Key::Character(s) = &k.logical_key {
            if let Some(ch) = s.chars().next() {
                if let Some(b) = ctrl_char(ch) {
                    let mut out = vec![b];
                    if alt { out.insert(0, 0x1b); }
                    return Some(out);
                }
            }
        }
    }
    match &k.logical_key {
        Key::Character(s) => {
            if !ctrl && !alt {
                if let Some(text) = &k.text { if !text.is_empty() { return Some(text.as_bytes().to_vec()); } }
            }
            if ctrl { return None; }
            let b = if let Some(text) = &k.text { if !text.is_empty() { text.as_bytes().to_vec() } else { s.as_bytes().to_vec() } } else { s.as_bytes().to_vec() };
            if b.is_empty() { return None; }
            if alt { let mut out = vec![0x1b]; out.extend_from_slice(&b); Some(out) } else { Some(b) }
        }
        Key::Named(NamedKey::Enter) => Some(b"\r".to_vec()),
        Key::Named(NamedKey::Backspace) => Some(if ctrl { vec![0x08] } else { vec![0x7f] }),
        Key::Named(NamedKey::Tab) => Some(b"\t".to_vec()),
        Key::Named(NamedKey::Escape) => Some(vec![0x1b]),
        Key::Named(NamedKey::Space) => if ctrl { Some(vec![0]) } else if alt { Some(b"\x1b ".to_vec()) } else if let Some(text) = &k.text { Some(text.as_bytes().to_vec()) } else { Some(b" ".to_vec()) },
        Key::Named(NamedKey::ArrowUp) => Some(b"\x1b[A".to_vec()),
        Key::Named(NamedKey::ArrowDown) => Some(b"\x1b[B".to_vec()),
        Key::Named(NamedKey::ArrowRight) => Some(b"\x1b[C".to_vec()),
        Key::Named(NamedKey::ArrowLeft) => Some(b"\x1b[D".to_vec()),
        Key::Named(NamedKey::Home) => Some(b"\x1b[H".to_vec()),
        Key::Named(NamedKey::End) => Some(b"\x1b[F".to_vec()),
        Key::Named(NamedKey::PageUp) => Some(b"\x1b[5~".to_vec()),
        Key::Named(NamedKey::PageDown) => Some(b"\x1b[6~".to_vec()),
        Key::Named(NamedKey::Delete) => Some(b"\x1b[3~".to_vec()),
        _ => None,
    }
}

fn arr_to_u32(a: [f32; 4]) -> u32 {
    let r = (a[0].clamp(0.0, 1.0) * 255.0) as u8;
    let g = (a[1].clamp(0.0, 1.0) * 255.0) as u8;
    let b = (a[2].clamp(0.0, 1.0) * 255.0) as u8;
    rgb_u32(r, g, b)
}

fn rgb_u32(r: u8, g: u8, b: u8) -> u32 { ((r as u32) << 16) | ((g as u32) << 8) | b as u32 }

fn fill_rect(buf: &mut [u32], bw: usize, bh: usize, x: usize, y: usize, w: usize, h: usize, color: u32) {
    let x2 = (x + w).min(bw);
    let y2 = (y + h).min(bh);
    for yy in y.min(bh)..y2 {
        let row = yy * bw;
        for xx in x.min(bw)..x2 { buf[row + xx] = color; }
    }
}

fn fill_rect_alpha(buf: &mut [u32], bw: usize, bh: usize, x: usize, y: usize, w: usize, h: usize, color: u32, a: u8) {
    let x2 = (x + w).min(bw);
    let y2 = (y + h).min(bh);
    let a = a as u32;
    let ia = 255 - a;
    for yy in y.min(bh)..y2 {
        let row = yy * bw;
        for xx in x.min(bw)..x2 { buf[row + xx] = blend(buf[row + xx], color, a, ia); }
    }
}

fn blend(dst: u32, src: u32, a: u32, ia: u32) -> u32 {
    let dr = (dst >> 16) & 0xff; let dg = (dst >> 8) & 0xff; let db = dst & 0xff;
    let sr = (src >> 16) & 0xff; let sg = (src >> 8) & 0xff; let sb = src & 0xff;
    let r = (sr * a + dr * ia) / 255;
    let g = (sg * a + dg * ia) / 255;
    let b = (sb * a + db * ia) / 255;
    (r << 16) | (g << 8) | b
}

fn blit_glyph(buf: &mut [u32], bw: usize, bh: usize, x0: i32, y0: i32, metrics: &fontdue::Metrics, bitmap: &[u8], color: u32) {
    for y in 0..metrics.height {
        let py = y0 + y as i32;
        if py < 0 || py >= bh as i32 { continue; }
        for x in 0..metrics.width {
            let px = x0 + x as i32;
            if px < 0 || px >= bw as i32 { continue; }
            let alpha = bitmap[y * metrics.width + x] as u32;
            if alpha == 0 { continue; }
            let idx = py as usize * bw + px as usize;
            let ia = 255 - alpha;
            buf[idx] = blend(buf[idx], color, alpha, ia);
        }
    }
}

struct Uterm {
    proxy: EventLoopProxy<UiEvent>,
    state: Option<AppState>,
}

impl ApplicationHandler<UiEvent> for Uterm {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() { return; }
        let window = Rc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("uterm")
                        .with_inner_size(LogicalSize::new(900.0, 620.0)),
                )
                .unwrap(),
        );
        let context = Context::new(window.clone()).unwrap();
        let surface = Surface::new(&context, window.clone()).unwrap();

        let font = Font::from_bytes(CASCADIA_TTF, fontdue::FontSettings::default()).expect("font");
        let scale = window.scale_factor() as f32;
        let font_px = (14.0 * scale * (96.0 / 72.0)).round().max(12.0);
        let lm = font.horizontal_line_metrics(font_px).unwrap();
        let (m_m, _) = font.rasterize('M', font_px);
        let cell_w = m_m.advance_width.ceil().max(1.0) as usize;
        let cell_h = (lm.ascent - lm.descent + lm.line_gap).ceil().max(1.0) as usize;
        let ascent = lm.ascent;

        let sz = window.inner_size();
        let cols = (sz.width as usize / cell_w).max(2);
        let total_rows = (sz.height as usize / cell_h).max(2);
        let rows = total_rows.saturating_sub(1).max(1);

        let listener = Listener { proxy: self.proxy.clone() };
        let first = Tab::new(0, cols, rows, cell_w, cell_h, listener);

        let clipboard = unsafe {
            event_loop
                .display_handle()
                .ok()
                .map(|h| Clipboard::new(h.as_raw()))
                .unwrap_or_default()
        };

        let st = AppState {
            window,
            surface,
            tabs: vec![first],
            active: 0,
            next_tab_id: 1,
            font,
            font_px,
            cell_w,
            cell_h,
            ascent,
            modifiers: Modifiers::default(),
            glyph_cache: HashMap::new(),
            proxy: self.proxy.clone(),
            cursor_pos: (0.0, 0.0),
            mouse_buttons: [false; 3],
            selecting: false,
            clipboard,
        };
        st.sync_window_title();
        st.window.request_redraw();
        self.state = Some(st);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UiEvent) {
        let Some(st) = &mut self.state else { return; };
        match event {
            UiEvent::Wakeup => st.window.request_redraw(),
            UiEvent::TabExited(id) => {
                st.close_tab_by_id(id, event_loop);
                st.window.request_redraw();
            }
            UiEvent::TitleChanged(id, title) => {
                st.set_tab_title(id, title);
                st.window.request_redraw();
            }
            UiEvent::ClipboardStore(ty, text) => {
                st.clipboard.set(ty, text);
            }
            UiEvent::ClipboardLoad(route_id, ty, formatter) => {
                let contents = st.clipboard.get(ty);
                let reply = formatter(&contents);
                if let Some(tab) = st.tabs.iter().find(|t| t.id == route_id) {
                    let _ = tab.pty.channel.send(Msg::Input(Cow::Owned(reply.into_bytes())));
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WinitWindowId, event: WindowEvent) {
        let Some(st) = &mut self.state else { return; };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => st.render(),
            WindowEvent::Resized(_) => { st.resize_all_tabs(); st.window.request_redraw(); }
            WindowEvent::ModifiersChanged(m) => { st.modifiers = m; }
            WindowEvent::CursorMoved { position, .. } => {
                st.cursor_pos = (position.x, position.y);
                if st.mouse_report_mode() {
                    if let Some((col, row, _)) = st.terminal_cell_at(position.x, position.y) {
                        st.send_mouse_motion(col as u16, row as u16);
                    }
                } else if st.selecting && st.mouse_buttons[0] {
                    if let Some((col, row, side)) = st.terminal_cell_at(position.x, position.y) {
                        if let Some(tab) = st.tabs.get(st.active) {
                            let mut term = tab.terminal.lock();
                            if let Some(sel) = &mut term.selection {
                                sel.update(Pos::new(Line(row as i32), Column(col)), side);
                            }
                        }
                        st.window.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                st.handle_mouse_press(button, state, event_loop);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if let Some((col, row, _)) = st.terminal_cell_at(st.cursor_pos.0, st.cursor_pos.1) {
                    let lines = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y as i32,
                        MouseScrollDelta::PixelDelta(pos) => {
                            if pos.y.abs() < 1.0 { 0 } else { (pos.y / st.cell_h as f64).round() as i32 }
                        }
                    };
                    if lines != 0 {
                        let (btn, count) = if lines > 0 { (64u8, lines) } else { (65u8, -lines) };
                        for _ in 0..count.min(10) {
                            st.send_mouse_report(btn, col as u16, row as u16, true);
                            st.send_mouse_report(btn, col as u16, row as u16, false);
                        }
                    }
                }
            }
            WindowEvent::KeyboardInput { event: k, .. } => {
                if !(k.state == ElementState::Pressed || k.repeat) { return; }
                let mods = st.modifiers.state();
                let ctrl = mods.control_key();
                let shift = mods.shift_key();

                if ctrl && shift {
                    if let Key::Character(s) = &k.logical_key {
                        let c = s.to_lowercase();
                        if c == "t" { st.new_tab(); st.window.request_redraw(); return; }
                        else if c == "w" { st.close_active_tab(event_loop); st.window.request_redraw(); return; }
                        else if c == "c" { st.copy_selection(); return; }
                        else if c == "v" { st.paste_clipboard(); return; }
                    }
                    if matches!(k.logical_key, Key::Named(NamedKey::Tab)) { st.prev_tab(); st.window.request_redraw(); return; }
                }
                if ctrl {
                    if matches!(k.logical_key, Key::Named(NamedKey::Tab)) && !shift { st.next_tab(); st.window.request_redraw(); return; }
                    if matches!(k.logical_key, Key::Named(NamedKey::PageDown)) { st.next_tab(); st.window.request_redraw(); return; }
                    if matches!(k.logical_key, Key::Named(NamedKey::PageUp)) { st.prev_tab(); st.window.request_redraw(); return; }
                }

                if let Some(bytes) = key_to_bytes(&k, st.modifiers) {
                    if let Some(tab) = st.tabs.get(st.active) {
                        let _ = tab.pty.channel.send(Msg::Input(Cow::Owned(bytes)));
                    }
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let event_loop = EventLoop::<UiEvent>::with_user_event().build().unwrap();
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let mut app = Uterm { proxy, state: None };
    event_loop.run_app(&mut app).unwrap();
}
