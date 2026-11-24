use crate::app::input::key_to_bytes;
use crate::app::{AppState, LocalPointerState, SystemClipboard, OVERLAY_TIMEOUT};
use crate::cli::StartupGeometry;
use crate::error::Error;
use crate::event::Listener;
use crate::event::UiEvent;
use crate::terminal::{cells_to_px, Tab};
use rio_vt::crosswords::grid::Scroll;
use rio_vt::crosswords::Mode;
use rio_vt::event::Msg;
use softbuffer::{Context, Surface};
use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use term_atlas::Face;
use winit::application::ApplicationHandler;
use winit::data_transfer::TypeHint;
use winit::dpi::{LogicalSize, PhysicalSize, Size};
use winit::event::{ButtonSource, ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, AsyncRequestSerial, ControlFlow, DndAction};
use winit::window::{Window, WindowAttributes, WindowId as WinitWindowId};

pub(crate) struct Uterm {
    pub(crate) listener: Listener,
    pub(crate) receiver: Receiver<UiEvent>,
    pub(crate) state: Option<AppState>,
    pub(crate) geometry: Option<StartupGeometry>,
    pub(crate) face: Option<Face>,
    pub(crate) font_size: f32,
    pub(crate) command: Vec<String>,
    /// Shared with `run` because `run_app` takes the handler by value.
    pub(crate) error: Rc<RefCell<Option<Error>>>,
    /// Serial of the in-flight drag-and-drop data request, if any.
    pub(crate) pending_drop: Option<AsyncRequestSerial>,
}

impl Uterm {
    fn fail(&mut self, event_loop: &dyn ActiveEventLoop, err: Error) {
        {
            let mut error = self.error.borrow_mut();
            if error.is_none() {
                *error = Some(err);
            }
        }
        event_loop.exit();
    }

    fn start(&mut self, event_loop: &dyn ActiveEventLoop) -> Result<(), Error> {
        let window: Rc<dyn Window> = Rc::from(
            event_loop.create_window(
                WindowAttributes::default()
                    .with_title("uterm")
                    .with_surface_size(LogicalSize::new(900.0, 620.0)),
            )?,
        );
        let context = Context::new(window.clone())?;
        let surface = Surface::new(&context, window.clone())?;

        let clipboard = SystemClipboard::new(event_loop);

        // Build state first so real font metrics are available when the first tab is created.
        let face = self
            .face
            .take()
            .ok_or_else(|| Error::Font("no font configured".into()))?;
        // Only the initial tab runs the startup command; new windows inherit
        // the CLI options but start with the default shell.
        let command = std::mem::take(&mut self.command);
        let window_args = crate::cli::new_window_args();
        let mut st = AppState::new(
            window,
            surface,
            Vec::new(),
            self.listener.clone(),
            clipboard,
            face,
            self.font_size,
            window_args,
        )?;

        let (cols, rows) = if let Some(g) = self.geometry {
            // Request the exact terminal pixel size (plus one row for the tab bar).
            let rows_with_bar = g.rows.checked_add(1).ok_or(Error::GridSize {
                cols: g.cols,
                rows: g.rows,
            })?;
            let width = cells_to_px(g.cols, st.cell_w, g.cols, g.rows)?;
            let height = cells_to_px(rows_with_bar, st.cell_h, g.cols, g.rows)?;
            let _ = st
                .window
                .request_surface_size(Size::Physical(PhysicalSize::new(width, height)));
            (g.cols, g.rows)
        } else {
            st.grid_dims()
        };
        let first = Tab::new(
            0,
            cols,
            rows,
            st.cell_w,
            st.cell_h,
            st.listener.clone(),
            &command,
        )?;
        st.tabs.push(first);

        st.sync_window_title();
        st.window.request_redraw();
        self.state = Some(st);
        Ok(())
    }

    fn handle_ui_event(&mut self, event_loop: &dyn ActiveEventLoop, event: UiEvent) {
        let Some(st) = &mut self.state else {
            return;
        };
        match event {
            UiEvent::Wakeup | UiEvent::TitleSync => {
                if let UiEvent::TitleSync = event {
                    st.sync_window_title();
                }
                st.window.request_redraw();
            }
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
                    let _ = tab
                        .pty
                        .channel
                        .send(Msg::Input(Cow::Owned(reply.into_bytes())));
                }
            }
        }
    }
}

