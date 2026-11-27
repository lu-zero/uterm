use super::{AppState, Click, LocalPointerState};
use rio_vt::clipboard::ClipboardType;
use rio_vt::crosswords::grid::Dimensions;
use rio_vt::crosswords::pos::{Column, Line, Pos, Side};
use rio_vt::crosswords::square::Wide;
use rio_vt::crosswords::Crosswords;
use rio_vt::crosswords::Mode;
use rio_vt::event::{EventListener, Msg};
use rio_vt::selection::{Selection, SelectionType};
use std::borrow::Cow;
use std::time::{Duration, Instant};
use winit::event::{ElementState, Modifiers, MouseButton};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, KeyCode, KeyLocation, NamedKey, PhysicalKey};

const DOUBLE_CLICK: Duration = Duration::from_millis(500);

/// Character range of the word under `index`, bounded by whitespace. A word
/// that looks like a URL is bounded by its trailing punctuation instead, so
/// the whole address is selected.
fn selection_bounds(chars: &[char], index: usize) -> Option<(usize, usize)> {
    let separator = |ch: char| ch.is_whitespace() || ch == '\0';
    let ch = *chars.get(index)?;
    if separator(ch) {
        return None;
    }
    let mut start = index;
    let mut end = index;
    while start > 0 && !separator(chars[start - 1]) {
        start -= 1;
    }
    while end + 1 < chars.len() && !separator(chars[end + 1]) {
        end += 1;
    }
    let word: String = chars[start..=end].iter().collect();
    if word.contains("://") || word.contains("www.") {
        while end > start
            && matches!(
                chars[end],
                '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '>' | '\'' | '"'
            )
        {
            end -= 1;
        }
        while start <= end && matches!(chars[start], '(' | '<' | '[' | '{') {
            start += 1;
        }
    }
    (start <= end).then_some((start, end))
}

/// Selection covering a whole logical line, including wrapped rows, without
/// the trailing newline that `SelectionType::Lines` would add.
fn line_selection<U: EventListener>(term: &Crosswords<U>, row: usize) -> Option<Selection> {
    if row >= term.grid.screen_lines() {
        return None;
    }
    let line = Line(row as i32 - term.display_offset() as i32);
    let point = Pos::new(line, Column(0));
    let start = term.row_search_left(point);
    let end = term.row_search_right(point);
    let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
    selection.update(end, Side::Right);
    Some(selection)
}

/// Whether this grid line wraps into the next one.
fn row_wrapped<U: EventListener>(term: &Crosswords<U>, line: Line, cols: usize) -> bool {
    term.grid[Pos::new(line, Column(cols - 1))].wrapline()
}

