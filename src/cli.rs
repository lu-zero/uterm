// Command-line/startup configuration parsing.

use crate::error::Error;
use std::ffi::OsStr;
use std::path::PathBuf;
use winnow_args::{help, Args, ErrorKind};

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
    pub(crate) list_fonts: bool,
    /// Words after `--`, run as a command instead of the default shell.
    pub(crate) command: Vec<String>,
}

#[derive(Debug)]
pub(crate) enum Startup {
    /// Help or version text, already rendered.
    Help(String),
    Run(RunOptions),
}

/// A tiny terminal.
///
/// `Ctrl+Shift+T/W`  *new/close tab*
/// `Ctrl+Shift+C/V`  *copy/paste*
/// `Ctrl+Shift+N`    *new window*
/// `Alt+Enter`       *toggle resize lock*
#[derive(Args, Debug)]
#[arg(name = "uterm", verbatim_doc_comment)]
struct Cli {
    /// Terminal size in cells, e.g. `128x40`.
    #[arg(long, value_name = "WxH", parse_with = geometry_value)]
    geometry: Option<StartupGeometry>,
    /// `sh`, `mc`, `ssh`, or a name from config.toml.
    #[arg(long, value_name = "NAME")]
    profile: Option<String>,
    /// Fonts and profiles (default: `~/.config/uterm/config.toml`).
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Print bundled and installed font families.
    #[arg(long)]
    list_fonts: bool,
    /// Run this command instead of the default shell.
    #[arg(positional, double_dash = "required", value_name = "ARG")]
    command: Vec<String>,
}

fn geometry_value(value: &str) -> Result<StartupGeometry, String> {
    parse_geometry(value)
        .map(|(cols, rows)| StartupGeometry { cols, rows })
        .map_err(|err| err.to_string())
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
    if cols == 0 || rows == 0 || cols > u16::MAX as usize || rows > u16::MAX as usize {
        return Err(invalid());
    }
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

/// Parse the process arguments, after the program name.
pub(crate) fn parse_args() -> Result<Startup, Error> {
    parse_args_from(std::env::args_os().skip(1))
}

pub(crate) fn parse_args_from<S: AsRef<OsStr>>(
    args: impl IntoIterator<Item = S>,
) -> Result<Startup, Error> {
    match Cli::parse_from(args) {
        Ok(cli) => Ok(Startup::Run(RunOptions {
            geometry: cli.geometry,
            profile: cli.profile,
            config: cli.config,
            list_fonts: cli.list_fonts,
            command: cli.command,
        })),
        Err(err) => match err.kind() {
            ErrorKind::HelpRequested => Ok(Startup::Help(
                err.render_help_styled("uterm", help::Style::auto())
                    .unwrap_or_default(),
            )),
            ErrorKind::VersionRequested => {
                Ok(Startup::Help(err.version_text("uterm").unwrap_or_default()))
            }
            _ => Err(Error::Usage(err.render(help::Style::auto_stderr()))),
        },
    }
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
        let Startup::Run(options) = parse_args_from(["--profile", "mc"]).unwrap() else {
            panic!("help");
        };
        assert!(options.geometry.is_none());
        assert_eq!(options.profile.as_deref(), Some("mc"));
    }

    #[test]
    fn trailing_words_after_double_dash_are_the_command() {
        let Startup::Run(options) =
            parse_args_from(["--profile", "mc", "--", "mc", "-d", "--flag"]).unwrap()
        else {
            panic!("help");
        };
        assert_eq!(options.profile.as_deref(), Some("mc"));
        assert_eq!(options.command, ["mc", "-d", "--flag"]);
    }

    #[test]
    fn zero_geometry_is_an_error() {
        let err = parse_args_from(["--geometry", "0x1"]).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("0x1") || message.contains("invalid"),
            "{message}"
        );
    }
}
