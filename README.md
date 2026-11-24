# uterm

A tiny, urxvt-in-spirit terminal emulator using Rio’s released VT/PTY underpinnings.

`uterm` wires [`rio-vt`](https://crates.io/crates/rio-vt) (terminal state, ANSI/VT parsing, grid/scrollback) and `teletypewriter` (PTY) together with a minimal CPU-rendered frontend: [`winit`](https://crates.io/crates/winit) for windowing/input, [`softbuffer`](https://crates.io/crates/softbuffer) for presentation, and [`fontdue`](https://crates.io/crates/fontdue) for glyph rasterization.

## Features

- PTY-backed interactive shell (default login shell via `rio_vt::teletypewriter::create_pty_with_spawn`)
- VT state and parsing driven by `rio_vt::crosswords::Crosswords` + `performer::Machine`
- HiDPI-aware cell sizing and rendering with bundled **Maple Mono NF**, with automatic system-font fallback for glyphs it lacks, or a configurable face list (`config.toml`)
- Per-cell foreground/background color rendering (ANSI 16/256/truecolor, named/indexed palette resolution, inverse)
- Basic keyboard input (text, arrows/home/end/page/delete, F1–F12, Ctrl/Alt combos)
- Minimal tab support (one shell per tab) with a styled tab bar showing per-tab titles (OSC/title updates), clickable tabs, and per-tab close buttons; tabs also close when the shell exits (e.g. `Ctrl+D`) or via shortcut
- Window title follows the active tab title
- Local mouse selection/highlight in the terminal area when mouse reporting is disabled by the application
- Basic copy/paste integration for local selection and clipboard (`Ctrl+Shift+C` to copy selection, `Ctrl+Shift+V` to paste; bracketed paste used when enabled by the shell/app)
- OSC 52 clipboard store/load integration (terminal apps can set/read clipboard via escape sequences, routed per tab)
- Basic mouse reporting to the terminal (SGR/UTF-8 modes) for button, motion/drag, and wheel events when enabled by applications
- Custom launch size via `--geometry WxH` (cells) or `--profile NAME` (`sh`, `mc`, `ssh`, or a profile in `config.toml`); an explicit geometry wins over a profile
- Words after `--` run a command instead of the login shell in the initial window's first tab; new tabs and windows start a shell
- New window via `Ctrl+Shift+N` (spawns an independent uterm process with the same CLI options but without the startup command), preset sizes via `Ctrl+Shift+=`/`-`, and a transient `cols x rows` overlay during resizing

### Keybindings

- **Ctrl+Shift+T**: new tab
- **Ctrl+Shift+W**: close active tab (exits if last)
- **Ctrl+Tab** / **Ctrl+PageDown**: next tab
- **Ctrl+Shift+Tab** / **Ctrl+PageUp**: previous tab
- **Cmd+C / Cmd+V / Cmd+N** on macOS, **Ctrl+Shift+C / Ctrl+Shift+V / Ctrl+Shift+N** elsewhere: copy selection, paste clipboard, spawn a new window (independent process/PTY, same size)
- **Ctrl+Shift+=** / **Ctrl+Shift+-**: cycle through pre-defined window sizes (clears the resize lock)
- **Alt+Enter**: toggle window-size lock (the window and the terminal grid are both locked)
- **Ctrl+Shift+H**: show/hide the keyboard-shortcut overlay
- **Mouse click on tab**: switch tab
- **Mouse click on ×**: close tab

## Building and running

Requires a Rust toolchain. This build uses `winit 0.31.0-beta.4`: 0.30 has no Wayland drag-and-drop support, so the beta is needed until 0.31 is stable.

```bash
cargo run
# or
cargo run --release
# with a specific size / profile:
cargo run -- --geometry 128x40
cargo run -- --profile mc
# run a command instead of the login shell:
cargo run -- --profile mc -- mc
```

### Build variants

The default build enables both window backends, the bundled font, all CJK
fallback tiers (common, Japanese and full Chinese) and the emoji fallback. On
a system with no CJK fonts installed, those embedded tiers are what keep kanji
and Hanzi from being blank. A `lean` build drops only the emoji data crate,
keeping clipboard/OSC 52, `--list-fonts`, full CJK coverage, profiles, tabs
and both backends:

```bash
cargo build --release --no-default-features --features lean
# optionally use mimalloc instead of the system allocator:
cargo build --release --features mimalloc
```

The core (PTY, config, clipboard, font enumeration, CJK fallback) is not
optional. `--no-default-features` alone does not select a window backend, so
use `lean` or name a backend (`x11`, `wayland`) explicitly. On macOS the lean
binary is about 760 KiB smaller than the default; build time is effectively
unchanged there because the backends are platform-gated.

### Fonts

With no config file, uterm uses the bundled Maple Mono NF. When a glyph is missing, uterm looks it up in installed system fonts (preferring the primary family, then monospace, system UI, and script-specific fallbacks) and caches the face it finds. To control the faces yourself, create `~/.config/uterm/config.toml` (or `$XDG_CONFIG_HOME/uterm/config.toml`, or pass `--config PATH`, or set `$UTERM_CONFIG`).

The first `fonts` entry selects the primary face and sets the cell size. Entries can be the bundled keyword `default`, an installed family name, or a path:

```toml
fonts = ["default", "Menlo", "/path/to/cjk.ttf"]
font_size = 14
```

`default` always means the face compiled into uterm, so configs survive a font change. Glyphs the primary face lacks come from term-atlas's embedded CJK fallback tiers and installed system fonts. A bare name is a family; a path needs a separator, a leading `.` or `~`, or a `.ttf`/`.otf`/`.ttc` extension. Paths may be absolute, relative to the config file, or `~/...`. An empty or missing `fonts` list is the same as `fonts = ["default"]`. `font_size` is in points and defaults to 14. Run `uterm --list-fonts` to see the bundled keyword and the installed font families. See `config.toml` for a fuller example.

### Profiles

`--profile NAME` picks a startup size. Built in: `ssh` (128×40), `sh` (80×25), and `mc` (80×24); the built-ins only set the size and start the login shell. Custom profiles in `config.toml` can also set fonts, font size, and a command with arguments.

See [docs/profiles.md](./docs/profiles.md) for the full reference, precedence rules, and examples.

`Cargo.lock` is not committed (library-style noise for this small app); dependency resolution comes from `Cargo.toml`.

On Linux the `clipboard`, `x11` and `wayland` features are enabled by default (forwarding to `rio-vt`), matching Rio’s clipboard backend pattern. You can override with cargo features if needed (e.g. `cargo build --no-default-features --features clipboard,x11`).

## Scope / limitations

Terminal route IDs are `Tab.id` values and are used for tab-exit routing (`UiEvent::TabExited(id)`). This is a small embedded-terminal app, so integer IDs are sufficient.

- Renders cell fg/bg colors, inverse, dim, bold, italic (sheared), underline (single/double/curly/dotted/dashed), strikethrough and hidden; blink is intentionally not animated, and cursor shapes beyond the basics are not styled per cell
- Wide characters occupy two cells in the grid and are drawn as one cell sprite covering both cells
- Basic local selection copy/paste and OSC 52 clipboard handling; no IME, xterm/kitty keyboard protocol, or clipboard popup on paste (plain `Ctrl+V` into terminal)
- No scrollback UI and no rendered graphics protocols
- Simple tab bar UI (titles and close buttons supported, but no tab reordering, dragging, or advanced widgets)

## License

- `uterm` source: [MIT](./LICENSE)
- Bundled fonts (via `term-atlas`): SIL Open Font License 1.1, in the `term-font-data-*` crates
