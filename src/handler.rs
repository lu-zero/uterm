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
use std::rc::Rc;
use term_atlas::Face;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::window::{Window, WindowId as WinitWindowId};

pub(crate) struct Uterm {
    pub(crate) proxy: EventLoopProxy<UiEvent>,
    pub(crate) state: Option<AppState>,
    pub(crate) geometry: Option<StartupGeometry>,
    pub(crate) face: Option<Face>,
    pub(crate) font_size: f32,
    pub(crate) command: Vec<String>,
    pub(crate) error: Option<Error>,
}

impl Uterm {
    fn fail(&mut self, event_loop: &ActiveEventLoop, err: Error) {
        if self.error.is_none() {
            self.error = Some(err);
        }
        event_loop.exit();
    }

    fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Error> {
        let window = Rc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("uterm")
                    .with_inner_size(LogicalSize::new(900.0, 620.0)),
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
        let mut st = AppState::new(
            window,
            surface,
            Vec::new(),
            self.proxy.clone(),
            clipboard,
            face,
            self.font_size,
            std::mem::take(&mut self.command),
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
                .request_inner_size(PhysicalSize::new(width, height));
            (g.cols, g.rows)
        } else {
            st.grid_dims()
        };
        let listener = Listener {
            proxy: self.proxy.clone(),
        };
        let first = Tab::new(0, cols, rows, st.cell_w, st.cell_h, listener, &st.command)?;
        st.tabs.push(first);

        st.sync_window_title();
        st.window.request_redraw();
        self.state = Some(st);
        Ok(())
    }
}

impl ApplicationHandler<UiEvent> for Uterm {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() || self.error.is_some() {
            return;
        }
        if let Err(err) = self.start(event_loop) {
            self.fail(event_loop, err);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UiEvent) {
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

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WinitWindowId,
        event: WindowEvent,
    ) {
        let Some(st) = &mut self.state else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => st.render(),
            WindowEvent::Resized(size) => {
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
                let size = st.window.inner_size();
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
            WindowEvent::DroppedFile(path) => {
                // Insert the path as text, never open or execute it.
                st.paste_text(path.to_string_lossy().into_owned());
            }
            WindowEvent::Focused(false) => {
                // No release event arrives after the focus loss.
                st.mouse_buttons = [false; 3];
                st.local_pointer = LocalPointerState::Idle;
            }
            WindowEvent::CursorMoved { position, .. } => {
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
            WindowEvent::MouseInput { state, button, .. } => {
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

                let app_cursor = st
                    .tabs
                    .get(st.active)
                    .map(|tab| tab.terminal.lock().mode().contains(Mode::APP_CURSOR))
                    .unwrap_or(false);
                if let Some(bytes) = key_to_bytes(&k, st.modifiers, app_cursor) {
                    if std::env::var_os("UTERM_KEY_DEBUG").is_some() {
                        eprintln!(
                            "key {:?}: {:?} (app_cursor={app_cursor})",
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

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
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
