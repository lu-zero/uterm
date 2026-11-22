//! One source for the shortcut list: rendered by winnow-args in `--help` and
//! drawn in the terminal when Ctrl+Shift+H is pressed.
//!
//! The file uses winnow-args markup (`code` and *emphasis*), which the overlay
//! strips before drawing.

/// Help text with winnow-args markup, used as the CLI's `after_help`.
pub(crate) const HELP: &str = include_str!("../keybindings.txt");

pub(crate) struct Binding {
    pub(crate) keys: String,
    pub(crate) action: String,
}

pub(crate) fn bindings() -> Vec<Binding> {
    HELP.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let (keys, action) = line.split_once("  ")?;
            Some(Binding {
                keys: strip(keys),
                action: strip(action),
            })
        })
        .collect()
}

fn strip(text: &str) -> String {
    let text = text.trim();
    let text = text.strip_prefix('`').unwrap_or(text);
    let text = text.strip_suffix('`').unwrap_or(text);
    let text = text.strip_prefix('*').unwrap_or(text);
    text.strip_suffix('*').unwrap_or(text).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_match_the_help_text() {
        let items = bindings();
        assert_eq!(items.len(), HELP.lines().count());
        assert!(items.iter().any(|b| b.keys == "Ctrl+Shift+H"));
        assert!(items
            .iter()
            .all(|b| !b.keys.contains('`') && !b.action.contains('*')));
    }
}
