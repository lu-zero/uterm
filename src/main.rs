use fontdue::Font;
use rio_vt::ansi::CursorShape;
use rio_vt::config::colors::NamedColor;
use rio_vt::crosswords::grid::Dimensions;
use rio_vt::crosswords::pos::Column;
use rio_vt::crosswords::{Crosswords, Mode};
use rio_vt::event::sync::FairMutex;
use rio_vt::event::{EventListener, Msg, RioEvent, WindowId, WindowSize};
use rio_vt::performer::Machine;
use softbuffer::{Context, Surface};
use std::borrow::Cow;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Modifiers, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId as WinitWindowId};

const CASCADIA_TTF: &[u8] = include_bytes!("../CascadiaCodeNF.ttf");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiEvent {
    Wakeup,
}

#[derive(Clone)]
struct Listener {
    proxy: EventLoopProxy<UiEvent>,
}

impl EventListener for Listener {
    fn send_event(&self, _event: RioEvent, _id: WindowId) {
        let _ = self.proxy.send_event(UiEvent::Wakeup);
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
    terminal: Arc<FairMutex<Crosswords<Listener>>>,
    pty: PtyHandle,
}

impl Tab {
    fn new(cols: usize, rows: usize, cell_w: usize, cell_h: usize, listener: Listener) -> Self {
        let terminal = Arc::new(FairMutex::new(Crosswords::new(
            TermSize { cols, rows, cell_w: cell_w as f32, cell_h: cell_h as f32 },
            CursorShape::Block,
            listener.clone(),
            WindowId::from(0),
            0,
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
        let machine = Machine::new(terminal.clone(), pty, listener, WindowId::from(0), 0).expect("machine");
        let channel = machine.channel();
        let thread = machine.spawn();
        Tab { terminal, pty: PtyHandle { channel, _thread: thread } }
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

struct AppState {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    tabs: Vec<Tab>,
    active: usize,
    font: Font,
    font_px: f32,
    cell_w: usize,
    cell_h: usize,
    ascent: f32,
    modifiers: Modifiers,
    glyph_cache: HashMap<char, (fontdue::Metrics, Vec<u8>)>,
    proxy: EventLoopProxy<UiEvent>,
}

impl AppState {
    fn grid_dims(&self) -> (usize, usize) {
        let sz = self.window.inner_size();
        let cols = (sz.width as usize / self.cell_w.max(1)).max(2);
        let total_rows = (sz.height as usize / self.cell_h.max(1)).max(2);
        // Reserve 1 row for the tab bar when there are tabs (always shown for simplicity).
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
        self.tabs.push(Tab::new(cols, rows, self.cell_w, self.cell_h, listener));
        self.active = self.tabs.len() - 1;
    }

    fn close_active_tab(&mut self, event_loop: &ActiveEventLoop) {
        if self.tabs.is_empty() { return; }
        self.tabs.remove(self.active);
        if self.tabs.is_empty() {
            event_loop.exit();
            return;
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
    }

    fn next_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active = (self.active + 1) % self.tabs.len();
        }
    }

    fn prev_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
        }
    }

    fn ensure_glyph(&mut self, ch: char) {
        if !self.glyph_cache.contains_key(&ch) {
            let raster = self.font.rasterize(ch, self.font_px);
            self.glyph_cache.insert(ch, raster);
        }
    }

    fn render(&mut self) {
        let size = self.window.inner_size();
        let (width, height) = (size.width, size.height);
        if width == 0 || height == 0 { return; }
        if let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
            let _ = self.surface.resize(w, h);
        }

        // Snapshot active tab
        let (rows_vec, cursor_row, cursor_col, show_cursor, cursor_shape, fg_u, bg_u, cur_u) = {
            let tab = &self.tabs[self.active];
            let mut term = tab.terminal.lock();
            let rows_vec: Vec<_> = term.visible_rows();
            let c = term.grid.cursor.pos;
            let show = term.mode().contains(Mode::SHOW_CURSOR);
            let shape = term.cursor_shape;
            let fg = arr_to_rgb(term.colors[NamedColor::Foreground], 0xd8, 0xd8, 0xd8);
            let bg = arr_to_rgb(term.colors[NamedColor::Background], 0x10, 0x10, 0x10);
            let cc = arr_to_rgb(term.colors[NamedColor::Cursor], 0xf0, 0xf0, 0xf0);
            term.reset_damage();
            term.damage_event_in_flight = false;
            (rows_vec, c.row.0, c.col.0, show, shape, fg, bg, cc)
        };

        // Pre-cache glyphs for tab bar labels/hint
        let tab_labels: Vec<String> = (0..self.tabs.len())
            .map(|i| if i == self.active { format!("[{}:sh]", i + 1) } else { format!(" {}:sh ", i + 1) })
            .collect();
        let hint = "  ^S-T:new ^S-W:close ^Tab:switch";
        for s in tab_labels.iter().chain(std::iter::once(&hint.to_string())) {
            for ch in s.chars() { self.ensure_glyph(ch); }
        }

        let mut buffer = match self.surface.buffer_mut() {
            Ok(b) => b,
            Err(_) => return,
        };
        buffer.fill(bg_u);

        let bw = width as usize;
        let bh = height as usize;
        let bar_h = self.cell_h.min(bh);

        // Tab bar
        let bar_bg = rgb_u32(0x22, 0x22, 0x28);
        let bar_fg = rgb_u32(0xc0, 0xc0, 0xc8);
        let bar_active_bg = rgb_u32(0x3a, 0x5f, 0x8a);
        let bar_active_fg = rgb_u32(0xff, 0xff, 0xff);
        fill_rect(&mut buffer, bw, bh, 0, 0, bw, bar_h, bar_bg);
        let mut xoff = 4usize;
        for (i, label) in tab_labels.iter().enumerate() {
            let (fg, bgc) = if i == self.active { (bar_active_fg, bar_active_bg) } else { (bar_fg, bar_bg) };
            let label_w = (label.chars().count() * self.cell_w).min(bw.saturating_sub(xoff));
            fill_rect(&mut buffer, bw, bh, xoff.saturating_sub(2), 0, label_w + 4, bar_h, bgc);
            let mut cx = xoff as i32;
            for ch in label.chars() {
                if let Some((metrics, bitmap)) = self.glyph_cache.get(&ch) {
                    let gx = cx + metrics.xmin;
                    let gy = self.ascent as i32 - metrics.ymin - metrics.height as i32;
                    blit_glyph(&mut buffer, bw, bh, gx, gy, metrics, bitmap, fg);
                    cx += metrics.advance_width.ceil() as i32;
                } else {
                    cx += self.cell_w as i32;
                }
                if cx >= bw as i32 { break; }
            }
            xoff += label_w + 8;
            if xoff >= bw { break; }
        }
        // hint
        if xoff + hint.chars().count() * self.cell_w < bw {
            let mut cx = xoff as i32;
            for ch in hint.chars() {
                if let Some((metrics, bitmap)) = self.glyph_cache.get(&ch) {
                    let gx = cx + metrics.xmin;
                    let gy = self.ascent as i32 - metrics.ymin - metrics.height as i32;
                    blit_glyph(&mut buffer, bw, bh, gx, gy, metrics, bitmap, rgb_u32(0x80, 0x80, 0x90));
                    cx += metrics.advance_width.ceil() as i32;
                } else {
                    cx += self.cell_w as i32;
                }
            }
        }

        let y_origin = bar_h;
        for (r, row) in rows_vec.iter().enumerate() {
            let cols = row.len();
            let py = y_origin + r * self.cell_h;
            if py >= bh { break; }
            for c in 0..cols {
                let square = &row[Column(c)];
                let ch = square.c();
                let px = c * self.cell_w;
                if px >= bw { break; }

                if ch != '\0' && ch != ' ' {
                    if !self.glyph_cache.contains_key(&ch) {
                        let raster = self.font.rasterize(ch, self.font_px);
                        self.glyph_cache.insert(ch, raster);
                    }
                    if let Some((metrics, bitmap)) = self.glyph_cache.get(&ch) {
                        let gx = px as i32 + metrics.xmin;
                        let gy = py as i32 + self.ascent as i32 - metrics.ymin - metrics.height as i32;
                        blit_glyph(&mut buffer, bw, bh, gx, gy, metrics, bitmap, fg_u);
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
}

fn ctrl_char(ch: char) -> Option<u8> {
    match ch {
        'a'..='z' => Some((ch as u8) - b'a' + 1),
        'A'..='Z' => Some((ch as u8) - b'A' + 1),
        '@' => Some(0),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' | '~' => Some(0x1e),
        '_' => Some(0x1f),
        ' ' => Some(0),
        '?' => Some(0x7f),
        '2' => Some(0),
        '3'..='7' => Some((ch as u8) - b'0' + 0x1b),
        '/' => Some(0x1f),
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
                if let Some(text) = &k.text {
                    if !text.is_empty() { return Some(text.as_bytes().to_vec()); }
                }
            }
            if ctrl { return None; }
            let b = if let Some(text) = &k.text {
                if !text.is_empty() { text.as_bytes().to_vec() } else { s.as_bytes().to_vec() }
            } else { s.as_bytes().to_vec() };
            if b.is_empty() { return None; }
            if alt {
                let mut out = vec![0x1b];
                out.extend_from_slice(&b);
                Some(out)
            } else { Some(b) }
        }
        Key::Named(NamedKey::Enter) => Some(b"\r".to_vec()),
        Key::Named(NamedKey::Backspace) => Some(if ctrl { vec![0x08] } else { vec![0x7f] }),
        Key::Named(NamedKey::Tab) => Some(b"\t".to_vec()),
        Key::Named(NamedKey::Escape) => Some(vec![0x1b]),
        Key::Named(NamedKey::Space) => {
            if ctrl { Some(vec![0]) }
            else if alt { Some(b"\x1b ".to_vec()) }
            else if let Some(text) = &k.text { Some(text.as_bytes().to_vec()) }
            else { Some(b" ".to_vec()) }
        }
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

fn arr_to_rgb(arr: Option<[f32; 4]>, dr: u8, dg: u8, db: u8) -> u32 {
    if let Some(a) = arr {
        let r = (a[0].clamp(0.0, 1.0) * 255.0) as u8;
        let g = (a[1].clamp(0.0, 1.0) * 255.0) as u8;
        let b = (a[2].clamp(0.0, 1.0) * 255.0) as u8;
        rgb_u32(r, g, b)
    } else { rgb_u32(dr, dg, db) }
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
        let first = Tab::new(cols, rows, cell_w, cell_h, listener);

        let st = AppState {
            window,
            surface,
            tabs: vec![first],
            active: 0,
            font,
            font_px,
            cell_w,
            cell_h,
            ascent,
            modifiers: Modifiers::default(),
            glyph_cache: HashMap::new(),
            proxy: self.proxy.clone(),
        };
        st.window.request_redraw();
        self.state = Some(st);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: UiEvent) {
        if let Some(st) = &self.state { st.window.request_redraw(); }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WinitWindowId, event: WindowEvent) {
        let Some(st) = &mut self.state else { return; };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => st.render(),
            WindowEvent::Resized(_) => { st.resize_all_tabs(); st.window.request_redraw(); }
            WindowEvent::ModifiersChanged(m) => { st.modifiers = m; }
            WindowEvent::KeyboardInput { event: k, .. } => {
                if !(k.state == ElementState::Pressed || k.repeat) { return; }
                let mods = st.modifiers.state();
                let ctrl = mods.control_key();
                let shift = mods.shift_key();

                // Minimal tab bindings (intercept before PTY):
                // Ctrl+Shift+T new tab, Ctrl+Shift+W close tab, Ctrl+Tab / Ctrl+PageDown next, Ctrl+Shift+Tab / Ctrl+PageUp prev
                if ctrl && shift {
                    if let Key::Character(s) = &k.logical_key {
                        let c = s.to_lowercase();
                        if c == "t" {
                            st.new_tab();
                            st.window.request_redraw();
                            return;
                        } else if c == "w" {
                            st.close_active_tab(event_loop);
                            st.window.request_redraw();
                            return;
                        }
                    }
                    if matches!(k.logical_key, Key::Named(NamedKey::Tab)) {
                        st.prev_tab();
                        st.window.request_redraw();
                        return;
                    }
                }
                if ctrl {
                    if matches!(k.logical_key, Key::Named(NamedKey::Tab)) && !shift {
                        st.next_tab();
                        st.window.request_redraw();
                        return;
                    }
                    if matches!(k.logical_key, Key::Named(NamedKey::PageDown)) {
                        st.next_tab();
                        st.window.request_redraw();
                        return;
                    }
                    if matches!(k.logical_key, Key::Named(NamedKey::PageUp)) {
                        st.prev_tab();
                        st.window.request_redraw();
                        return;
                    }
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
