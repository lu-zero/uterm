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
zero-width extras brought the median to 12.9%. These are macOS numbers; the
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
