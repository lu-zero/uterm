//! Resolve config font entries into `term_atlas::Face`s.
//!
//! The atlas owns rasterization, glyph caching and fallback (embedded CJK
//! subsets plus installed system fonts), so this module only maps the config
//! keywords to faces.

use crate::error::Error;
use fontique::{Collection, CollectionOptions};
use std::path::Path;
use term_atlas::Face;

/// The config keyword for the face compiled into uterm.
pub(crate) const DEFAULT_FACE: &str = "default";

pub(crate) fn bundled() -> Face {
    Face::bundled()
}

pub(crate) fn family(name: &str) -> Result<Face, Error> {
    // Face::load falls back to the bundled face for a missing family, which
    // would silently ignore a typo in config.
    let face = Face::load(name);
    if face.family() != name {
        return Err(Error::Font(format!(
            "font family `{name}` is not installed"
        )));
    }
    Ok(face)
}

pub(crate) fn file(path: &Path) -> Result<Face, Error> {
    let face = Face::from_file(path)
        .map_err(|err| Error::Font(format!("failed to read {}: {err}", path.display())))?;
    if !face.valid() {
        return Err(Error::Font(format!(
            "failed to parse {}: invalid font",
            path.display()
        )));
    }
    Ok(face)
}

/// Bundled keyword plus installed families, for `--list-fonts`.
pub(crate) fn font_families() -> Vec<String> {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: true,
    });
    let mut families: Vec<String> = collection
        .family_names()
        .filter(|name| !name.starts_with('.'))
        .map(str::to_owned)
        .collect();
    families.sort_by_key(|name| name.to_lowercase());
    families.insert(0, DEFAULT_FACE.to_owned());
    families
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_face_is_valid() {
        assert!(bundled().valid());
    }

    #[test]
    fn missing_family_is_an_error() {
        assert!(family("uterm no such family").is_err());
    }
}
