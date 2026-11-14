# For coding agents

`uterm` is a small, standalone terminal emulator built on Rio’s released VT/PTY underpinnings.

- **Core:** [`rio-vt`](https://crates.io/crates/rio-vt) (`Crosswords` terminal state/grid, parser, `performer::Machine`) and `teletypewriter` for PTY.
- **Frontend:** `winit` window/event loop, `softbuffer` CPU presentation, `fontdue` rasterization with bundled Cascadia Code NF.
- **Scope:** intentionally minimal (basic input, HiDPI font/cell sizing, simple tabs, default palette rendering). See `README.md` for features and limitations.

## Working here

- This is a single-crate binary (`src/main.rs`). Build/check with `cargo check`, run with `cargo run`.
- `Cargo.lock` is gitignored. `target/` is gitignored.
- Cascadia Code NF is bundled as `CascadiaCodeNF.ttf` (SIL OFL 1.1, see `CascadiaCode-LICENSE.txt`); keep its license file with the font.
- Prefer small, behavior-preserving changes. Keep the terminal minimal and avoid speculative abstractions.

## Commits

- A commit you assisted carries `Assisted-by: AGENT:MODEL` (for example `Assisted-by: Iceman:iceman-free`).
- Never add `Co-Authored-By` or a `Signed-off-by`: the sign-off is the human's.
- Keep commit messages clear and scoped to the actual change.
