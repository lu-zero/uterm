# uterm

A tiny, urxvt-in-spirit terminal emulator using Rio’s released VT/PTY underpinnings.

`uterm` wires [`rio-vt`](https://crates.io/crates/rio-vt) (terminal state, ANSI/VT parsing, grid/scrollback) and `teletypewriter` (PTY) together with a minimal CPU-rendered frontend: [`winit`](https://crates.io/crates/winit) for windowing/input, [`softbuffer`](https://crates.io/crates/softbuffer) for presentation, and [`fontdue`](https://crates.io/crates/fontdue) for glyph rasterization.

## Features

- PTY-backed interactive shell (default login shell via `rio_vt::teletypewriter::create_pty_with_spawn`)
- VT state and parsing driven by `rio_vt::crosswords::Crosswords` + `performer::Machine`
- HiDPI-aware cell sizing and rendering with bundled **Cascadia Code NF**
- Per-cell foreground/background color rendering (ANSI 16/256/truecolor, named/indexed palette resolution, inverse)
- Basic keyboard input (text, arrows/home/end/page/delete, Ctrl/Alt combos)
- Minimal tab support (one shell per tab) with a styled tab bar showing per-tab titles (OSC/title updates), clickable tabs, and per-tab close buttons; tabs also close when the shell exits (e.g. `Ctrl+D`) or via shortcut
- Window title follows the active tab title
- Local mouse selection/highlight in the terminal area when mouse reporting is disabled by the application
- Basic copy/paste integration for local selection and clipboard (`Ctrl+Shift+C` to copy selection, `Ctrl+Shift+V` to paste; bracketed paste used when enabled by the shell/app)
- OSC 52 clipboard store/load integration (terminal apps can set/read clipboard via escape sequences, routed per tab)
- Basic mouse reporting to the terminal (SGR/UTF-8 modes) for button, motion/drag, and wheel events when enabled by applications

### Keybindings

- **Ctrl+Shift+T**: new tab
- **Ctrl+Shift+W**: close active tab (exits if last)
- **Ctrl+Tab** / **Ctrl+PageDown**: next tab
- **Ctrl+Shift+Tab** / **Ctrl+PageUp**: previous tab
- **Ctrl+Shift+C**: copy current selection to clipboard
- **Ctrl+Shift+V**: paste clipboard into terminal
- **Alt+Enter**: toggle window-size lock (while locked, `WindowEvent::Resized` no longer resizes the terminal grid)
- **Mouse click on tab**: switch tab
- **Mouse click on tab close (×)**: close tab

## Building and running

Requires a Rust toolchain.

```bash
cargo run
# or
cargo run --release
```

`Cargo.lock` is not committed (library-style noise for this small app); dependency resolution comes from `Cargo.toml`.

On Linux the `clipboard`, `x11` and `wayland` features are enabled by default (forwarding to `rio-vt`), matching Rio’s clipboard backend pattern. You can override with cargo features if needed (e.g. `cargo build --no-default-features --features clipboard,x11`).

## Scope / limitations

Terminal route IDs are `Tab.id` values and are used for tab-exit routing (`UiEvent::TabExited(id)`). This is a small embedded-terminal app, so integer IDs are sufficient.

- Renders cell fg/bg colors (and inverse) but not full text attributes (bold/italic/underline/strikethrough styling, cursor-shape-per-cell details beyond basics)
- Basic local selection copy/paste and OSC 52 clipboard handling; no IME, xterm/kitty keyboard protocol, or clipboard popup on paste (plain `Ctrl+V` into terminal)
- No scrollback UI and no rendered graphics protocols
- Simple tab bar UI (titles and close buttons supported, but no tab reordering, dragging, or advanced widgets)

## License

- `uterm` source: [MIT](./LICENSE)
- Cascadia Code NF (bundled): [SIL Open Font License 1.1](./CascadiaCode-LICENSE.txt)
