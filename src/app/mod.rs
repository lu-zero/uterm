//! Application state and its supporting state types.

pub(crate) mod input;
pub(crate) mod render;
pub(crate) mod tabs;

use crate::cli::PRESET_SIZES;
use crate::error::Error;
use crate::event::UiEvent;
use crate::terminal::{cells_to_px, Tab};
use rio_vt::clipboard::{Clipboard, ClipboardType};
use rio_vt::crosswords::Mode;
use rio_vt::event::Msg;
use softbuffer::Surface;
use std::borrow::Cow;
use std::rc::Rc;
use std::time::{Duration, Instant};
use term_atlas::{Atlas, Face, PixelFormat};
use winit::dpi::PhysicalSize;
use winit::event::Modifiers;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy, OwnedDisplayHandle};
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
    pub(crate) fn new(event_loop: &ActiveEventLoop) -> Self {
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
    pub(crate) window: Rc<Window>,
    pub(crate) surface: Surface<Rc<Window>, Rc<Window>>,
    pub(crate) tabs: Vec<Tab>,
    pub(crate) active: usize,
    pub(crate) next_tab_id: usize,
    /// Primary face and its glyph atlas; the atlas handles fallback and caching.
    pub(crate) face: Face,
    pub(crate) atlas: Atlas,
    /// Command from `--`, used for every tab instead of the default shell.
    pub(crate) command: Vec<String>,
    /// Point size from the config, before DPI scaling.
    pub(crate) font_size: f32,
    pub(crate) cell_w: usize,
    pub(crate) cell_h: usize,
    pub(crate) modifiers: Modifiers,
    pub(crate) proxy: EventLoopProxy<UiEvent>,
    pub(crate) cursor_pos: (f64, f64),
    pub(crate) mouse_buttons: [bool; 3], // left, middle, right
    pub(crate) local_pointer: LocalPointerState,
    pub(crate) clipboard: SystemClipboard,
    pub(crate) geometry_locked: bool,
    /// (timestamp, text) of the transient "cols x rows" overlay shown after a resize.
    pub(crate) resize_overlay: Option<(Instant, String)>,
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
    pub(crate) fn new(
        window: Rc<Window>,
        surface: Surface<Rc<Window>, Rc<Window>>,
        tabs: Vec<Tab>,
        proxy: EventLoopProxy<UiEvent>,
        clipboard: SystemClipboard,
        face: Face,
        font_size: f32,
        command: Vec<String>,
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
            command,
            font_size,
            cell_w,
            cell_h,
            modifiers: Modifiers::default(),
            proxy,
            cursor_pos: (0.0, 0.0),
            mouse_buttons: [false; 3],
            local_pointer: LocalPointerState::Idle,
            clipboard,
            geometry_locked: false,
            resize_overlay: None,
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

    pub(crate) fn lock_toggle(&mut self) {
        self.geometry_locked = !self.geometry_locked;
    }

    /// Spawn a fresh, independent uterm window/process, inheriting our CLI args
    /// so it opens with the same geometry/profile.
    pub(crate) fn spawn_new_window(&self) {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let mut cmd = std::process::Command::new(exe);
        cmd.args(std::env::args().skip(1));
        if let Err(err) = cmd.spawn() {
            eprintln!("uterm: failed to spawn a new window: {err}");
        }
    }

    /// Apply a preset size in cells. An explicit size change clears the resize lock.
    pub(crate) fn apply_preset(&mut self, direction: i32) -> Result<(), Error> {
        let len = PRESET_SIZES.len() as i32;
        self.preset_index = (self.preset_index as i32 + direction).rem_euclid(len) as usize;
        self.geometry_locked = false;
        let (cols, rows) = PRESET_SIZES[self.preset_index];
        let rows_with_bar = rows.checked_add(1).ok_or(Error::GridSize { cols, rows })?;
        let width = cells_to_px(cols, self.cell_w, cols, rows)?;
        let height = cells_to_px(rows_with_bar, self.cell_h, cols, rows)?;
        let _ = self
            .window
            .request_inner_size(PhysicalSize::new(width, height));
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
        let sz = self.window.inner_size();
        self.grid_dims_from(sz.width as usize, sz.height as usize)
    }

    pub(crate) fn grid_dims_from(&self, width: usize, height: usize) -> (usize, usize) {
        let max_cells = usize::from(u16::MAX);
        let cols = (width / self.cell_w.max(1)).clamp(2, max_cells);
        let total_rows = (height / self.cell_h.max(1)).clamp(2, max_cells);
        let content_rows = total_rows.saturating_sub(1).clamp(1, max_cells);
        (cols, content_rows)
    }
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

    pub(crate) fn paste_from(&mut self, ty: ClipboardType) {
        let mut text = self.clipboard.get(ty);
        if text.is_empty() && matches!(ty, ClipboardType::Selection) {
            text = self.clipboard.get(ClipboardType::Clipboard);
        }
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

    pub(crate) fn paste_clipboard(&mut self) {
        self.paste_from(ClipboardType::Clipboard);
    }
}
