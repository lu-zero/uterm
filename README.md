# uterm

A tiny, urxvt-in-spirit terminal emulator using Rio’s released VT/PTY underpinnings.

`uterm` wires [`rio-vt`](https://crates.io/crates/rio-vt) (terminal state, ANSI/VT parsing, grid/scrollback) and `teletypewriter` (PTY) together with a minimal CPU-rendered frontend: [`winit`](https://crates.io/crates/winit) for windowing/input, [`softbuffer`](https://crates.io/crates/softbuffer) for presentation, and [`fontdue`](https://crates.io/crates/fontdue) for glyph rasterization.

## Features

- PTY-backed interactive shell (default login shell via `rio_vt::teletypewriter::create_pty_with_spawn`)
- VT state and parsing driven by `rio_vt::crosswords::Crosswords` + `performer::Machine`
- HiDPI-aware cell sizing and rendering with bundled **Cascadia Code NF**
- Basic keyboard input (text, arrows/home/end/page/delete, Ctrl/Alt combos)
- Minimal tab support (one shell per tab)

### Tab keybindings

- **Ctrl+Shift+T**: new tab
- **Ctrl+Shift+W**: close active tab (exits if last)
- **Ctrl+Tab** / **Ctrl+PageDown**: next tab
- **Ctrl+Shift+Tab** / **Ctrl+PageUp**: previous tab

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

- Renders default foreground/background/cursor palette (no per-cell SGR color/attribute rendering yet)
- No mouse reporting, selection/clipboard, IME, or xterm/kitty keyboard protocol
- No scrollback UI and no rendered graphics protocols
- Simple static tab labels/bar (no OSC titles, no mouse tab controls)

## License

- `uterm` source: [MIT](./LICENSE)
- Cascadia Code NF (bundled): [SIL Open Font License 1.1](./CascadiaCode-LICENSE.txt)
