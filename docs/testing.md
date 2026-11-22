# Measurements that need Linux

Two open items cannot be verified from a macOS host. These recipes make the
checks reproducible.

## CPU with a 10 Hz status line (#7)

Run a shell command that rewrites one line ten times a second inside uterm:

```bash
cargo run --release -- --geometry 128x40 -- \
  sh -c 'while :; do printf "status %s\r" "$(date +%s.%N)"; sleep 0.1; done'
```

In another terminal, sample the uterm process for ~20 seconds:

```bash
pidstat -p "$(pgrep -x uterm)" 1
```

Record the median user+system CPU percentage and compare it with the blog's
reference numbers (urxvt ~0.7%, GTK terminals 10–16%, the old framebuffer
build ~7%). Only optimize what shows up in the sample — the old 7% figure is
not a baseline for the current tree.

A first local run on macOS (release build, sampled via deltas of `ps -o time=`)
gave a median of 16.7% CPU. Building a cluster `String` for every cell every
frame accounted for part of that: avoiding the allocation for cells without
zero-width extras brought one median to 12.9%. Reusing the per-frame row/cell/
rect buffers and an optional mimalloc allocator were then measured back to
back: 17.6% (default) vs 16.8%/17.8% (mimalloc). The spread is larger than the
difference, so neither is a proven win on this noisy desktop; the dominant
cost is likely the full-frame softbuffer repaint. These are macOS numbers; the
Linux baseline is still needed.

## Wayland exit paths (#9)

On a real Wayland session, run uterm and exercise all three exits:

1. type `exit` in the last shell,
2. click the window's close button,
3. open a second tab, close it with Ctrl+Shift+W, then close the last one.

For each path note whether the process exits cleanly or aborts (for example
`Segmentation fault`, or a `wl_display` assertion). The relevant code is
`SystemClipboard` in `src/app/mod.rs`, which must drop the clipboard before its
owned display handle. Close #9 only when all three paths are clean.

### Setup used for a first attempt

- macOS: `cocoa-way` (from `brew tap J-x-Z/tap && brew install cocoa-way
  waypipe-darwin`). Start `cocoa-way`; it creates a `wayland-1` socket under an
  `XDG_RUNTIME_DIR` such as `/var/folders/.../T/cocoa-way`.
- Linux host: install `waypipe[lz4]` and `dev-libs/wayland` — winit needs
  `libwayland-client.so` at runtime — then build uterm there with
  `cargo build --release`.
- Forward with an explicit display path, since the remote SSH session has no
  `XDG_RUNTIME_DIR`, and pass an executable rather than a shell command:

  ```bash
  export XDG_RUNTIME_DIR=/path/to/cocoa-way
  export WAYLAND_DISPLAY=wayland-1
  waypipe --display /tmp/uterm-wayland-0 ssh -o StreamLocalBindUnlink=yes \
    thalia /path/to/uterm -- /bin/sh
  ```

**Result so far:** the "last shell exits" path was clean (uterm exit status 0,
empty log) with the app's shell replaced by `/bin/true`. The close button and
Ctrl+Shift+W could not be injected from macOS — System Events synthetic events
did not reach the Wayland client — so those two paths still need a real
Wayland session.

## Shared-crate allocation notes

Review of term-atlas 0.1.4 / term-font 0.1.1 by the rune-term side:

- `Atlas::ensure`/`ensure_str` are allocation-free on a cache hit (ASCII array
  lookup / cluster-map hash and compare). Misses allocate scratch buffers,
  a rustybuzz buffer and one `Box<str>` per distinct cluster, and misses are
  capped at 4096; past that, novel clusters re-rasterize each frame.
- `Face::load` caches by family name and the fallback data is `OnceLock`-cached;
  there is no repeated fontique scan from a configured face.
- `term_font::discover` has no cache of its own and builds a fresh fontique
  `Collection`/`SourceCache` per call, but atlas miss caching keeps it off the
  per-frame path in uterm.
- Suggested shared-crate improvements: reuse the cluster scratch `Vec`s and
  `regular_coords` Vec as `Atlas` fields, and add a `(family, char)` discovery
  cache in term-font. A four-pane headless rune-term benchmark likewise found
  no frame-time benefit from allocator tuning: repaint/upload dominates.

### Validation of term-atlas 0.1.5 / term-font 0.1.2

All three shared-crate fixes are in 0.1.5/0.1.2. uterm validation with a
finishing 10 Hz status-line workload (50 updates, 10 paired alternating rounds,
`scripts/ab.py`):

| metric | 0.1.4 | 0.1.5 | ratio (95% CI) |
| --- | --- | --- | --- |
| wall | 5.761 s | 5.759 s | 1.0008 [0.9943, 1.0069] — unresolved |
| CPU | 0.9452 s | 0.9453 s | 0.9969 [0.9898, 1.0211] — unresolved |

Behavior oracle: screenshots of both builds rendering the same text, cropped
to the terminal content, were pixel-identical (ImageMagick `AE = 0`). Tests
pass with default and `lean` features. The win in the shared crates is fewer
allocations in miss paths, not uterm frame time — as predicted by the A/B.

## Where the frame time goes

A 5-second `sample` of a live 10 Hz session put 209 of 210 samples inside
`AppState::render` in `softbuffer::Buffer::present` → CoreAnimation commit →
CoreGraphics/vImage colour conversion. A temporary `UTERM_SKIP_PRESENT` hook
(since removed) confirmed the mechanism with paired A/B on the same binary:

| host | CPU ratio (skip/present) | 95% CI | rounds |
| --- | --- | --- | --- |
| macOS | 0.596 | 0.531–0.712 | 8 |
| Linux (thalia, waypipe) | 0.569 | 0.463–0.605 | 6 |

So about 40–43% of the CPU is the framebuffer presentation path, not glyph
rasterization, blitting, or terminal state work. Wall time is unchanged because
the status line sleeps between updates. RSS numbers from the Linux run are not
meaningful: they include the waypipe/ssh process tree.
