// Command-line/startup configuration parsing.

/// A requested terminal geometry in character cells (columns x rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StartupGeometry {
    pub(crate) cols: usize,
    pub(crate) rows: usize,
}

const PROFILES: &[(&str, usize, usize)] = &[("sh", 80, 25), ("mc", 80, 24), ("ssh", 128, 40)];

/// Pre-defined window sizes (columns, rows) cycled with Ctrl+Shift+/-.
pub(crate) const PRESET_SIZES: &[(usize, usize)] =
    &[(80, 24), (100, 30), (128, 40), (80, 25), (120, 40)];

pub(crate) fn parse_geometry(s: &str) -> Option<(usize, usize)> {
    let (cols, rows) = s.trim().split_once(['x', 'X'])?;
    Some((
        cols.trim().parse::<usize>().ok()?,
        rows.trim().parse::<usize>().ok()?,
    ))
}

/// Command-style modifier for app shortcuts: Command on macOS, Ctrl+Shift elsewhere.
pub(crate) fn is_command_modifier(mods: winit::keyboard::ModifiersState) -> bool {
    #[cfg(target_os = "macos")]
    {
        mods.super_key()
    }
    #[cfg(not(target_os = "macos"))]
    {
        mods.control_key() && mods.shift_key()
    }
}

/// Parse `--geometry WxH` and `--profile NAME` from process arguments.
/// An explicit `--geometry` wins over a profile.
pub(crate) fn parse_args() -> Option<StartupGeometry> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut geometry = None;
    let mut profile = None;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let value = arg
            .strip_prefix("--geometry=")
            .or_else(|| arg.strip_prefix("--profile="));
        if let Some(v) = value {
            if arg.starts_with("--geometry") {
                geometry = parse_geometry(v);
            } else {
                profile = Some(v.to_string());
            }
        } else if arg == "--geometry" || arg == "--profile" {
            if let Some(v) = args.get(i + 1) {
                if arg == "--geometry" {
                    geometry = parse_geometry(v);
                } else {
                    profile = Some(v.clone());
                }
                i += 1;
            }
        } else if arg == "--help" || arg == "-h" {
            println!("uterm: a tiny terminal\n  --geometry WxH   terminal size in cells (e.g. 128x40)\n  --profile NAME   one of: sh, mc, ssh\n  Ctrl+Shift+T/W    new/close tab\n  Ctrl+Shift+C/V    copy/paste\n  Ctrl+Shift+N      new window\n  Alt+Enter         toggle resize lock");
            std::process::exit(0);
        }
        i += 1;
    }
    if geometry.is_none() {
        if let Some(p) = profile.as_deref() {
            geometry = PROFILES
                .iter()
                .find(|(name, _, _)| *name == p)
                .map(|(_, c, r)| (*c, *r));
        }
    }
    geometry.map(|(cols, rows)| StartupGeometry { cols, rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_parsing() {
        assert_eq!(parse_geometry("128x40"), Some((128, 40)));
        assert_eq!(parse_geometry("80X24"), Some((80, 24)));
        assert_eq!(parse_geometry(" 100 x 30 "), Some((100, 30)));
        assert_eq!(parse_geometry("garbage"), None);
        assert_eq!(parse_geometry("12"), None);
    }
}
