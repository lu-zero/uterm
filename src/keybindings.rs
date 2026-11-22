//! The default key map: one declaration drives `--help`, the in-terminal
//! overlay, and (later) config overrides.
//!
//! Entries are `keys -> action`. winnow-args markup is added only when the
//! help string is generated; the overlay reads the plain fields.

pub(crate) struct Binding {
    pub(crate) keys: &'static str,
    pub(crate) action: &'static str,
}

macro_rules! bindings {
    ($(($keys:literal, $action:literal)),+ $(,)?) => {
        pub(crate) const BINDINGS: &[Binding] = &[
            $(Binding { keys: $keys, action: $action }),+
        ];
        /// Help text with winnow-args markup, used as the CLI's `after_help`.
        pub(crate) const HELP: &str = concat!(
            $(concat!("`", $keys, "`  *", $action, "*\n")),+
        );
    };
}

bindings!(
    ("Ctrl+Shift+T", "new tab"),
    ("Ctrl+Shift+W", "close active tab"),
    ("Ctrl+Tab / Ctrl+PageDown", "next tab"),
    ("Ctrl+Shift+Tab / Ctrl+PageUp", "previous tab"),
    ("Cmd+C / Cmd+V / Cmd+N", "copy / paste / new window (macOS)"),
    ("Ctrl+Shift+C / V / N", "copy / paste / new window (Linux)"),
    ("Ctrl+Shift+= / -", "cycle preset sizes"),
    ("Alt+Enter", "toggle resize lock"),
    ("Ctrl+Shift+H", "show this help"),
    ("Mouse click on tab", "switch tab"),
    ("Mouse click on ×", "close tab"),
);

pub(crate) fn bindings() -> impl Iterator<Item = &'static Binding> {
    BINDINGS.iter()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_is_generated_from_the_binding_map() {
        assert_eq!(bindings().count(), BINDINGS.len());
        for binding in bindings() {
            assert!(HELP.contains(&format!("`{}`  *{}*", binding.keys, binding.action)));
        }
        assert_eq!(HELP.lines().count(), BINDINGS.len());
    }
}