/// Selection covering the word (or URL) under a cell, including rows joined
/// by line wrapping so long addresses survive a wrap.
fn word_selection<U: EventListener>(
    term: &Crosswords<U>,
    row: usize,
    col: usize,
) -> Option<Selection> {
    let cols = term.grid.columns();
    if col >= cols {
        return None;
    }
    let clicked = Line(row as i32 - term.display_offset() as i32);
    let mut first = clicked.0;
    while first > term.grid.topmost_line().0 && row_wrapped(term, Line(first - 1), cols) {
        first -= 1;
    }
    let mut last = clicked.0;
    while row_wrapped(term, Line(last), cols) && last + 1 < term.grid.bottommost_line().0 {
        last += 1;
    }
    let mut cells = Vec::new();
    for line in first..=last {
        for c in 0..cols {
            let square = term.grid[Pos::new(Line(line), Column(c))];
            if matches!(square.wide(), Wide::Spacer | Wide::LeadingSpacer) {
                continue;
            }
            cells.push((line, c, square.c()));
        }
    }
    let index = cells
        .iter()
        .position(|&(line, c, _)| line == clicked.0 && c == col)
        .or_else(|| {
            cells
                .iter()
                .rposition(|&(line, c, _)| line < clicked.0 || (line == clicked.0 && c < col))
        })?;
    let chars: Vec<char> = cells.iter().map(|&(_, _, ch)| ch).collect();
    let (start, end) = selection_bounds(&chars, index)?;
    let (start_line, start_col, _) = cells[start];
    let (end_line, end_col, _) = cells[end];
    let mut selection = Selection::new(
        SelectionType::Simple,
        Pos::new(Line(start_line), Column(start_col)),
        Side::Left,
    );
    selection.update(Pos::new(Line(end_line), Column(end_col)), Side::Right);
    Some(selection)
}

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

    fn mouse_mode(&self) -> Option<Mode> {
        Some(self.tabs.get(self.active)?.terminal.lock().mode())
    }

    fn send_mouse_bytes(&self, bytes: Vec<u8>) {
        if let Some(tab) = self.tabs.get(self.active) {
            let _ = tab.pty.channel.send(Msg::Input(Cow::Owned(bytes)));
        }
    }

    pub(crate) fn mouse_report_mode(&self) -> bool {
        self.mouse_mode()
            .is_some_and(|mode| mode.intersects(Mode::MOUSE_MODE))
    }

    pub(crate) fn send_mouse_report(&self, button: u8, col: u16, row: u16, pressed: bool) {
        let Some(mode) = self.mouse_mode().filter(|m| m.intersects(Mode::MOUSE_MODE)) else {
            return;
        };
        let sgr = mode.contains(Mode::SGR_MOUSE);
        let utf8 = mode.contains(Mode::UTF8_MOUSE);
        let bytes = encode_mouse_report(button, col, row, pressed, sgr, utf8);
        if std::env::var_os("UTERM_MOUSE_DEBUG").is_some() {
            eprintln!("mouse report: {bytes:?} (sgr={sgr}, utf8={utf8})");
        }
        self.send_mouse_bytes(bytes);
    }

    pub(crate) fn send_mouse_motion(&self, col: u16, row: u16) {
        let Some(mode) = self.mouse_mode() else {
            return;
        };
        let any_button = self.mouse_buttons.iter().any(|&b| b);
        if !mode.contains(Mode::MOUSE_MOTION) && !(mode.contains(Mode::MOUSE_DRAG) && any_button) {
            return;
        }
        let button = motion_button(
            self.mouse_buttons[0],
            self.mouse_buttons[1],
            self.mouse_buttons[2],
        );
        let bytes = encode_mouse_report(
            button,
            col,
            row,
            true,
            mode.contains(Mode::SGR_MOUSE),
            mode.contains(Mode::UTF8_MOUSE),
        );
        self.send_mouse_bytes(bytes);
    }

    pub(crate) fn send_wheel_motion(&self, button: u8, col: u16, row: u16) {
        // Report wheel as button motion (64/65) during wheel reporting.
        let Some(mode) = self.mouse_mode() else {
            return;
        };
        let bytes = encode_mouse_report(
            button,
            col,
            row,
            true,
            mode.contains(Mode::SGR_MOUSE),
            mode.contains(Mode::UTF8_MOUSE),
        );
        self.send_mouse_bytes(bytes);
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
                if close {
                    self.remove_tab_index(idx, event_loop);
                } else {
                    self.select_tab(idx);
                }
                self.reset_pointer();
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
        let Some((col, row, side)) = self.terminal_cell_at(x, y) else {
            return;
        };
        let click = Click {
            at: Instant::now(),
            x,
            y,
        };
        let sequence =
            self.clicks
                .advance(click, self.cell_w as f64, self.cell_h as f64, DOUBLE_CLICK);
        let count = sequence.count();
        if std::env::var_os("UTERM_MOUSE_DEBUG").is_some() {
            eprintln!("click {col},{row} at ({x:.1},{y:.1}): count={count}");
        }
        self.clicks = sequence;
        if count >= 3 {
            self.select_line(row);
            self.update_primary_selection();
            return;
        }
        if count == 2 {
            self.select_word_or_url(row, col);
            self.update_primary_selection();
            return;
        }
        if let Some(tab) = self.tabs.get(self.active) {
            let mut term = tab.terminal.lock();
            let line = Line(row as i32 - term.display_offset() as i32);
            let point = Pos::new(line, Column(col));
            term.selection = Some(Selection::new(SelectionType::Simple, point, side));
        }
        self.local_pointer = LocalPointerState::Selecting;
    }

    /// Select the whitespace-delimited word under the cursor, or the whole
    /// URL when the word looks like one.
    fn select_word_or_url(&mut self, row: usize, col: usize) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let mut term = tab.terminal.lock();
        if let Some(selection) = word_selection(&term, row, col) {
            term.selection = Some(selection);
        }
    }

    fn select_line(&mut self, row: usize) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let mut term = tab.terminal.lock();
        if let Some(selection) = line_selection(&term, row) {
            term.selection = Some(selection);
        }
    }

    fn update_selection_at(&self, x: f64, y: f64) {
        let Some((col, row, side)) = self.terminal_cell_at(x, y) else {
            return;
        };
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let mut term = tab.terminal.lock();
        let line = Line(row as i32 - term.display_offset() as i32);
        if let Some(sel) = &mut term.selection {
            sel.update(Pos::new(line, Column(col)), side);
        }
    }

    pub(crate) fn update_local_selection(&mut self, x: f64, y: f64) {
        if self.local_pointer == LocalPointerState::Selecting && self.mouse_buttons[0] {
            self.update_selection_at(x, y);
        }
    }

    pub(crate) fn finish_local_selection(&mut self, x: f64, y: f64) {
        if self.local_pointer == LocalPointerState::Selecting {
            self.update_selection_at(x, y);
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

/// Number of a top-row function key, `1` for F1 through `24` for F24.
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
        NamedKey::F13 => 13,
        NamedKey::F14 => 14,
        NamedKey::F15 => 15,
        NamedKey::F16 => 16,
        NamedKey::F17 => 17,
        NamedKey::F18 => 18,
        NamedKey::F19 => 19,
        NamedKey::F20 => 20,
        NamedKey::F21 => 21,
        NamedKey::F22 => 22,
        NamedKey::F23 => 23,
        NamedKey::F24 => 24,
        _ => return None,
    })
}

