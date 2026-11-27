//! Application state and its supporting state types.

pub(crate) mod input;
pub(crate) mod render;
pub(crate) mod tabs;

use crate::cli::PRESET_SIZES;
use crate::error::Error;
use crate::event::Listener;
use crate::keybindings::{Action, ScrollModifier};
use crate::terminal::{cells_to_px, Tab};
use rio_vt::clipboard::{Clipboard, ClipboardType};
use rio_vt::crosswords::grid::row::Row;
use rio_vt::crosswords::grid::Scroll;
use rio_vt::crosswords::square::Square;
use rio_vt::crosswords::Mode;
use rio_vt::event::Msg;
use softbuffer::Surface;
use std::borrow::Cow;
use std::rc::Rc;
use std::time::{Duration, Instant};
use term_atlas::{Atlas, Face, GlyphRect, PixelFormat};
use winit::dpi::{PhysicalSize, Size};
use winit::event::Modifiers;
use winit::event_loop::{ActiveEventLoop, OwnedDisplayHandle};
use winit::raw_window_handle::HasDisplayHandle;
use winit::window::Window;

pub(crate) const OVERLAY_TIMEOUT: Duration = Duration::from_millis(1400);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LocalPointerState {
    Idle,
    Selecting,
}

/// The system clipboard together with the display handle it borrows from.
///
/// On Wayland, `Clipboard` holds a raw pointer into the event loop's display
/// and joins a worker thread on drop. The display must therefore outlive the
/// clipboard. Fields drop in declaration order: `clipboard` first (joining its
/// thread), then `_display`, so the pointer stays valid for the clipboard's
/// whole lifetime — including shutdown, after `run_app` has dropped the event
/// loop. Keeping the pair in one type makes that ordering hard to break.
pub(crate) struct SystemClipboard {
    pub(crate) clipboard: Clipboard,
    pub(crate) _display: OwnedDisplayHandle,
}

impl SystemClipboard {
    pub(crate) fn new(event_loop: &dyn ActiveEventLoop) -> Self {
        let display = event_loop.owned_display_handle();
        let clipboard = unsafe {
            display
                .display_handle()
                .ok()
                .map(|h| Clipboard::new(h.as_raw()))
                .unwrap_or_default()
        };
        Self {
            clipboard,
            _display: display,
        }
    }

    pub(crate) fn set(&mut self, ty: ClipboardType, text: impl Into<String>) {
        self.clipboard.set(ty, text);
    }

    pub(crate) fn get(&mut self, ty: ClipboardType) -> String {
        self.clipboard.get(ty)
    }
}

pub(crate) struct AppState {
    pub(crate) window: Rc<dyn Window>,
    pub(crate) surface: Surface<Rc<dyn Window>, Rc<dyn Window>>,
    pub(crate) tabs: Vec<Tab>,
    pub(crate) active: usize,
    pub(crate) next_tab_id: usize,
    /// Primary face and its glyph atlas; the atlas handles fallback and caching.
    pub(crate) face: Face,
    pub(crate) atlas: Atlas,
    /// Reused per-frame buffers, kept here to avoid reallocating each redraw.
    pub(crate) rows: Vec<Row<Square>>,
    pub(crate) cells: Vec<render::CellRender>,
    pub(crate) cell_rects: Vec<Option<GlyphRect>>,
    /// Raw CLI options a new window inherits; the `--` command is not included.
    pub(crate) window_args: Vec<String>,
    /// Point size from the config, before DPI scaling.
    pub(crate) font_size: f32,
    /// Which modifier Page keys use to scroll the viewport.
    pub(crate) scrollback_modifier: ScrollModifier,
    pub(crate) cell_w: usize,
    pub(crate) cell_h: usize,
    pub(crate) modifiers: Modifiers,
    pub(crate) listener: Listener,
    pub(crate) cursor_pos: (f64, f64),
    pub(crate) mouse_buttons: [bool; 3], // left, middle, right
    pub(crate) local_pointer: LocalPointerState,
    /// Last left-press and click count in the sequence: (time, x, y, count).
    pub(crate) last_click: Option<(Instant, f64, f64, u8)>,
    pub(crate) clipboard: SystemClipboard,
    pub(crate) geometry_locked: bool,
    /// (timestamp, text) of the transient "cols x rows" overlay shown after a resize.
    pub(crate) resize_overlay: Option<(Instant, String)>,
    /// Whether the keyboard-shortcut help overlay is visible.
    pub(crate) key_help: bool,
    pub(crate) preset_index: usize,
}

