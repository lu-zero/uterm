// Command-line/startup configuration parsing.

use crate::error::Error;
use crate::terminal::checked_grid;
use std::path::PathBuf;

/// A requested terminal geometry in character cells (columns x rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StartupGeometry {
    pub(crate) cols: usize,
    pub(crate) rows: usize,
}

#[derive(Debug)]
pub(crate) struct RunOptions {
    /// Set only by `--geometry`. A profile size is applied later, from the
    /// built-in table or from `config.toml`.
    pub(crate) geometry: Option<StartupGeometry>,
    pub(crate) profile: Option<String>,
    pub(crate) config: Option<PathBuf>,
}

#[derive(Debug)]
pub(crate) enum Startup {
    Help,
    Run(RunOptions),
}

pub(crate) fn help_text() -> &'static str {
    "uterm: a tiny terminal\n  --geometry WxH   terminal size in cells (e.g. 128x40)\n  --profile NAME   sh, mc, ssh, or a name from config.toml\n  --config PATH    fonts and profiles (default: ~/.config/uterm/config.toml)\n  Ctrl+Shift+T/W    new/close tab\n  Ctrl+Shift+C/V    copy/paste\n  Ctrl+Shift+N      new window\n  Alt+Enter         toggle resize lock"
}

const PROFILES: &[(&str, usize, usize)] = &[("sh", 80, 25), ("mc", 80, 24), ("ssh", 128, 40)];

/// Built-in size for `sh`, `mc`, and `ssh`. A `config.toml` profile of the
/// same name replaces this when it sets `geometry`.
pub(crate) fn builtin_geometry(name: &str) -> Option<(usize, usize)> {
    PROFILES
        .iter()
        .find(|(profile, _, _)| *profile == name)
        .map(|(_, cols, rows)| (*cols, *rows))
}

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
/// An explicit `--geometry` is returned as geometry. The profile name is
/// returned as given; built-in and `config.toml` sizes are applied in `config`.
pub(crate) fn parse_args() -> Result<Startup, Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    parse_args_from(&args)
}

pub(crate) fn parse_args_from(args: &[String]) -> Result<Startup, Error> {
    let mut geometry = None;
    let mut profile = None;
    let mut config = None;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let value = arg
            .strip_prefix("--geometry=")
            .or_else(|| arg.strip_prefix("--profile="))
            .or_else(|| arg.strip_prefix("--config="));
        if let Some(v) = value {
            if arg.starts_with("--geometry") {
                let (cols, rows) = parse_geometry(v)?;
                geometry = Some((cols, rows));
            } else if arg.starts_with("--config") {
                config = Some(PathBuf::from(v));
            } else {
                profile = Some(v.to_string());
            }
        } else if arg == "--geometry" || arg == "--profile" || arg == "--config" {
            let Some(v) = args.get(i + 1) else {
                return Err(Error::Usage(format!("missing value for {arg}")));
            };
            if arg == "--geometry" {
                let (cols, rows) = parse_geometry(v)?;
                geometry = Some((cols, rows));
            } else if arg == "--config" {
                config = Some(PathBuf::from(v));
            } else {
                profile = Some(v.clone());
            }
            i += 1;
        } else if arg == "--help" || arg == "-h" {
            return Ok(Startup::Help);
        }
        i += 1;
    }
    Ok(Startup::Run(RunOptions {
        geometry: geometry.map(|(cols, rows)| StartupGeometry { cols, rows }),
        profile,
        config,
    }))
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
    fn profile_name_is_left_for_the_config() {
        let Startup::Run(options) = parse_args_from(&["--profile".into(), "mc".into()]).unwrap()
        else {
            panic!("help");
        };
        assert!(options.geometry.is_none());
        assert_eq!(options.profile.as_deref(), Some("mc"));
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
