# Startup profiles

`--profile NAME` selects a startup configuration: terminal size, fonts, font
size, and optionally a command to run instead of the login shell. A profile
applies to the window that starts with it; new tabs reuse the same settings,
and `Ctrl+Shift+N` spawns a new process with the original arguments, so it
picks the same profile.

## Built-in profiles

| name | size | command |
| --- | --- | --- |
| `sh` | 80×25 | none (login shell) |
| `mc` | 80×24 | none (login shell) |
| `ssh` | 128×40 | none (login shell) |

The built-in profiles only define a size. They deliberately do **not** set a
command or arguments, so they all start the user's login shell: on macOS
through `/usr/bin/login -flp <user> <shell>`, and on other platforms by
spawning the shell directly.

## Defining a profile

Profiles live in `config.toml` under `[profiles.NAME]`. The config file is
found in this order, first hit wins:

1. `--config PATH`
2. `$UTERM_CONFIG`
3. `$XDG_CONFIG_HOME/uterm/config.toml`
4. `~/.config/uterm/config.toml`

A missing default file is fine; a missing file passed explicitly is an error.
Unknown keys are rejected.

```toml
[profiles.mc]
geometry = "80x24"
fonts = ["default", "Menlo", "/path/to/cjk.ttf"]
font_size = 13
command = "mc"
args = ["-d"]
```

| field | meaning | default |
| --- | --- | --- |
| `geometry` | `"COLSxROWS"`, each side 1..=65535 | built-in size for `sh`/`mc`/`ssh`, otherwise none |
| `fonts` | primary face (first entry) sets the cell size | top-level `fonts`, or `["default"]` |
| `font_size` | points, 4..=256 | top-level `font_size`, or 14 |
| `command` | program to run instead of the login shell | none (login shell) |
| `args` | arguments for `command` | `[]` |

`default` is the keyword for the face compiled into uterm, so configs survive
a font change. A bare `fonts` entry is an installed family name; a path needs
a separator, a leading `.` or `~`, or a `.ttf`/`.otf`/`.ttc` extension. Glyphs the primary face lacks come from term-atlas's embedded CJK fallback
tiers and installed system fonts.

A custom profile may pick a name that has no built-in size; it then uses the
default window size unless it sets `geometry`. An unknown name is an error.

## Precedence

- **Size:** `--geometry WxH` → profile `geometry` → built-in size → default window size.
- **Command:** words after `--` on the command line → profile `command`/`args` → login shell. This is the initial window's first tab only; new tabs and windows start a shell.
- **Fonts:** profile `fonts` → top-level `fonts` → `["default"]`. Omitting `fonts` in a profile inherits the top-level list; `fonts = []` keeps only the bundled face.
- **Font size:** profile `font_size` → top-level `font_size` → 14.

Examples:

```bash
# Use the mc profile's own command and arguments.
uterm --profile mc

# Override them from the command line.
uterm --profile mc -- mc -d

# Override just the size.
uterm --profile mc --geometry 100x30
```

`args` without a `command`, an empty `command`, and out-of-range `font_size`
values are configuration errors.