impl ApplicationHandler for Uterm {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.state.is_some() || self.error.borrow().is_some() {
            return;
        }
        if let Err(err) = self.start(event_loop) {
            self.fail(event_loop, err);
        }
    }

    fn proxy_wake_up(&mut self, event_loop: &dyn ActiveEventLoop) {
        // Wake-ups are coalesced; the events themselves live in the channel.
        let events: Vec<_> = self.receiver.try_iter().collect();
        for event in events {
            self.handle_ui_event(event_loop, event);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        _id: WinitWindowId,
        event: WindowEvent,
    ) {
        let Some(st) = &mut self.state else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => st.render(),
            WindowEvent::SurfaceResized(size) => {
                // A minimized window reports 0x0; resizing the grid to the
                // clamped minimum would destroy scrollback for no reason.
                if size.width == 0 || size.height == 0 {
                    return;
                }
                if let Err(err) = st.set_geometry_from_size(size) {
                    self.fail(event_loop, err);
                    return;
                }
                st.show_resize_overlay();
                event_loop.set_control_flow(ControlFlow::wait_duration(OVERLAY_TIMEOUT));
                st.window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // Cell metrics are physical pixels; recompute them and the grid
                // when the window moves to a monitor with a different DPI.
                if let Err(err) = st.sync_font_metrics(scale_factor) {
                    self.fail(event_loop, err);
                    return;
                }
                let size = st.window.surface_size();
                if let Err(err) = st.resize_all_tabs_with(size.width as usize, size.height as usize)
                {
                    self.fail(event_loop, err);
                    return;
                }
                st.window.request_redraw();
            }
            WindowEvent::ModifiersChanged(m) => {
                st.modifiers = m;
            }
            WindowEvent::DragEntered { id, .. } => {
                // Accept the drag so the OS sends DragDropped rather than DragLeft.
                let _ = event_loop.set_valid_dnd_actions(id, &[DndAction::Copy]);
            }
            WindowEvent::DragDropped { id, .. } => {
                // Prefer a URI list (file drops), falling back to plain text.
                // In both cases the data is pasted as text below; never opened.
                let kind = event_loop.data_transfer(id).ok().and_then(|transfer| {
                    if transfer.has_type(&TypeHint::UriList) {
                        Some(TypeHint::UriList)
                    } else if transfer.has_type(&TypeHint::Plaintext) {
                        Some(TypeHint::Plaintext)
                    } else {
                        None
                    }
                });
                if let Some(kind) = kind {
                    if let Ok(serial) = event_loop.fetch_data_transfer(id, &kind) {
                        self.pending_drop = Some(serial);
                    }
                }
            }
            WindowEvent::DragLeft { .. } => {
                self.pending_drop = None;
            }
            WindowEvent::DataTransferReceived { serial, value, .. } => {
                if self.pending_drop != Some(serial) {
                    return;
                }
                self.pending_drop = None;
                let text = match value.type_().hint() {
                    Some(TypeHint::UriList) => value.try_as_uris().ok().map(|uris| {
                        uris.iter()
                            .map(|uri| uri_to_text(uri))
                            .collect::<Vec<_>>()
                            .join("\n")
                    }),
                    Some(TypeHint::Plaintext) => value.try_as_string().ok(),
                    _ => None,
                };
                if let Some(text) = text {
                    st.paste_text(text);
                    st.window.request_redraw();
                }
            }
            WindowEvent::Focused(false) => {
                // No release event arrives after the focus loss.
                st.mouse_buttons = [false; 3];
                st.local_pointer = LocalPointerState::Idle;
            }
            WindowEvent::PointerMoved { position, .. } => {
                st.cursor_pos = (position.x, position.y);
                if st.mouse_report_mode() {
                    st.local_pointer = LocalPointerState::Idle;
                    if let Some((col, row, _)) = st.terminal_cell_at(position.x, position.y) {
                        st.send_mouse_motion(col as u16, row as u16);
                    }
                } else if st.local_pointer == LocalPointerState::Selecting && st.mouse_buttons[0] {
                    st.update_local_selection(position.x, position.y);
                    st.window.request_redraw();
                }
            }
            WindowEvent::PointerButton {
                state,
                button: ButtonSource::Mouse(button),
                ..
            } => {
                st.handle_mouse_press(button, state, event_loop);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if let Some((col, row, _)) = st.terminal_cell_at(st.cursor_pos.0, st.cursor_pos.1) {
                    let lines = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y as i32,
                        MouseScrollDelta::PixelDelta(pos) => {
                            if pos.y.abs() < 1.0 {
                                0
                            } else {
                                (pos.y / st.cell_h as f64).round() as i32
                            }
                        }
                        _ => 0,
                    };
                    if lines != 0 {
                        let (btn, count) = if lines > 0 {
                            (64u8, lines)
                        } else {
                            (65u8, -lines)
                        };
                        for _ in 0..count.min(10) {
                            if st.mouse_report_mode() {
                                st.send_wheel_motion(btn, col as u16, row as u16);
                            } else if let Some(tab) = st.tabs.get(st.active) {
                                // No application grabbed the mouse: scroll the
                                // viewport ourselves instead of reporting nothing.
                                tab.terminal.lock().scroll_display(Scroll::Delta(lines));
                                st.window.request_redraw();
                            }
                        }
                    }
                }
            }
            WindowEvent::KeyboardInput { event: k, .. } => {
                if k.state != ElementState::Pressed {
                    return;
                }
                let mods = st.modifiers.state();
                if let Some(action) = crate::keybindings::lookup_key(mods, &k.logical_key) {
                    if std::env::var_os("UTERM_KEY_DEBUG").is_some() {
                        eprintln!(
                            "binding {:?} for {:?} (mods={mods:?})",
                            action, k.logical_key
                        );
                    }
                    // A held new-window shortcut must not spawn a process per repeat.
                    if action == crate::keybindings::Action::NewWindow && k.repeat {
                        return;
                    }
                    if let Err(err) = st.apply_action(action, event_loop) {
                        self.fail(event_loop, err);
                        return;
                    }
                    st.window.request_redraw();
                    return;
                }

                let (app_cursor, app_keypad) = st
                    .tabs
                    .get(st.active)
                    .map(|tab| {
                        let mode = tab.terminal.lock().mode();
                        (
                            mode.contains(Mode::APP_CURSOR),
                            mode.contains(Mode::APP_KEYPAD),
                        )
                    })
                    .unwrap_or((false, false));
                if let Some(bytes) = key_to_bytes(&k, st.modifiers, app_cursor, app_keypad) {
                    if std::env::var_os("UTERM_KEY_DEBUG").is_some() {
                        eprintln!(
                            "key {:?}: {:?} (app_cursor={app_cursor}, app_keypad={app_keypad})",
                            k.logical_key, bytes
                        );
                    }
                    if let Some(tab) = st.tabs.get(st.active) {
                        let _ = tab.pty.channel.send(Msg::Input(Cow::Owned(bytes)));
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        let Some(st) = &mut self.state else {
            return;
        };
        if let Some((since, _)) = st.resize_overlay {
            if since.elapsed() >= OVERLAY_TIMEOUT {
                st.resize_overlay = None;
                st.window.request_redraw();
                event_loop.set_control_flow(ControlFlow::Wait);
            }
        }
    }
}

/// Turn a dropped `file:` URI into the plain path text that uterm inserts.
/// The text is only ever pasted into the terminal, never opened or executed.
fn uri_to_text(uri: &str) -> String {
    let path = uri
        .strip_prefix("file://")
        .map(|rest| rest.strip_prefix("localhost").unwrap_or(rest))
        .unwrap_or(uri);
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::uri_to_text;

    #[test]
    fn dropped_file_uris_become_plain_path_text() {
        assert_eq!(uri_to_text("file:///tmp/a%20b.txt"), "/tmp/a b.txt");
        assert_eq!(uri_to_text("file://localhost/tmp/x"), "/tmp/x");
        assert_eq!(uri_to_text("https://example.com"), "https://example.com");
        assert_eq!(uri_to_text("%2Ftmp%2Fx"), "/tmp/x");
    }
}
