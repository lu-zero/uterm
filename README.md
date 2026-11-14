# uterm

A tiny, urxvt-in-spirit terminal emulator using Rio’s released VT/PTY underpinnings.

`uterm` wires [`rio-vt`](https://crates.io/crates/rio-vt) (terminal state, ANSI/VT parsing, grid/scrollback) and `teletypewriter` (PTY) together with a minimal CPU-rendered frontend: [`winit`](https://crates.io/crates/winit) for windowing/input, [`softbuffer`](https://crates.io/crates/softbuffer) for presentation, and [`fontdue`](https://crates.io/crates/fontdue) for glyph rasterization.

## Features

- PTY-backed interactive shell (default login shell via `rio_vt::teletypewriter::create_pty_with_spawn`)
- VT state and parsing driven by `rio_vt::crosswords::Crosswords` + `performer::Machine`
- HiDPI-aware cell sizing and rendering with bundled **Cascadia Code NF**
- Per-cell foreground/background color rendering (ANSI 16/256/truecolor, named/indexed palette resolution, inverse)
- Basic keyboard input (text, arrows/home/end/page/delete, Ctrl/Alt combos)
- Minimal tab support (one shell per tab), clickable tab bar; tabs close when the shell exits (e.g. `Ctrl+D`) or via shortcut
- Local mouse selection/highlight in the terminal area when mouse reporting is disabled by the application
- Basic mouse reporting to the terminal (SGR/UTF-8 modes) for button, motion/drag, and wheel events when enabled by applications

### Tab keybindings

- **Ctrl+Shift+T**: new tab
- **Ctrl+Shift+W**: close active tab (exits if last)
- **Ctrl+Tab** / **Ctrl+PageDown**: next tab
- **Ctrl+Shift+Tab** / **Ctrl+PageUp**: previous tab
- **Mouse click on tab bar**: switch tab

## Building and running

Requires a Rust toolchain.

```bash
cargo run
# or
cargo run --release
```

`Cargo.lock` is not committed (library-style noise for this small app); dependency resolution comes from `Cargo.toml`.

## Scope / limitations

This is intentionally minimal:

- Renders cell fg/bg colors (and inverse) but not full text attributes (bold/italic/underline/strikethrough styling, cursor-shape-per-cell details beyond basics)
- Local text selection/highlight only (no clipboard copy/paste integration), no IME, and no xterm/kitty keyboard protocol
- No scrollback UI and no rendered graphics protocols
- Simple static tab labels/bar (no OSC titles, no mouse tab reordering/close buttons)

## License

- `uterm` source: [MIT](./LICENSE)
- Cascadia Code NF (bundled): [SIL Open Font License 1.1](./CascadiaCode-LICENSE.txt)