/// Bytes for F1–F24 as `xterm-256color` and `rio` list them (`kf1`–`kf63`).
///
/// Unmodified F1–F4 are SS3 (`\EOP`…`\EOS`). F5–F12 are CSI, skipping 16 and
/// 22 (`\E[15~`, `\E[17~`, …, `\E[24~`). A modifier uses the xterm parameter
/// (2 shift, 3 alt, 4 both, 5 ctrl, and the combinations of those): `\E[1;{mod}P`
/// for F1–F4 and `\E[{code};{mod}~` for F5–F12. xterm sends F13–F24 as shifted
/// F1–F12, so their parameter always includes shift; other modifiers add to it.
/// Midnight Commander reads these from terminfo after it turns the keypad on.
fn function_key_bytes(n: u8, shift: bool, alt: bool, ctrl: bool) -> Option<Vec<u8>> {
    let (n, shift) = if (13..=24).contains(&n) {
        (n - 12, true)
    } else {
        (n, shift)
    };
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

/// CSI sequences for the edit keys, as xterm-256color defines them. With a
/// modifier held, xterm uses `CSI n;m~` with the 1-based modifier parameter.
fn edit_key_bytes(key: NamedKey, modifiers: Modifiers) -> Option<Vec<u8>> {
    let number = match key {
        NamedKey::Insert => 2,
        NamedKey::Delete => 3,
        NamedKey::PageUp => 5,
        NamedKey::PageDown => 6,
        _ => return None,
    };
    let param = modifier_param(modifiers.state());
    Some(if param == 1 {
        format!("\x1b[{number}~").into_bytes()
    } else {
        format!("\x1b[{number};{param}~").into_bytes()
    })
}

/// xterm's 1-based modifier parameter: shift=1, alt=2, ctrl=4, super=8.
fn modifier_param(state: winit::keyboard::ModifiersState) -> u8 {
    1 + u8::from(state.shift_key())
        + 2 * u8::from(state.alt_key())
        + 4 * u8::from(state.control_key())
        + 8 * u8::from(state.meta_key())
}

/// Kitty's private-use code for keys with no legacy escape sequence. Kitty
/// sends these as `CSI {code} u` even when the keyboard protocol is off, so an
/// application that does not speak it still sees the keypress.
fn named_key_code(key: NamedKey) -> Option<u32> {
    Some(match key {
        NamedKey::ScrollLock => 57359,
        NamedKey::NumLock => 57360,
        NamedKey::PrintScreen => 57361,
        NamedKey::Pause => 57362,
        NamedKey::ContextMenu => 57363,
        NamedKey::MediaPlay => 57428,
        NamedKey::MediaPause => 57429,
        NamedKey::MediaPlayPause => 57430,
        NamedKey::MediaStop => 57432,
        NamedKey::MediaTrackNext => 57435,
        NamedKey::MediaTrackPrevious => 57436,
        NamedKey::AudioVolumeDown => 57438,
        NamedKey::AudioVolumeMute => 57440,
        NamedKey::AudioVolumeUp => 57439,
        _ => return None,
    })
}

pub(crate) fn key_to_bytes(
    k: &winit::event::KeyEvent,
    modifiers: Modifiers,
    app_cursor: bool,
    app_keypad: bool,
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
        // The numpad comma has no SS3 form; kitty's KP_SEPARATOR is used even
        // in legacy mode. macOS delivers it as text, others as a named key.
        Key::Character(s)
            if s.as_str() == ","
                && (k.location == KeyLocation::Numpad
                    || k.physical_key == PhysicalKey::Code(KeyCode::NumpadComma)) =>
        {
            Some(b"\x1b[57416u".to_vec())
        }
        Key::Character(s) => {
            if ctrl {
                return None;
            }
            let text = k
                .text
                .as_deref()
                .filter(|text| !text.is_empty())
                .unwrap_or(s.as_str());
            if text.is_empty() {
                return None;
            }
            let mut out = Vec::with_capacity(text.len() + usize::from(alt));
            if alt {
                out.push(0x1b);
            }
            out.extend_from_slice(text.as_bytes());
            Some(out)
        }
        // The numpad Enter is only distinguishable by its location. In
        // application keypad mode xterm sends SS3 M (`kent`); outside it,
        // Enter types CR like the main key.
        Key::Named(NamedKey::Enter) if app_keypad && k.location == KeyLocation::Numpad => {
            let param = modifier_param(modifiers.state());
            Some(if param == 1 {
                b"\x1bOM".to_vec()
            } else {
                format!("\x1bO{param}M").into_bytes()
            })
        }
        Key::Named(NamedKey::Enter) => Some(b"\r".to_vec()),
        // DEL for plain Backspace, as xterm and ghostty send it; readline
        // expects it. Ctrl+Backspace is ^H so shells can bind it separately.
        Key::Named(NamedKey::Backspace) => Some(if ctrl { vec![0x08] } else { vec![0x7f] }),
        Key::Named(NamedKey::Tab) => Some(if modifiers.state().shift_key() {
            b"\x1b[Z".to_vec()
        } else {
            b"\t".to_vec()
        }),
        Key::Named(NamedKey::Escape) => Some(vec![0x1b]),
        // xterm's "Begin" (`kb2`) is the numpad Clear key on many keyboards.
        Key::Named(NamedKey::Clear) => Some(b"\x1bOE".to_vec()),
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
        Key::Named(named) => match named_key_code(*named) {
            Some(code) => Some(format!("\x1b[{code}u").into_bytes()),
            None => edit_key_bytes(*named, modifiers),
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cursor_key_bytes, edit_key_bytes, encode_mouse_report, function_key_bytes,
        function_key_index, key_to_bytes, line_selection, motion_button, named_key_code,
        selection_bounds, word_selection,
    };
    use rio_vt::crosswords::pos::{Column, Line, Pos, Side};
    use rio_vt::selection::{Selection, SelectionType};
    use winit::event::{ElementState, KeyEvent, Modifiers};
    use winit::keyboard::{
        Key, KeyCode, KeyLocation, ModifiersState, NamedKey, NativeKeyCode, PhysicalKey,
    };

    fn key_event(logical: Key, location: KeyLocation) -> KeyEvent {
        KeyEvent {
            physical_key: PhysicalKey::Unidentified(NativeKeyCode::Unidentified),
            logical_key: logical.clone(),
            text: None,
            location,
            state: ElementState::Pressed,
            repeat: false,
            text_with_all_modifiers: None,
            key_without_modifiers: logical.clone(),
        }
    }

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
    fn edit_keys_send_xterm_sequences() {
        let none = Modifiers::from(ModifiersState::empty());
        assert_eq!(
            edit_key_bytes(NamedKey::Insert, none).as_deref(),
            Some(&b"\x1b[2~"[..])
        );
        assert_eq!(
            edit_key_bytes(NamedKey::Delete, none).as_deref(),
            Some(&b"\x1b[3~"[..])
        );
        assert_eq!(
            edit_key_bytes(NamedKey::PageUp, none).as_deref(),
            Some(&b"\x1b[5~"[..])
        );
        assert_eq!(
            edit_key_bytes(NamedKey::PageDown, none).as_deref(),
            Some(&b"\x1b[6~"[..])
        );
        assert_eq!(edit_key_bytes(NamedKey::Enter, none), None);

        let shift = Modifiers::from(ModifiersState::SHIFT);
        assert_eq!(
            edit_key_bytes(NamedKey::Insert, shift).as_deref(),
            Some(&b"\x1b[2;2~"[..])
        );
        let ctrl = Modifiers::from(ModifiersState::CONTROL);
        assert_eq!(
            edit_key_bytes(NamedKey::PageDown, ctrl).as_deref(),
            Some(&b"\x1b[6;5~"[..])
        );
    }

    #[test]
    fn shift_tab_sends_back_tab() {
        let none = Modifiers::from(ModifiersState::empty());
        let shift = Modifiers::from(ModifiersState::SHIFT);
        let tab = key_event(Key::Named(NamedKey::Tab), KeyLocation::Standard);
        assert_eq!(
            key_to_bytes(&tab, none, false, false).as_deref(),
            Some(&b"\t"[..])
        );
        assert_eq!(
            key_to_bytes(&tab, shift, false, false).as_deref(),
            Some(&b"\x1b[Z"[..])
        );
    }

    #[test]
    fn numpad_enter_is_ss3_only_in_application_keypad_mode() {
        let none = Modifiers::from(ModifiersState::empty());
        let ctrl = Modifiers::from(ModifiersState::CONTROL);
        let enter = key_event(Key::Named(NamedKey::Enter), KeyLocation::Numpad);
        assert_eq!(
            key_to_bytes(&enter, none, false, false).as_deref(),
            Some(&b"\r"[..])
        );
        assert_eq!(
            key_to_bytes(&enter, none, false, true).as_deref(),
            Some(&b"\x1bOM"[..])
        );
        assert_eq!(
            key_to_bytes(&enter, ctrl, false, true).as_deref(),
            Some(&b"\x1bO5M"[..])
        );
        let main = key_event(Key::Named(NamedKey::Enter), KeyLocation::Standard);
        assert_eq!(
            key_to_bytes(&main, none, false, true).as_deref(),
            Some(&b"\r"[..])
        );
    }

    #[test]
    fn clear_sends_xterm_begin_and_backspace_stays_del() {
        let none = Modifiers::from(ModifiersState::empty());
        let ctrl = Modifiers::from(ModifiersState::CONTROL);
        let clear = key_event(Key::Named(NamedKey::Clear), KeyLocation::Numpad);
        assert_eq!(
            key_to_bytes(&clear, none, false, false).as_deref(),
            Some(&b"\x1bOE"[..])
        );
        let backspace = key_event(Key::Named(NamedKey::Backspace), KeyLocation::Standard);
        assert_eq!(
            key_to_bytes(&backspace, none, false, false).as_deref(),
            Some(&b"\x7f"[..])
        );
        assert_eq!(
            key_to_bytes(&backspace, ctrl, false, false).as_deref(),
            Some(&b"\x08"[..])
        );
    }

    #[test]
    fn keys_without_a_legacy_form_send_their_kitty_code() {
        let none = Modifiers::from(ModifiersState::empty());
        for (named, code) in [
            (NamedKey::ScrollLock, 57359),
            (NamedKey::NumLock, 57360),
            (NamedKey::PrintScreen, 57361),
            (NamedKey::Pause, 57362),
            (NamedKey::ContextMenu, 57363),
            (NamedKey::MediaPlayPause, 57430),
            (NamedKey::MediaStop, 57432),
            (NamedKey::MediaTrackNext, 57435),
            (NamedKey::MediaTrackPrevious, 57436),
            (NamedKey::AudioVolumeDown, 57438),
            (NamedKey::AudioVolumeMute, 57440),
            (NamedKey::AudioVolumeUp, 57439),
        ] {
            assert_eq!(named_key_code(named), Some(code), "{named:?}");
            let event = key_event(Key::Named(named), KeyLocation::Standard);
            let expected = format!("\x1b[{code}u").into_bytes();
            assert_eq!(
                key_to_bytes(&event, none, false, false).as_deref(),
                Some(expected.as_slice()),
                "{named:?}"
            );
        }
        // Numpad comma is text on macOS and a named key elsewhere.
        let comma = key_event(Key::Character(",".into()), KeyLocation::Numpad);
        assert_eq!(
            key_to_bytes(&comma, none, false, false).as_deref(),
            Some(&b"\x1b[57416u"[..])
        );
        let mut physical_comma = key_event(Key::Character(",".into()), KeyLocation::Standard);
        physical_comma.physical_key = PhysicalKey::Code(KeyCode::NumpadComma);
        assert_eq!(
            key_to_bytes(&physical_comma, none, false, false).as_deref(),
            Some(&b"\x1b[57416u"[..])
        );
    }

    #[test]
    fn selection_range_round_trips_through_rio() {
        use rio_vt::ansi::CursorShape;
        use rio_vt::crosswords::{Crosswords, CrosswordsSize};
        use rio_vt::event::{VoidListener, WindowId};

        let dims = CrosswordsSize::new_with_dimensions(20, 3, 0, 0, 9, 18);
        let mut term = Crosswords::new(
            dims,
            CursorShape::Block,
            VoidListener,
            WindowId::from(0),
            0,
            100,
        );
        for (i, ch) in "hello world".chars().enumerate() {
            term.grid[Pos::new(Line(0), Column(i))].set_c(ch);
        }
        let mut selection = Selection::new(
            SelectionType::Simple,
            Pos::new(Line(0), Column(0)),
            Side::Left,
        );
        selection.update(Pos::new(Line(0), Column(4)), Side::Right);
        term.selection = Some(selection);
        assert_eq!(term.selection_to_string().as_deref(), Some("hello"));
    }

    #[test]
    fn a_url_wrapped_across_rows_is_selected_whole() {
        use rio_vt::ansi::CursorShape;
        use rio_vt::crosswords::{Crosswords, CrosswordsSize};
        use rio_vt::event::{VoidListener, WindowId};

        let dims = CrosswordsSize::new_with_dimensions(20, 3, 0, 0, 9, 18);
        let mut term = Crosswords::new(
            dims,
            CursorShape::Block,
            VoidListener,
            WindowId::from(0),
            0,
            100,
        );
        for (i, ch) in "see http://example.c".chars().enumerate() {
            term.grid[Pos::new(Line(0), Column(i))].set_c(ch);
        }
        term.grid[Pos::new(Line(0), Column(19))].set_wrapline(true);
        for (i, ch) in "om/path?q=1. end".chars().enumerate() {
            term.grid[Pos::new(Line(1), Column(i))].set_c(ch);
        }
        term.selection = word_selection(&term, 1, 2);
        assert_eq!(
            term.selection_to_string().as_deref(),
            Some("http://example.com/path?q=1")
        );
    }

    #[test]
    fn selection_uses_grid_lines_when_scrolled_into_history() {
        use rio_vt::ansi::CursorShape;
        use rio_vt::crosswords::grid::Scroll;
        use rio_vt::crosswords::{Crosswords, CrosswordsSize};
        use rio_vt::event::{VoidListener, WindowId};

        let dims = CrosswordsSize::new_with_dimensions(20, 3, 0, 0, 9, 18);
        let mut term = Crosswords::new(
            dims,
            CursorShape::Block,
            VoidListener,
            WindowId::from(0),
            0,
            100,
        );
        for (i, ch) in "history line".chars().enumerate() {
            term.grid[Pos::new(Line(0), Column(i))].set_c(ch);
        }
        for (i, ch) in "current line".chars().enumerate() {
            term.grid[Pos::new(Line(1), Column(i))].set_c(ch);
        }
        term.scroll_up_relative(Line(0), 1);
        term.scroll_display(Scroll::Delta(1));
        assert_eq!(term.display_offset(), 1);

        term.selection = line_selection(&term, 0);
        assert_eq!(term.selection_to_string().as_deref(), Some("history line"));
    }

    #[test]
    fn triple_click_selects_the_whole_logical_line() {
        use rio_vt::ansi::CursorShape;
        use rio_vt::crosswords::{Crosswords, CrosswordsSize};
        use rio_vt::event::{VoidListener, WindowId};

        let dims = CrosswordsSize::new_with_dimensions(20, 3, 0, 0, 9, 18);
        let mut term = Crosswords::new(
            dims,
            CursorShape::Block,
            VoidListener,
            WindowId::from(0),
            0,
            100,
        );
        for (i, ch) in "see http://example.c".chars().enumerate() {
            term.grid[Pos::new(Line(0), Column(i))].set_c(ch);
        }
        term.grid[Pos::new(Line(0), Column(19))].set_wrapline(true);
        for (i, ch) in "om/path now".chars().enumerate() {
            term.grid[Pos::new(Line(1), Column(i))].set_c(ch);
        }
        term.selection = line_selection(&term, 1);
        assert_eq!(
            term.selection_to_string().as_deref(),
            Some("see http://example.com/path now")
        );
    }

    #[test]
    fn double_click_bounds_words_and_urls() {
        let selected = |text: &str, at: usize| {
            let chars: Vec<char> = text.chars().collect();
            selection_bounds(&chars, at)
                .map(|(start, end)| chars[start..=end].iter().collect::<String>())
        };
        assert_eq!(selected("hello world", 2).as_deref(), Some("hello"));
        assert_eq!(selected("foo.bar", 4).as_deref(), Some("foo.bar"));
        assert_eq!(selected("hi  x", 2), None);

        let url = "see https://example.com/a?b=1. now";
        assert_eq!(
            selected(url, 20).as_deref(),
            Some("https://example.com/a?b=1")
        );
        assert_eq!(
            selected("<www.example.com>", 6).as_deref(),
            Some("www.example.com")
        );
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
        assert!(function_key_bytes(25, false, false, false).is_none());

        // F13-F24 are xterm's shifted F1-F12, with extra modifiers added to
        // the same parameter (kf37 and kf49-equivalent forms).
        assert_eq!(function_key_index(NamedKey::F13), Some(13));
        assert_eq!(function_key_index(NamedKey::F24), Some(24));
        assert_eq!(
            function_key_bytes(13, false, false, false).as_deref(),
            Some(&b"\x1b[1;2P"[..])
        );
        assert_eq!(
            function_key_bytes(17, false, false, false).as_deref(),
            Some(&b"\x1b[15;2~"[..])
        );
        assert_eq!(
            function_key_bytes(13, false, false, true).as_deref(),
            Some(&b"\x1b[1;6P"[..])
        );
        assert_eq!(
            function_key_bytes(24, false, false, false).as_deref(),
            Some(&b"\x1b[24;2~"[..])
        );
    }
}