/// Device-pixel cell pitch for a face at a point size, and an atlas using it.
fn atlas_for(face: &Face, font_size: f32, scale: f64) -> Atlas {
    let (width_pt, height_pt) = term_atlas::face_cell_for(face, font_size);
    let scale = scale as f32;
    let width = (width_pt * scale).round().max(1.0) as u32;
    let height = (height_pt * scale).round().max(1.0) as u32;
    Atlas::with_face_format(face.clone(), width, height, PixelFormat::Argb)
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        window: Rc<dyn Window>,
        surface: Surface<Rc<dyn Window>, Rc<dyn Window>>,
        tabs: Vec<Tab>,
        listener: Listener,
        clipboard: SystemClipboard,
        face: Face,
        font_size: f32,
        scrollback_modifier: ScrollModifier,
        window_args: Vec<String>,
    ) -> Result<Self, Error> {
        if !face.valid() {
            return Err(Error::Font("no usable font configured".to_string()));
        }
        let atlas = atlas_for(&face, font_size, window.scale_factor());
        let cell_w = atlas.cell().0 as usize;
        let cell_h = atlas.cell().1 as usize;
        let mut st = Self {
            window,
            surface,
            tabs,
            active: 0,
            next_tab_id: 1,
            face,
            atlas,
            rows: Vec::new(),
            cells: Vec::new(),
            cell_rects: Vec::new(),
            window_args,
            font_size,
            scrollback_modifier,
            cell_w,
            cell_h,
            modifiers: Modifiers::default(),
            listener,
            cursor_pos: (0.0, 0.0),
            mouse_buttons: [false; 3],
            local_pointer: LocalPointerState::Idle,
            last_click: None,
            clipboard,
            geometry_locked: false,
            resize_overlay: None,
            key_help: false,
            preset_index: 0,
        };
        st.sync_font_metrics(st.window.scale_factor())?;
        Ok(st)
    }

    pub(crate) fn sync_font_metrics(&mut self, scale: f64) -> Result<(), Error> {
        self.atlas = atlas_for(&self.face, self.font_size, scale);
        self.cell_w = self.atlas.cell().0 as usize;
        self.cell_h = self.atlas.cell().1 as usize;
        Ok(())
    }

    fn set_resize_lock(&mut self, locked: bool) {
        self.geometry_locked = locked;
        // Lock the window as well as the grid; otherwise dragging the window
        // still fires Resized events and changes the terminal behind the lock.
        self.window.set_resizable(!locked);
    }

    pub(crate) fn toggle_key_help(&mut self) {
        self.key_help = !self.key_help;
    }

    /// Run the action selected by the binding map.
    pub(crate) fn apply_action(
        &mut self,
        action: Action,
        event_loop: &dyn ActiveEventLoop,
    ) -> Result<(), Error> {
        match action {
            Action::NewTab => self.new_tab(),
            Action::CloseActiveTab => {
                self.close_active_tab(event_loop);
                Ok(())
            }
            Action::NextTab => {
                self.next_tab();
                Ok(())
            }
            Action::PrevTab => {
                self.prev_tab();
                Ok(())
            }
            Action::Copy => {
                self.copy_selection();
                Ok(())
            }
            Action::Paste => {
                self.paste_clipboard();
                Ok(())
            }
            Action::NewWindow => {
                self.spawn_new_window();
                Ok(())
            }
            Action::PresetNext => self.apply_preset(1),
            Action::PresetPrev => self.apply_preset(-1),
            Action::LockResize => {
                self.lock_toggle();
                Ok(())
            }
            Action::ToggleHelp => {
                self.toggle_key_help();
                Ok(())
            }
            Action::ScrollUp => {
                self.scroll_display(Scroll::PageUp);
                Ok(())
            }
            Action::ScrollDown => {
                self.scroll_display(Scroll::PageDown);
                Ok(())
            }
            // Pointer clicks are resolved with their tab index in input.rs.
            Action::SelectTab | Action::CloseTabAt => Ok(()),
        }
    }

    pub(crate) fn lock_toggle(&mut self) {
        let locked = !self.geometry_locked;
        self.set_resize_lock(locked);
    }

    pub(crate) fn scroll_display(&mut self, scroll: Scroll) {
        if let Some(tab) = self.tabs.get(self.active) {
            tab.terminal.lock().scroll_display(scroll);
        }
    }

    /// Spawn a fresh, independent uterm window/process with the same CLI
    /// options but no startup command, so it opens a shell.
    pub(crate) fn spawn_new_window(&self) {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let mut cmd = std::process::Command::new(exe);
        cmd.args(&self.window_args);
        if let Err(err) = cmd.spawn() {
            eprintln!("uterm: failed to spawn a new window: {err}");
        }
    }

    /// Apply a preset size in cells. An explicit size change clears the resize lock.
    pub(crate) fn apply_preset(&mut self, direction: i32) -> Result<(), Error> {
        let len = PRESET_SIZES.len() as i32;
        self.preset_index = (self.preset_index as i32 + direction).rem_euclid(len) as usize;
        self.set_resize_lock(false);
        let (cols, rows) = PRESET_SIZES[self.preset_index];
        let rows_with_bar = rows.checked_add(1).ok_or(Error::GridSize { cols, rows })?;
        let width = cells_to_px(cols, self.cell_w, cols, rows)?;
        let height = cells_to_px(rows_with_bar, self.cell_h, cols, rows)?;
        let _ = self
            .window
            .request_surface_size(Size::Physical(PhysicalSize::new(width, height)));
        Ok(())
    }

    pub(crate) fn show_resize_overlay(&mut self) {
        let (cols, rows) = self.grid_dims();
        self.resize_overlay = Some((Instant::now(), format!("{}x{}", cols, rows)));
    }

    pub(crate) fn set_geometry_from_size(&mut self, size: PhysicalSize<u32>) -> Result<(), Error> {
        // `size` and cell_w/cell_h are both physical pixels, so use it directly.
        let width = size.width as usize;
        let height = size.height as usize;
        if self.geometry_locked {
            return Ok(());
        }
        self.resize_all_tabs_with(width, height)
    }

    pub(crate) fn resize_all_tabs_with(
        &mut self,
        width: usize,
        height: usize,
    ) -> Result<(), Error> {
        let (cols, rows) = self.grid_dims_from(width, height);
        for tab in &self.tabs {
            tab.resize(cols, rows, self.cell_w, self.cell_h)?;
        }
        Ok(())
    }

    pub(crate) fn grid_dims(&self) -> (usize, usize) {
        let sz = self.window.surface_size();
        self.grid_dims_from(sz.width as usize, sz.height as usize)
    }

    pub(crate) fn grid_dims_from(&self, width: usize, height: usize) -> (usize, usize) {
        grid_dims_from(width, height, self.cell_w, self.cell_h)
    }
}

