//! Font list and named profiles from `config.toml`.
//!
//! Search order, first hit wins:
//! 1. `--config PATH`
//! 2. `$UTERM_CONFIG`
//! 3. `$XDG_CONFIG_HOME/uterm/config.toml`
//! 4. `~/.config/uterm/config.toml`
//!
//! A missing default file keeps the bundled Cascadia Code. A path given with
//! `--config` or `$UTERM_CONFIG` must exist. `fonts` is a fallback list, tried
//! in order. The first face sets the cell size. Each later face adds glyphs
//! the earlier faces lack. The name `cascadia` is the bundled face, so
//! `fonts = ["cascadia", "coverage.ttf"]` keeps Cascadia's Latin and takes
//! other scripts from a second face of the user's choosing.
//!
//! `[profiles.NAME]` may set `geometry` (`"128x40"`) and its own `fonts`.
//! `--profile sh|mc|ssh` still has a built-in size when the file does not
//! replace it. `--geometry` wins over either size. An unknown name is an error.

use crate::cli::{self, StartupGeometry};
use crate::error::Error;
use fontdue::{Font, FontSettings};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    #[serde(default)]
    fonts: Vec<String>,
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
}

/// One entry in a font list. `cascadia` is the face compiled into the binary.
/// Any other string is a file path.
#[derive(Clone, Debug, PartialEq, Eq)]
enum FontSource {
    Bundled,
    File(PathBuf),
}

#[derive(Default)]
struct ParsedConfig {
    fonts: Vec<FontSource>,
    profiles: BTreeMap<String, ParsedProfile>,
}

struct ParsedProfile {
    geometry: Option<StartupGeometry>,
    fonts: Option<Vec<FontSource>>,
}

#[derive(Debug)]
pub(crate) struct Session {
    pub(crate) fonts: Vec<Font>,
    pub(crate) geometry: Option<StartupGeometry>,
}

pub(crate) fn load(
    explicit: Option<&Path>,
    profile: Option<&str>,
    explicit_geometry: Option<StartupGeometry>,
) -> Result<Session, Error> {
    let parsed = read_config(explicit)?;
    let (paths, geometry) = resolve(&parsed, profile, explicit_geometry)?;
    Ok(Session {
        fonts: load_paths(&paths)?,
        geometry,
    })
}

fn load_paths(sources: &[FontSource]) -> Result<Vec<Font>, Error> {
    if sources.is_empty() {
        return Ok(vec![bundled_font()?]);
    }
    let mut fonts = Vec::with_capacity(sources.len());
    for source in sources {
        let font = match source {
            FontSource::Bundled => bundled_font()?,
            FontSource::File(path) => {
                let bytes = std::fs::read(path).map_err(|err| {
                    Error::Font(format!("failed to read {}: {err}", path.display()))
                })?;
                Font::from_bytes(bytes, FontSettings::default()).map_err(|err| {
                    Error::Font(format!("failed to parse {}: {err}", path.display()))
                })?
            }
        };
        fonts.push(font);
    }
    Ok(fonts)
}

const CASCADIA_TTF: &[u8] = include_bytes!("../CascadiaCodeNF.ttf");

/// The name of the bundled Cascadia Code NF face inside a `fonts` list.
const BUNDLED_FACE: &str = "cascadia";

pub(crate) fn bundled_font() -> Result<Font, Error> {
    Font::from_bytes(CASCADIA_TTF, FontSettings::default())
        .map_err(|err| Error::Font(format!("failed to load the bundled font: {err}")))
}

fn read_config(explicit: Option<&Path>) -> Result<ParsedConfig, Error> {
    let Some(path) = resolve_config_path(explicit) else {
        return Ok(ParsedConfig::default());
    };
    let required = explicit.is_some() || std::env::var_os("UTERM_CONFIG").is_some();
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_config(&text, path.parent().unwrap_or(Path::new("."))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound && !required => {
            Ok(ParsedConfig::default())
        }
        Err(err) => Err(Error::Font(format!(
            "failed to read config {}: {err}",
            path.display()
        ))),
    }
}

