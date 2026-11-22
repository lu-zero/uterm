use super::{AppState, LocalPointerState};
use rio_vt::clipboard::ClipboardType;
use rio_vt::crosswords::pos::{Column, Line, Pos, Side};
use rio_vt::crosswords::Mode;
use rio_vt::event::Msg;
use rio_vt::selection::{Selection, SelectionType};
use std::borrow::Cow;
use winit::event::{ElementState, Modifiers, MouseButton};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};

impl AppState {
    pub(crate) fn terminal_cell_at(&self, x: f64, y: f64) -> Option<(usize, usize, Side)> {
        let bar_h = self.cell_h as f64;
        let ty = y - bar_h;
        if ty < 0.0 || x < 0.0 {
            return None;
        }
        let (cols, rows) = self.grid_dims();
        let col_f = x / self.cell_w as f64;
        let row_f = ty / self.cell_h as f64;
        let col = col_f.floor() as i64;
        let row = row_f.floor() as i64;
        if col < 0 || row < 0 {
            return None;
        }
        let col = (col as usize).min(cols.saturating_sub(1));
        let row = (row as usize).min(rows.saturating_sub(1));
        let side = if col_f - (col as f64) < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        Some((col, row, side))
    }

    pub(crate) fn mouse_report_mode(&self) -> bool {
        let Some(tab) = self.tabs.get(self.active) else {
            return false;
        };
        let term = tab.terminal.lock();
        term.mode().intersects(Mode::MOUSE_MODE)
    }