fn grid_dims_from(width: usize, height: usize, cell_w: usize, cell_h: usize) -> (usize, usize) {
    let max_cells = usize::from(u16::MAX);
    let cols = (width / cell_w.max(1)).clamp(2, max_cells);
    let total_rows = (height / cell_h.max(1)).clamp(2, max_cells);
    let content_rows = total_rows.saturating_sub(1).clamp(1, max_cells);
    (cols, content_rows)
}

impl AppState {
    pub(crate) fn copy_selection(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let text = {
            let term = tab.terminal.lock();
            term.selection_to_string()
        };
        if let Some(text) = text {
            if !text.is_empty() {
                self.clipboard.set(ClipboardType::Clipboard, text.clone());
                self.clipboard.set(ClipboardType::Selection, text);
            }
        }
    }

    pub(crate) fn update_primary_selection(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let text = {
            let term = tab.terminal.lock();
            term.selection_to_string()
        };
        if let Some(text) = text {
            if !text.is_empty() {
                self.clipboard.set(ClipboardType::Selection, text);
            }
        }
    }

    /// Insert text as if pasted, using bracketed paste when the app asked for it.
    pub(crate) fn paste_text(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let bracketed = {
            let term = tab.terminal.lock();
            term.mode().contains(Mode::BRACKETED_PASTE)
        };
        let payload = if bracketed {
            format!("\x1b[200~{text}\x1b[201~")
        } else {
            text
        };
        let _ = tab
            .pty
            .channel
            .send(Msg::Input(Cow::Owned(payload.into_bytes())));
    }

    pub(crate) fn paste_from(&mut self, ty: ClipboardType) {
        let mut text = self.clipboard.get(ty);
        if text.is_empty() && matches!(ty, ClipboardType::Selection) {
            text = self.clipboard.get(ClipboardType::Clipboard);
        }
        self.paste_text(text);
    }

    pub(crate) fn paste_clipboard(&mut self) {
        self.paste_from(ClipboardType::Clipboard);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_sizes_map_back_to_the_same_grid() {
        let (cell_w, cell_h) = (10, 20);
        for &(cols, rows) in PRESET_SIZES {
            let width = cols * cell_w;
            let height = (rows + 1) * cell_h;
            assert_eq!(grid_dims_from(width, height, cell_w, cell_h), (cols, rows));
        }
    }
}
