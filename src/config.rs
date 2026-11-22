//! Font list and named profiles from `config.toml`.
//!
//! Search order, first hit wins:
//! 1. `--config PATH`
//! 2. `$UTERM_CONFIG`
//! 3. `$XDG_CONFIG_HOME/uterm/config.toml`
//! 4. `~/.config/uterm/config.toml`
//!
//! A missing default file keeps the bundled Maple Mono NF. A path given with
//! `--config` or `$UTERM_CONFIG` must exist. The first `fonts` entry selects
//! the primary face, which sets the cell size; entries are the bundled keyword
//! `default`, an installed family name, or a font path. Missing glyphs are
//! supplied by term-atlas's embedded CJK fallback tiers and system fonts.
//! `font_size` sets the point size, default 14.
//!
//! `[profiles.NAME]` may set `geometry` (`"128x40"`) and its own `fonts`.
//! `--profile sh|mc|ssh` still has a built-in size when the file does not
//! replace it. `--geometry` wins over either size. An unknown name is an error.

use crate::cli::{self, StartupGeometry};
use crate::error::Error;
use crate::fonts;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    #[serde(default)]
    fonts: Vec<String>,
    #[serde(default)]
    font_size: Option<f32>,
    #[serde(default)]
    profiles: BTreeMap<String, ProfileFile>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ProfileFile {
    #[serde(default)]
    geometry: Option<String>,
    /// `None` inherits the top-level list. `Some`, including empty, is this
    /// profile's own list (empty keeps the bundled face).
    #[serde(default)]
    fonts: Option<Vec<String>>,
    #[serde(default)]
    font_size: Option<f32>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Option<Vec<String>>,
}

/// One entry in a font list: a bundled keyword, a file path, or an installed
/// family name.
#[derive(Clone, Debug, PartialEq, Eq)]
enum FontSource {
    Bundled(String),
    Family(String),
    File(PathBuf),
}

#[derive(Default)]
struct ParsedConfig {
    fonts: Vec<FontSource>,
    font_size: Option<f32>,
    profiles: BTreeMap<String, ParsedProfile>,
}

struct ParsedProfile {
    geometry: Option<StartupGeometry>,
    fonts: Option<Vec<FontSource>>,
    font_size: Option<f32>,
    /// Program and arguments, or empty for the default login shell.
    command: Vec<String>,
}

type Resolved = (Vec<FontSource>, Option<StartupGeometry>, f32, Vec<String>);

#[derive(Debug)]
pub(crate) struct Session {
    pub(crate) face: term_atlas::Face,
    pub(crate) geometry: Option<StartupGeometry>,
    pub(crate) font_size: f32,
    /// Program and arguments, or empty for the default login shell.
    pub(crate) command: Vec<String>,
}

pub(crate) fn load(
    explicit: Option<&Path>,
    profile: Option<&str>,
    explicit_geometry: Option<StartupGeometry>,
    explicit_command: Vec<String>,
) -> Result<Session, Error> {
    let parsed = read_config(explicit)?;
    let (sources, geometry, font_size, command) =
        resolve(&parsed, profile, explicit_geometry, explicit_command)?;
    let face = primary_face(&sources)?;
    Ok(Session {
        face,
        geometry,
        font_size,
        command,
    })
}

fn primary_face(sources: &[FontSource]) -> Result<term_atlas::Face, Error> {
    match sources.first() {
        None | Some(FontSource::Bundled(_)) => Ok(fonts::bundled()),
        Some(FontSource::Family(name)) => fonts::family(name),
        Some(FontSource::File(path)) => fonts::file(path),
    }
}

fn read_config(explicit: Option<&Path>) -> Result<ParsedConfig, Error> {
    let env_config = std::env::var_os("UTERM_CONFIG")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let required = explicit.is_some() || env_config.is_some();
    let Some(path) = resolve_config_path(explicit, env_config.as_deref()) else {
        return Ok(ParsedConfig::default());
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_config(&text, path.parent().unwrap_or(Path::new("."))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound && !required => {
            Ok(ParsedConfig::default())
        }
        Err(err) => Err(Error::Config(format!(
            "failed to read config {}: {err}",
            path.display()
        ))),
    }
}

fn resolve_config_path(explicit: Option<&Path>, env_config: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path.to_path_buf());
    }
    if let Some(path) = env_config {
        return Some(path.to_path_buf());
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(xdg).join("uterm").join("config.toml"));
    }
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("uterm")
            .join("config.toml"),
    )
}