    pub(crate) fn send_mouse_report(&self, button: u8, col: u16, row: u16, pressed: bool) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let (sgr, utf8, mouse_mode) = {
            let term = tab.terminal.lock();
            let mode = term.mode();
            (
                mode.contains(Mode::SGR_MOUSE),
                mode.contains(Mode::UTF8_MOUSE),
                mode.intersects(Mode::MOUSE_MODE),
            )
        };
        if !mouse_mode {
            return;
        }
        let bytes = encode_mouse_report(button, col, row, pressed, sgr, utf8);
        if std::env::var_os("UTERM_MOUSE_DEBUG").is_some() {
            eprintln!("mouse report: {bytes:?} (sgr={sgr}, utf8={utf8})");
        }
        let _ = tab.pty.channel.send(Msg::Input(Cow::Owned(bytes)));
    }

    pub(crate) fn send_mouse_motion(&self, col: u16, row: u16) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let (sgr, utf8, report_motion, report_drag) = {
            let term = tab.terminal.lock();
            let mode = term.mode();
            (
                mode.contains(Mode::SGR_MOUSE),
                mode.contains(Mode::UTF8_MOUSE),
                mode.contains(Mode::MOUSE_MOTION),
                mode.contains(Mode::MOUSE_DRAG),
            )
        };
        let any_button = self.mouse_buttons.iter().any(|&b| b);
        if !report_motion && !(report_drag && any_button) {
            return;
        }
        let button = motion_button(
            self.mouse_buttons[0],
            self.mouse_buttons[1],
            self.mouse_buttons[2],
        );
        let bytes = encode_mouse_report(button, col, row, true, sgr, utf8);
        let _ = tab.pty.channel.send(Msg::Input(Cow::Owned(bytes)));
    }

    pub(crate) fn send_wheel_motion(&self, button: u8, col: u16, row: u16) {
        // Report wheel as button motion (64/65) during wheel reporting.
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let (sgr, utf8) = {
            let term = tab.terminal.lock();
            let mode = term.mode();
            (
                mode.contains(Mode::SGR_MOUSE),
                mode.contains(Mode::UTF8_MOUSE),
            )
        };
        let bytes = encode_mouse_report(button, col, row, true, sgr, utf8);
        let _ = tab.pty.channel.send(Msg::Input(Cow::Owned(bytes)));
    }

    pub(crate) fn handle_mouse_press(
        &mut self,
        button: MouseButton,
        state: ElementState,
        event_loop: &dyn ActiveEventLoop,
    ) {
        let (x, y) = self.cursor_pos;
        if std::env::var_os("UTERM_MOUSE_DEBUG").is_some() {
            let cell = self.terminal_cell_at(x, y).map(|(col, row, _)| (col, row));
            eprintln!(
                "mouse {button:?} {state:?}: pos=({x:.1},{y:.1}) cell={}x{} -> {cell:?}",
                self.cell_w, self.cell_h
            );
        }
        if state == ElementState::Pressed && button == MouseButton::Left {
            if let Some((idx, close)) = self.tab_bar_hit(x, y) {
                match crate::keybindings::mouse_action(close) {
                    crate::keybindings::Action::CloseTabAt => self.close_tab_at(idx, event_loop),
                    crate::keybindings::Action::SelectTab => self.select_tab(idx),
                    _ => unreachable!("mouse bindings resolve to tab actions"),
                }
                self.local_pointer = LocalPointerState::Idle;
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
            self.local_pointer = LocalPointerState::Idle;
            self.mouse_buttons[idx] = state == ElementState::Pressed;
            if let Some((col, row, _)) = self.terminal_cell_at(x, y) {
                self.send_mouse_report(
                    btn_code,
                    col as u16,
                    row as u16,
                    state == ElementState::Pressed,
                );
            }
            return;
        }

        match (button, state) {
            (MouseButton::Left, ElementState::Pressed) => {
                self.mouse_buttons[0] = true;
                self.begin_local_selection(x, y);
            }
            (MouseButton::Left, ElementState::Released) => {
                self.mouse_buttons[0] = false;
                self.finish_local_selection(x, y);
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

    pub(crate) fn begin_local_selection(&mut self, x: f64, y: f64) {
        self.local_pointer = LocalPointerState::Idle;
        if let Some((col, row, side)) = self.terminal_cell_at(x, y) {
            if let Some(tab) = self.tabs.get(self.active) {
                let mut term = tab.terminal.lock();
                let point = Pos::new(Line(row as i32), Column(col));
                term.selection = Some(Selection::new(SelectionType::Simple, point, side));
            }
            self.local_pointer = LocalPointerState::Selecting;
        }
    }

    pub(crate) fn update_local_selection(&mut self, x: f64, y: f64) {
        if self.local_pointer != LocalPointerState::Selecting || !self.mouse_buttons[0] {
            return;
        }
        if let Some((col, row, side)) = self.terminal_cell_at(x, y) {
            if let Some(tab) = self.tabs.get(self.active) {
                let mut term = tab.terminal.lock();
                if let Some(sel) = &mut term.selection {
                    sel.update(Pos::new(Line(row as i32), Column(col)), side);
                }
            }
        }
    }

    pub(crate) fn finish_local_selection(&mut self, x: f64, y: f64) {
        if self.local_pointer == LocalPointerState::Selecting {
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
        self.local_pointer = LocalPointerState::Idle;
    }
}

/// Drag/motion button parameter: 32..=34 for drags, 35 for plain motion.
/// `encode_mouse_report` applies the extra +32 that X10/UTF-8 need.
fn motion_button(left: bool, middle: bool, right: bool) -> u8 {
    if left {
        32
    } else if middle {
        33
    } else if right {
        34
    } else {
        35
    }
}

pub(crate) fn encode_mouse_report(
    button: u8,
    col: u16,
    row: u16,
    pressed: bool,
    sgr: bool,
    utf8: bool,
) -> Vec<u8> {
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

pub(crate) fn ctrl_char(ch: char) -> Option<u8> {
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

/// Number of a top-row function key, `1` for F1 through `12` for F12.
fn function_key_index(key: NamedKey) -> Option<u8> {
    Some(match key {
        NamedKey::F1 => 1,
        NamedKey::F2 => 2,
        NamedKey::F3 => 3,
        NamedKey::F4 => 4,
        NamedKey::F5 => 5,
        NamedKey::F6 => 6,
        NamedKey::F7 => 7,
        NamedKey::F8 => 8,
        NamedKey::F9 => 9,
        NamedKey::F10 => 10,
        NamedKey::F11 => 11,
        NamedKey::F12 => 12,
        _ => return None,
    })
}

/// Bytes for F1–F12 as `xterm-256color` and `rio` list them (`kf1`–`kf63`).
///
/// Unmodified F1–F4 are SS3 (`\EOP`…`\EOS`). F5–F12 are CSI, skipping 16 and
/// 22 (`\E[15~`, `\E[17~`, …, `\E[24~`). A modifier uses the xterm parameter
/// (2 shift, 3 alt, 4 both, 5 ctrl, and the combinations of those): `\E[1;{mod}P`
/// for F1–F4 and `\E[{code};{mod}~` for F5–F12. Midnight Commander reads these
/// from terminfo after it turns the keypad on.
fn function_key_bytes(n: u8, shift: bool, alt: bool, ctrl: bool) -> Option<Vec<u8>> {
    let (ss3, code) = match n {
        1 => (b'P', 0),
        2 => (b'Q', 0),
        3 => (b'R', 0),
        4 => (b'S', 0),
        5 => (0, 15),
        6 => (0, 17),
        7 => (0, 18),
        8 => (0, 19),
        9 => (0, 20),
        10 => (0, 21),
        11 => (0, 23),
        12 => (0, 24),
        _ => return None,
    };
    let modifier = u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    if modifier == 0 {
        return Some(if n <= 4 {
            vec![0x1b, b'O', ss3]
        } else {
            format!("\x1b[{code}~").into_bytes()
        });
    }
    let param = modifier + 1;
    Some(if n <= 4 {
        format!("\x1b[1;{param}{}", ss3 as char).into_bytes()
    } else {
        format!("\x1b[{code};{param}~").into_bytes()
    })
}

/// Unmodified cursor keys use SS3 when the application enabled DECCKM (`?1h`),
/// matching xterm's terminfo (`kcuu1=\EOA`), and CSI otherwise. Modified keys
/// stay in the parameterized CSI form in both modes.
fn cursor_key_bytes(key: NamedKey, app_cursor: bool, modifiers: Modifiers) -> Option<Vec<u8>> {
    let final_byte = match key {
        NamedKey::ArrowUp => b'A',
        NamedKey::ArrowDown => b'B',
        NamedKey::ArrowRight => b'C',
        NamedKey::ArrowLeft => b'D',
        NamedKey::Home => b'H',
        NamedKey::End => b'F',
        _ => return None,
    };
    let state = modifiers.state();
    let shift = state.shift_key();
    let alt = state.alt_key();
    let ctrl = state.control_key();
    let modifier = u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    Some(if modifier == 0 {
        if app_cursor {
            vec![0x1b, b'O', final_byte]
        } else {
            vec![0x1b, b'[', final_byte]
        }
    } else {
        format!("\x1b[1;{}{}", modifier + 1, final_byte as char).into_bytes()
    })
}

pub(crate) fn key_to_bytes(
    k: &winit::event::KeyEvent,
    modifiers: Modifiers,
    app_cursor: bool,
) -> Option<Vec<u8>> {
    let ctrl = modifiers.state().control_key();
    let alt = modifiers.state().alt_key();
    if let Key::Named(key) = k.logical_key {
        if let Some(n) = function_key_index(key) {
            let shift = modifiers.state().shift_key();
            return function_key_bytes(n, shift, alt, ctrl);
        }
    }
    if ctrl {
        if let Key::Character(s) = &k.logical_key {
            if let Some(ch) = s.chars().next() {
                if let Some(b) = ctrl_char(ch) {
                    let mut out = vec![b];
                    // Ctrl+Space is NUL. In winit 0.31 space is a Character,
                    // so keep the old Ctrl+Alt+Space behaviour of sending just NUL.
                    if alt && ch != ' ' {
                        out.insert(0, 0x1b);
                    }
                    return Some(out);
                }
            }
        }
    }
    match &k.logical_key {
        Key::Character(s) => {
            if !ctrl && !alt {
                if let Some(text) = &k.text {
                    if !text.is_empty() {
                        return Some(text.as_bytes().to_vec());
                    }
                }
            }
            if ctrl {
                return None;
            }
            let b = if let Some(text) = &k.text {
                if !text.is_empty() {
                    text.as_bytes().to_vec()
                } else {
                    s.as_bytes().to_vec()
                }
            } else {
                s.as_bytes().to_vec()
            };
            if b.is_empty() {
                return None;
            }
            if alt {
                let mut out = vec![0x1b];
                out.extend_from_slice(&b);
                Some(out)
            } else {
                Some(b)
            }
        }
        Key::Named(NamedKey::Enter) => Some(b"\r".to_vec()),
        Key::Named(NamedKey::Backspace) => Some(if ctrl { vec![0x08] } else { vec![0x7f] }),
        Key::Named(NamedKey::Tab) => Some(b"\t".to_vec()),
        Key::Named(NamedKey::Escape) => Some(vec![0x1b]),
        // Space is `Key::Character(" ")` since winit 0.31; the generic
        // `Character` arm above produces the same bytes as the old branch.
        Key::Named(
            named @ (NamedKey::ArrowUp
            | NamedKey::ArrowDown
            | NamedKey::ArrowRight
            | NamedKey::ArrowLeft
            | NamedKey::Home
            | NamedKey::End),
        ) => cursor_key_bytes(*named, app_cursor, modifiers),
        Key::Named(NamedKey::PageUp) => Some(b"\x1b[5~".to_vec()),
        Key::Named(NamedKey::PageDown) => Some(b"\x1b[6~".to_vec()),
        Key::Named(NamedKey::Delete) => Some(b"\x1b[3~".to_vec()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cursor_key_bytes, encode_mouse_report, function_key_bytes, function_key_index,
        motion_button,
    };
    use winit::event::Modifiers;
    use winit::keyboard::{ModifiersState, NamedKey};

    #[test]
    fn cursor_keys_follow_application_cursor_mode() {
        let none = Modifiers::from(ModifiersState::empty());
        assert_eq!(
            cursor_key_bytes(NamedKey::ArrowUp, false, none).as_deref(),
            Some(&b"\x1b[A"[..])
        );
        assert_eq!(
            cursor_key_bytes(NamedKey::ArrowUp, true, none).as_deref(),
            Some(&b"\x1bOA"[..])
        );
        assert_eq!(
            cursor_key_bytes(NamedKey::ArrowLeft, true, none).as_deref(),
            Some(&b"\x1bOD"[..])
        );
        assert_eq!(
            cursor_key_bytes(NamedKey::Home, true, none).as_deref(),
            Some(&b"\x1bOH"[..])
        );
        let shift = Modifiers::from(ModifiersState::SHIFT);
        assert_eq!(
            cursor_key_bytes(NamedKey::ArrowUp, true, shift).as_deref(),
            Some(&b"\x1b[1;2A"[..])
        );
        assert!(cursor_key_bytes(NamedKey::Enter, true, none).is_none());
    }

    #[test]
    fn motion_button_matches_the_reporting_protocol() {
        assert_eq!(motion_button(true, false, false), 32);
        assert_eq!(motion_button(false, false, false), 35);
        // The wire form adds 32 for X10/UTF-8, but not for SGR.
        assert_eq!(
            encode_mouse_report(motion_button(true, false, false), 0, 0, true, false, false),
            b"\x1b[M@!!"
        );
        assert_eq!(
            encode_mouse_report(motion_button(false, false, false), 0, 0, true, false, false),
            b"\x1b[MC!!"
        );
        assert_eq!(
            encode_mouse_report(motion_button(true, false, false), 0, 0, true, true, false),
            b"\x1b[<32;1;1M"
        );
    }

    #[test]
    fn mouse_reports_use_x10_and_sgr_encoding() {
        // SGR (1006): 1-based coordinates, M for press and m for release.
        assert_eq!(
            encode_mouse_report(0, 9, 4, true, true, false),
            b"\x1b[<0;10;5M"
        );
        assert_eq!(
            encode_mouse_report(2, 9, 4, false, true, false),
            b"\x1b[<2;10;5m"
        );
        // X10: 32 + button, with bit 2 set on release.
        assert_eq!(
            encode_mouse_report(0, 0, 0, true, false, false),
            b"\x1b[M !!"
        );
        assert_eq!(
            encode_mouse_report(0, 0, 0, false, false, false),
            b"\x1b[M#!!"
        );
    }

    #[test]
    fn function_keys_match_xterm_terminfo() {
        assert_eq!(function_key_index(NamedKey::F2), Some(2));
        assert_eq!(function_key_index(NamedKey::Enter), None);

        assert_eq!(
            function_key_bytes(1, false, false, false).as_deref(),
            Some(&b"\x1bOP"[..])
        );
        assert_eq!(
            function_key_bytes(2, false, false, false).as_deref(),
            Some(&b"\x1bOQ"[..])
        );
        assert_eq!(
            function_key_bytes(3, false, false, false).as_deref(),
            Some(&b"\x1bOR"[..])
        );
        assert_eq!(
            function_key_bytes(4, false, false, false).as_deref(),
            Some(&b"\x1bOS"[..])
        );
        assert_eq!(
            function_key_bytes(5, false, false, false).as_deref(),
            Some(&b"\x1b[15~"[..])
        );
        assert_eq!(
            function_key_bytes(6, false, false, false).as_deref(),
            Some(&b"\x1b[17~"[..])
        );
        assert_eq!(
            function_key_bytes(11, false, false, false).as_deref(),
            Some(&b"\x1b[23~"[..])
        );
        assert_eq!(
            function_key_bytes(12, false, false, false).as_deref(),
            Some(&b"\x1b[24~"[..])
        );

        // kf14 / kf49 / kf29 / kf48: shift-F2, alt-F1, ctrl-F5, ctrl+shift-F12.
        assert_eq!(
            function_key_bytes(2, true, false, false).as_deref(),
            Some(&b"\x1b[1;2Q"[..])
        );
        assert_eq!(
            function_key_bytes(1, false, true, false).as_deref(),
            Some(&b"\x1b[1;3P"[..])
        );
        assert_eq!(
            function_key_bytes(5, false, false, true).as_deref(),
            Some(&b"\x1b[15;5~"[..])
        );
        assert_eq!(
            function_key_bytes(12, true, false, true).as_deref(),
            Some(&b"\x1b[24;6~"[..])
        );
        assert_eq!(
            function_key_bytes(2, true, true, true).as_deref(),
            Some(&b"\x1b[1;8Q"[..])
        );

        assert!(function_key_bytes(0, false, false, false).is_none());
        assert!(function_key_bytes(13, false, false, false).is_none());
    }
}
