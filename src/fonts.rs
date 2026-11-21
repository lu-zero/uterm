//! Font faces: bundled defaults, installed families, and glyph fallback.
//!
//! A `fonts` entry in the config is either the bundled keyword `maple`, a
//! file path, or an installed family name. `fontique` resolves
//! family names and finds a system face when a codepoint is missing; fontdue
//! rasterizes the bytes either way.

use crate::error::Error;
use fontdue::{Font, FontSettings};
use fontique::{
    Collection, CollectionOptions, FallbackKey, FontStyle, FontWeight, FontWidth, GenericFamily,
    QueryFamily, QueryStatus, Script, SourceCache, SourceCacheOptions,
};

/// The bundled face: Maple Mono NF, with Nerd Font icons and Latin/Cyrillic.
pub(crate) const MAPLE_TTF: &[u8] = include_bytes!("../MapleMonoNF.ttf");
pub(crate) const MAPLE_FACE: &str = "maple";

/// The bundled default, also used as the primary family for fallback.
pub(crate) const DEFAULT_FACE: &str = MAPLE_FACE;

pub(crate) fn font_from(bytes: &[u8], index: u32, label: &str) -> Result<Font, Error> {
    Font::from_bytes(
        bytes,
        FontSettings {
            collection_index: index,
            ..FontSettings::default()
        },
    )
    .map_err(|err| Error::Font(format!("failed to parse {label}: {err}")))
}

pub(crate) fn bundled_font(name: &str) -> Result<Font, Error> {
    if name != MAPLE_FACE {
        return Err(Error::Font(format!("unknown bundled font `{name}`")));
    }
    font_from(MAPLE_TTF, 0, name)
}

fn collection() -> Collection {
    Collection::new(CollectionOptions {
        shared: false,
        system_fonts: true,
    })
}

/// The regular face of an installed family, as font bytes and a TTC index.
pub(crate) fn family_font(family: &str) -> Option<(Vec<u8>, u32)> {
    let mut collection = collection();
    let info = collection.family_by_name(family)?;
    let font = info.match_font(
        FontWidth::NORMAL,
        FontStyle::Normal,
        FontWeight::NORMAL,
        false,
    )?;
    let blob = font.load(None)?;
    Some((blob.as_ref().to_vec(), font.index()))
}

/// Installed families, sorted, with the bundled keyword first.
pub(crate) fn font_families() -> Vec<String> {
    let mut collection = collection();
    let mut families: Vec<String> = collection
        .family_names()
        .filter(|name| !name.starts_with('.'))
        .map(str::to_owned)
        .collect();
    families.sort_by_key(|name| name.to_lowercase());
    families.insert(0, MAPLE_FACE.to_owned());
    families
}

/// Find an installed font covering `ch`, preferring `primary_family` and then
/// monospace, system UI and sans families before the platform's script fallbacks.
pub(crate) fn discover(primary_family: &str, ch: char) -> Option<(Vec<u8>, u32)> {
    let mut collection = collection();
    let mut cache = SourceCache::new(SourceCacheOptions::default());
    let mut query = collection.query(&mut cache);
    query.set_families([
        QueryFamily::Named(primary_family),
        QueryFamily::Generic(GenericFamily::Monospace),
        QueryFamily::Generic(GenericFamily::SystemUi),
        QueryFamily::Generic(GenericFamily::SansSerif),
    ]);
    query.set_fallbacks(FallbackKey::new(script_of(ch), None));

    let mut found = None;
    query.matches_with(|font| {
        if font
            .charmap()
            .and_then(|charmap| charmap.map(ch))
            .is_some_and(|glyph| glyph != 0)
        {
            found = Some((font.blob.as_ref().to_vec(), font.index));
            QueryStatus::Stop
        } else {
            QueryStatus::Continue
        }
    });
    found
}

/// The script whose fallback families are most likely to cover `ch`. Fontique
/// has no char-to-script routine; these are the blocks a terminal meets most.
fn script_of(ch: char) -> Script {
    let cp = u32::from(ch);
    let tag = if (0x3040..=0x30FF).contains(&cp) {
        *b"Kana"
    } else if (0x4E00..=0x9FFF).contains(&cp) {
        *b"Hani"
    } else if (0xAC00..=0xD7AF).contains(&cp) {
        *b"Hang"
    } else if (0x0400..=0x052F).contains(&cp) {
        *b"Cyrl"
    } else if (0x0370..=0x03FF).contains(&cp) {
        *b"Grek"
    } else if (0x0590..=0x05FF).contains(&cp) {
        *b"Hebr"
    } else if (0x0600..=0x06FF).contains(&cp) {
        *b"Arab"
    } else if (0x0900..=0x097F).contains(&cp) {
        *b"Deva"
    } else if (0x0E00..=0x0E7F).contains(&cp) {
        *b"Thai"
    } else {
        *b"Zyyy"
    };
    Script::from_bytes(tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_faces_load_and_have_their_expected_coverage() {
        let maple = bundled_font(MAPLE_FACE).unwrap();
        assert!(maple.has_glyph('A'));
        assert!(bundled_font("no such face").is_err());
    }

    #[test]
    fn discovery_finds_a_font_that_covers_the_codepoint() {
        let Some((bytes, index)) = discover(MAPLE_FACE, '日') else {
            return;
        };
        let face = font_from(&bytes, index, "discovered").unwrap();
        assert!(face.has_glyph('日'));
    }

    #[test]
    fn enumeration_starts_with_the_bundled_face() {
        let families = font_families();
        assert_eq!(families[0], MAPLE_FACE);
    }
}