fn resolve_config_path(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path.to_path_buf());
    }
    if let Some(path) = std::env::var_os("UTERM_CONFIG") {
        return Some(PathBuf::from(path));
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
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
        toml::from_str(text).map_err(|err| Error::Usage(format!("invalid config: {err}")))?;
    let fonts = font_list(&file.fonts, config_dir)?;
    let mut profiles = BTreeMap::new();
    for (name, profile) in file.profiles {
        if name.is_empty() {
            return Err(Error::Usage("config profile name is empty".to_string()));
        }
        let geometry = match profile.geometry.as_deref() {
            Some(raw) => Some(geometry_from(raw)?),
            None => None,
        };
        let fonts = match profile.fonts {
            Some(list) => Some(font_list(&list, config_dir)?),
            None => None,
        };
        profiles.insert(name, ParsedProfile { geometry, fonts });
    }
    Ok(ParsedConfig { fonts, profiles })
}

fn font_list(raw_paths: &[String], config_dir: &Path) -> Result<Vec<FontSource>, Error> {
    let mut sources = Vec::with_capacity(raw_paths.len());
    for raw in raw_paths {
        if raw.is_empty() {
            return Err(Error::Usage("config font path is empty".to_string()));
        }
        if raw == BUNDLED_FACE {
            sources.push(FontSource::Bundled);
        } else {
            sources.push(FontSource::File(expand_font_path(raw, config_dir)));
        }
    }
    Ok(sources)
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
) -> Result<(Vec<FontSource>, Option<StartupGeometry>), Error> {
    let Some(name) = profile else {
        return Ok((parsed.fonts.clone(), explicit_geometry));
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
    Ok((fonts, geometry))
}

fn expand_font_path(raw: &str, config_dir: &Path) -> PathBuf {
    let path = if raw == "~" {
        home_dir().unwrap_or_else(|| PathBuf::from(raw))
    } else if let Some(rest) = raw.strip_prefix("~/") {
        match home_dir() {
            Some(home) => home.join(rest),
            None => PathBuf::from(raw),
        }
    } else {
        PathBuf::from(raw)
    };
    if path.is_absolute() {
        path
    } else {
        config_dir.join(path)
    }
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
        let fonts = load(Some(&path), None, None).unwrap().fonts;
        let _ = std::fs::remove_file(&path);
        assert_eq!(fonts.len(), 1);
        assert!(fonts[0].has_glyph('M'));
    }

    #[test]
    fn missing_explicit_config_is_an_error() {
        let path = std::env::temp_dir().join("uterm-missing-config");
        let err = load(Some(&path), None, None).unwrap_err();
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
        let (fonts, geometry) = resolve(&parsed, Some("ssh"), None).unwrap();
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

        let (fonts, geometry) = resolve(&parsed, Some("mail"), None).unwrap();
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
        let (_, geometry) = resolve(&parsed, Some("mc"), None).unwrap();
        assert_eq!(geometry, Some(StartupGeometry { cols: 80, rows: 24 }));

        let explicit = StartupGeometry { cols: 90, rows: 30 };
        let (_, geometry) = resolve(&parsed, Some("mc"), Some(explicit)).unwrap();
        assert_eq!(geometry, Some(explicit));

        let err = resolve(&parsed, Some("nope"), None).unwrap_err();
        assert!(err.to_string().contains("unknown profile"));
    }

    #[test]
    fn profile_can_replace_a_builtin_size() {
        let text = "[profiles.mc]\ngeometry = \"100x40\"\n";
        let parsed = parse_config(text, Path::new("/cfg")).unwrap();
        let (_, geometry) = resolve(&parsed, Some("mc"), None).unwrap();
        assert_eq!(
            geometry,
            Some(StartupGeometry {
                cols: 100,
                rows: 40
            })
        );
    }

    #[test]
    fn cascadia_name_is_the_bundled_face() {
        let text = "fonts = [\"cascadia\", \"jp.ttf\"]\n";
        let fonts = parse_config(text, Path::new("/cfg")).unwrap().fonts;
        assert_eq!(
            fonts,
            vec![
                FontSource::Bundled,
                FontSource::File(PathBuf::from("/cfg/jp.ttf")),
            ]
        );
        // A relative path keeps the file, including one spelled like the keyword.
        let text = "fonts = [\"./cascadia\"]\n";
        let fonts = parse_config(text, Path::new("/cfg")).unwrap().fonts;
        assert_eq!(
            fonts,
            vec![FontSource::File(PathBuf::from("/cfg/./cascadia"))]
        );
    }

    #[test]
    fn profile_rejects_a_bad_geometry_and_an_empty_font() {
        assert!(parse_config("[profiles.irc]\ngeometry = \"0x1\"\n", Path::new(".")).is_err());
        assert!(parse_config("[profiles.irc]\nfonts = [\"\"]\n", Path::new(".")).is_err());
    }
}
