// Command-line/startup configuration parsing.

use crate::error::Error;
use crate::terminal::checked_grid;

/// A requested terminal geometry in character cells (columns x rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StartupGeometry {
    pub(crate) cols: usize,
    pub(crate) rows: usize,
}

#[derive(Debug)]
pub(crate) enum Startup {
    Help,
    Run(Option<StartupGeometry>),
}

pub(crate) fn help_text() -> &'static str {
    "uterm: a tiny terminal\n  --geometry WxH   terminal size in cells (e.g. 128x40)\n  --profile NAME   one of: sh, mc, ssh\n  Ctrl+Shift+T/W    new/close tab\n  Ctrl+Shift+C/V    copy/paste\n  Ctrl+Shift+N      new window\n  Alt+Enter         toggle resize lock"
}

const PROFILES: &[(&str, usize, usize)] = &[("sh", 80, 25), ("mc", 80, 24), ("ssh", 128, 40)];

/// Pre-defined window sizes (columns, rows) cycled with Ctrl+Shift+/-.
pub(crate) const PRESET_SIZES: &[(usize, usize)] =
    &[(80, 24), (100, 30), (128, 40), (80, 25), (120, 40)];

pub(crate) fn parse_geometry(s: &str) -> Result<(usize, usize), Error> {
    let invalid = || {
        Error::Usage(format!(
            "invalid geometry `{s}`; expected COLSxROWS with each side in 1..=65535"
        ))
    };
    let Some((cols, rows)) = s.trim().split_once(['x', 'X']) else {
        return Err(invalid());
    };
    let cols: usize = cols.trim().parse().map_err(|_| invalid())?;
    let rows: usize = rows.trim().parse().map_err(|_| invalid())?;
    checked_grid(cols, rows)?;
    Ok((cols, rows))
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
pub(crate) fn parse_args() -> Result<Startup, Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    parse_args_from(&args)
}

pub(crate) fn parse_args_from(args: &[String]) -> Result<Startup, Error> {
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
                let (cols, rows) = parse_geometry(v)?;
                geometry = Some((cols, rows));
            } else {
                profile = Some(v.to_string());
            }
        } else if arg == "--geometry" || arg == "--profile" {
            let Some(v) = args.get(i + 1) else {
                return Err(Error::Usage(format!("missing value for {arg}")));
            };
            if arg == "--geometry" {
                let (cols, rows) = parse_geometry(v)?;
                geometry = Some((cols, rows));
            } else {
                profile = Some(v.clone());
            }
            i += 1;
        } else if arg == "--help" || arg == "-h" {
            return Ok(Startup::Help);
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
    Ok(Startup::Run(
        geometry.map(|(cols, rows)| StartupGeometry { cols, rows }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_parsing() {
        assert_eq!(parse_geometry("128x40").unwrap(), (128, 40));
        assert_eq!(parse_geometry("80X24").unwrap(), (80, 24));
        assert_eq!(parse_geometry(" 100 x 30 ").unwrap(), (100, 30));
        assert!(parse_geometry("garbage").is_err());
        assert!(parse_geometry("12").is_err());
        assert!(parse_geometry("0x1").is_err());
        assert!(parse_geometry("1x0").is_err());
        assert!(parse_geometry("65536x1").is_err());
    }

    #[test]
    fn zero_geometry_is_an_error() {
        let err = parse_args_from(&["--geometry".into(), "0x1".into()]).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("0x1") || message.contains("invalid"),
            "{message}"
        );
    }
}