fn parse_config(text: &str, config_dir: &Path) -> Result<ParsedConfig, Error> {
    let file: ConfigFile =
        toml::from_str(text).map_err(|err| Error::Config(format!("invalid config: {err}")))?;
    let fonts = font_list(&file.fonts, config_dir)?;
    let font_size = validate_font_size(file.font_size)?;
    let mut profiles = BTreeMap::new();
    for (name, profile) in file.profiles {
        if name.is_empty() {
            return Err(Error::Config("config profile name is empty".to_string()));
        }
        let geometry = match profile.geometry.as_deref() {
            Some(raw) => Some(geometry_from(raw)?),
            None => None,
        };
        let fonts = match profile.fonts {
            Some(list) => Some(font_list(&list, config_dir)?),
            None => None,
        };
        let font_size = validate_font_size(profile.font_size)?;
        let command = profile_command(&name, profile.command, profile.args)?;
        profiles.insert(
            name,
            ParsedProfile {
                geometry,
                fonts,
                font_size,
                command,
            },
        );
    }
    Ok(ParsedConfig {
        fonts,
        font_size,
        profiles,
    })
}

/// A profile's `command` plus `args`; no command means the login shell.
fn profile_command(
    name: &str,
    command: Option<String>,
    args: Option<Vec<String>>,
) -> Result<Vec<String>, Error> {
    match (command, args) {
        (None, None) => Ok(Vec::new()),
        (None, Some(_)) => Err(Error::Config(format!(
            "profile `{name}` sets args without a command"
        ))),
        (Some(program), args) => {
            if program.trim().is_empty() {
                return Err(Error::Config(format!(
                    "profile `{name}` has an empty command"
                )));
            }
            let mut argv = vec![program];
            argv.extend(args.unwrap_or_default());
            Ok(argv)
        }
    }
}

fn validate_font_size(size: Option<f32>) -> Result<Option<f32>, Error> {
    match size {
        Some(size) if size.is_finite() && (4.0..=256.0).contains(&size) => Ok(Some(size)),
        Some(_) => Err(Error::Config(
            "font_size must be between 4 and 256".to_string(),
        )),
        None => Ok(None),
    }
}

fn font_list(raw_paths: &[String], config_dir: &Path) -> Result<Vec<FontSource>, Error> {
    let mut sources = Vec::with_capacity(raw_paths.len());
    for raw in raw_paths {
        if raw.is_empty() {
            return Err(Error::Config("config font entry is empty".to_string()));
        }
        if raw.starts_with('~') && raw != "~" && !raw.starts_with("~/") {
            return Err(Error::Config(format!(
                "unsupported home path `{raw}`; use ~ or ~/... "
            )));
        }
        if raw == fonts::DEFAULT_FACE {
            sources.push(FontSource::Bundled(raw.clone()));
        } else if looks_like_path(raw) {
            sources.push(FontSource::File(expand_font_path(raw, config_dir)?));
        } else {
            sources.push(FontSource::Family(raw.clone()));
        }
    }
    Ok(sources)
}

/// Paths are unambiguous about being paths: a separator, a leading `.` or `~`,
/// or a font-file extension. Every other bare name is a font family.
fn looks_like_path(raw: &str) -> bool {
    let lower = raw.to_lowercase();
    raw.starts_with('.')
        || raw.starts_with('~')
        || raw.contains('/')
        || raw.contains(std::path::MAIN_SEPARATOR)
        || [".ttf", ".otf", ".ttc"]
            .iter()
            .any(|ext| lower.ends_with(ext))
}

fn geometry_from(raw: &str) -> Result<StartupGeometry, Error> {
    let (cols, rows) = cli::parse_geometry(raw)?;
    Ok(StartupGeometry { cols, rows })
}

/// `--geometry` wins. A profile's `geometry` wins over the built-in size of
/// the same name. A profile `fonts` list, when present, replaces the top-level
/// list.
fn resolve(
    parsed: &ParsedConfig,
    profile: Option<&str>,
    explicit_geometry: Option<StartupGeometry>,
    explicit_command: Vec<String>,
) -> Result<Resolved, Error> {
    let Some(name) = profile else {
        return Ok((
            parsed.fonts.clone(),
            explicit_geometry,
            parsed.font_size.unwrap_or(DEFAULT_FONT_SIZE),
            explicit_command,
        ));
    };
    let configured = parsed.profiles.get(name);
    let builtin = cli::builtin_geometry(name);
    if configured.is_none() && builtin.is_none() {
        return Err(Error::Usage(format!("unknown profile `{name}`")));
    }
    let geometry = explicit_geometry.or_else(|| {
        configured
            .and_then(|profile| profile.geometry)
            .or_else(|| builtin.map(|(cols, rows)| StartupGeometry { cols, rows }))
    });
    let fonts = match configured.and_then(|profile| profile.fonts.as_ref()) {
        Some(fonts) => fonts.clone(),
        None => parsed.fonts.clone(),
    };
    let font_size = configured
        .and_then(|profile| profile.font_size)
        .or(parsed.font_size)
        .unwrap_or(DEFAULT_FONT_SIZE);
    let command = if explicit_command.is_empty() {
        configured
            .map(|profile| profile.command.clone())
            .unwrap_or_default()
    } else {
        explicit_command
    };
    Ok((fonts, geometry, font_size, command))
}

