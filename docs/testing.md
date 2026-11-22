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