pub(crate) const DEFAULT_FONT_SIZE: f32 = 14.0;

fn expand_font_path(raw: &str, config_dir: &Path) -> Result<PathBuf, Error> {
    let path = if raw == "~" {
        home_dir().ok_or_else(|| Error::Config("cannot expand `~`: HOME is not set".to_string()))?
    } else if let Some(rest) = raw.strip_prefix("~/") {
        let home = home_dir()
            .ok_or_else(|| Error::Config("cannot expand `~/`: HOME is not set".to_string()))?;
        home.join(rest)
    } else {
        PathBuf::from(raw)
    };
    Ok(if path.is_absolute() {
        path
    } else {
        config_dir.join(path)
    })
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_font_list_and_ignores_comments() {
        let text = "\n# primary\nfonts = [\n  \"Cascadia.ttf\",\n  \"~/Fonts/unifont.ttf\", # plane 0\n]\n";
        let fonts = parse_config(text, Path::new("/cfg")).unwrap().fonts;
        assert_eq!(
            fonts[0],
            FontSource::File(PathBuf::from("/cfg/Cascadia.ttf"))
        );
        let expected_home = std::env::var("HOME").unwrap();
        assert_eq!(
            fonts[1],
            FontSource::File(PathBuf::from(expected_home).join("Fonts/unifont.ttf"))
        );
    }

    #[test]
    fn rejects_unknown_settings_and_empty_paths() {
        assert!(parse_config("size = 14\n", Path::new(".")).is_err());
        assert!(parse_config("fonts = \"Cascadia.ttf\"\n", Path::new(".")).is_err());
        assert!(parse_config("fonts = [\"\"]\n", Path::new(".")).is_err());
        assert!(parse_config("fonts = [\n", Path::new(".")).is_err());
    }

    #[test]
    fn empty_config_keeps_the_bundled_font() {
        let path = std::env::temp_dir().join("uterm-empty-config");
        std::fs::write(&path, "# no fonts\n").unwrap();
        let face = load(Some(&path), None, None, Vec::new()).unwrap().face;
        let _ = std::fs::remove_file(&path);
        assert!(face.valid());
    }

    #[test]
    fn missing_explicit_config_is_an_error() {
        let path = std::env::temp_dir().join("uterm-missing-config");
        let err = load(Some(&path), None, None, Vec::new()).unwrap_err();
        assert!(err.to_string().contains("uterm-missing-config"));
    }

    #[test]
    fn profile_sets_geometry_and_fonts() {
        let text = r#"
fonts = ["Cascadia.ttf"]

[profiles.ssh]
geometry = "100x30"
fonts = ["~/Fonts/unifont.ttf"]

[profiles.mail]
geometry = "128x40"
"#;
        let parsed = parse_config(text, Path::new("/cfg")).unwrap();
        let (fonts, geometry, _, _) = resolve(&parsed, Some("ssh"), None, Vec::new()).unwrap();
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            fonts,
            vec![FontSource::File(
                PathBuf::from(home).join("Fonts/unifont.ttf")
            )]
        );
        assert_eq!(
            geometry,
            Some(StartupGeometry {
                cols: 100,
                rows: 30
            })
        );

        let (fonts, geometry, _, _) = resolve(&parsed, Some("mail"), None, Vec::new()).unwrap();
        assert_eq!(
            fonts,
            vec![FontSource::File(PathBuf::from("/cfg/Cascadia.ttf"))]
        );
        assert_eq!(
            geometry,
            Some(StartupGeometry {
                cols: 128,
                rows: 40
            })
        );
    }

    #[test]
    fn builtin_profile_and_explicit_geometry() {
        let parsed = ParsedConfig::default();
        let (_, geometry, _, _) = resolve(&parsed, Some("mc"), None, Vec::new()).unwrap();
        assert_eq!(geometry, Some(StartupGeometry { cols: 80, rows: 24 }));

        let explicit = StartupGeometry { cols: 90, rows: 30 };
        let (_, geometry, _, _) = resolve(&parsed, Some("mc"), Some(explicit), Vec::new()).unwrap();
        assert_eq!(geometry, Some(explicit));

        let err = resolve(&parsed, Some("nope"), None, Vec::new()).unwrap_err();
        assert!(err.to_string().contains("unknown profile"));
    }

    #[test]
    fn profile_can_replace_a_builtin_size() {
        let text = "[profiles.mc]\ngeometry = \"100x40\"\n";
        let parsed = parse_config(text, Path::new("/cfg")).unwrap();
        let (_, geometry, _, _) = resolve(&parsed, Some("mc"), None, Vec::new()).unwrap();
        assert_eq!(
            geometry,
            Some(StartupGeometry {
                cols: 100,
                rows: 40
            })
        );
    }

    #[test]
    fn default_name_is_the_bundled_face() {
        let text = "fonts = [\"default\", \"jp.ttf\"]\n";
        let fonts = parse_config(text, Path::new("/cfg")).unwrap().fonts;
        assert_eq!(
            fonts,
            vec![
                FontSource::Bundled("default".to_owned()),
                FontSource::File(PathBuf::from("/cfg/jp.ttf")),
            ]
        );
        // A relative path keeps the file, including one spelled like the keyword.
        let text = "fonts = [\"./default\"]\n";
        let fonts = parse_config(text, Path::new("/cfg")).unwrap().fonts;
        assert_eq!(
            fonts,
            vec![FontSource::File(PathBuf::from("/cfg/./default"))]
        );
    }

    #[test]
    fn bare_names_are_families_and_paths_stay_paths() {
        let text = "fonts = [\"Menlo\", \"~/.fonts/Code.ttf\", \"/abs/face.otf\"]\n";
        let fonts = parse_config(text, Path::new("/cfg")).unwrap().fonts;
        assert_eq!(fonts[0], FontSource::Family("Menlo".to_owned()));
        assert!(matches!(fonts[1], FontSource::File(_)));
        assert!(matches!(fonts[2], FontSource::File(_)));
    }

    #[test]
    fn profile_command_and_args_are_used_unless_the_cli_overrides() {
        let text = "[profiles.mc]\ncommand = \"mc\"\nargs = [\"-d\", \"--nocolor\"]\n";
        let parsed = parse_config(text, Path::new("/cfg")).unwrap();
        let (_, _, _, command) = resolve(&parsed, Some("mc"), None, Vec::new()).unwrap();
        assert_eq!(command, ["mc", "-d", "--nocolor"]);

        let explicit = vec!["sh".to_owned(), "-l".to_owned()];
        let (_, _, _, command) = resolve(&parsed, Some("mc"), None, explicit.clone()).unwrap();
        assert_eq!(command, explicit);

        let (_, _, _, command) = resolve(&parsed, None, None, Vec::new()).unwrap();
        assert!(command.is_empty());
        assert!(parse_config("[profiles.mc]\nargs = [\"-d\"]\n", Path::new(".")).is_err());
        assert!(parse_config("[profiles.mc]\ncommand = \"\"\n", Path::new(".")).is_err());
    }

    #[test]
    fn font_size_defaults_and_profile_overrides() {
        let parsed = ParsedConfig::default();
        let (_, _, size, _) = resolve(&parsed, None, None, Vec::new()).unwrap();
        assert_eq!(size, DEFAULT_FONT_SIZE);

        let text = "font_size = 16\n[profiles.mc]\nfont_size = 18\n";
        let parsed = parse_config(text, Path::new(".")).unwrap();
        let (_, _, size, _) = resolve(&parsed, Some("mc"), None, Vec::new()).unwrap();
        assert_eq!(size, 18.0);
        let (_, _, size, _) = resolve(&parsed, Some("ssh"), None, Vec::new()).unwrap();
        assert_eq!(size, 16.0);
        assert!(parse_config("font_size = 0\n", Path::new(".")).is_err());
    }

    #[test]
    fn profile_rejects_a_bad_geometry_and_an_empty_font() {
        assert!(parse_config("[profiles.irc]\ngeometry = \"0x1\"\n", Path::new(".")).is_err());
        assert!(parse_config("[profiles.irc]\nfonts = [\"\"]\n", Path::new(".")).is_err());
    }
}
