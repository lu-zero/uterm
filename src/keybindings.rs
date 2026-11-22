//! The default binding map: structured triggers mapped to actions.
//!
//! `--help`, the Ctrl+Shift+H overlay and keyboard dispatch all read this map.

use winit::keyboard::{Key, ModifiersState, NamedKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeySpec {
    Char(char),
    Named(NamedKey),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Trigger {
    Keys {
        mods: ModifiersState,
        keys: &'static [KeySpec],
    },
    /// Command modifier: super on macOS, Ctrl+Shift elsewhere.
    Command(&'static [KeySpec]),
    MouseTab,
    MouseClose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    NewTab,
    CloseActiveTab,
    NextTab,
    PrevTab,
    Copy,
    Paste,
    NewWindow,
    PresetNext,
    PresetPrev,
    LockResize,
    ToggleHelp,
    SelectTab,
    CloseTabAt,
}

pub(crate) struct Binding {
    pub(crate) label: &'static str,
    pub(crate) trigger: Trigger,
    pub(crate) action: Action,
    pub(crate) description: &'static str,
}

const CTRL_SHIFT: ModifiersState = ModifiersState::CONTROL.union(ModifiersState::SHIFT);
const CTRL: ModifiersState = ModifiersState::CONTROL;
const ALT: ModifiersState = ModifiersState::ALT;

macro_rules! bindings {
    ($(($label:literal, $trigger:expr, $action:ident, $desc:literal)),+ $(,)?) => {
        pub(crate) const BINDINGS: &[Binding] = &[
            $(Binding { label: $label, trigger: $trigger, action: Action::$action, description: $desc }),+
        ];
        /// Help text with winnow-args markup, generated from the same map.
        pub(crate) const HELP: &str = concat!(
            $(concat!("`", $label, "`  *", $desc, "*\n")),+
        );
    };
}

bindings!(
    (
        "Ctrl+Shift+T",
        Trigger::Keys {
            mods: CTRL_SHIFT,
            keys: &[KeySpec::Char('t')]
        },
        NewTab,
        "new tab"
    ),
    (
        "Ctrl+Shift+W",
        Trigger::Keys {
            mods: CTRL_SHIFT,
            keys: &[KeySpec::Char('w')]
        },
        CloseActiveTab,
        "close active tab"
    ),
    (
        "Ctrl+Tab",
        Trigger::Keys {
            mods: CTRL,
            keys: &[KeySpec::Named(NamedKey::Tab)]
        },
        NextTab,
        "next tab"
    ),
    (
        "Ctrl+PageDown",
        Trigger::Keys {
            mods: CTRL,
            keys: &[KeySpec::Named(NamedKey::PageDown)]
        },
        NextTab,
        "next page"
    ),
    (
        "Ctrl+Shift+Tab",
        Trigger::Keys {
            mods: CTRL_SHIFT,
            keys: &[KeySpec::Named(NamedKey::Tab)]
        },
        PrevTab,
        "previous tab"
    ),
    (
        "Ctrl+PageUp",
        Trigger::Keys {
            mods: CTRL,
            keys: &[KeySpec::Named(NamedKey::PageUp)]
        },
        PrevTab,
        "previous page"
    ),
    (
        "Cmd+C / Ctrl+Shift+C",
        Trigger::Command(&[KeySpec::Char('c')]),
        Copy,
        "copy selection"
    ),
    (
        "Cmd+V / Ctrl+Shift+V",
        Trigger::Command(&[KeySpec::Char('v')]),
        Paste,
        "paste clipboard"
    ),
    (
        "Cmd+N / Ctrl+Shift+N",
        Trigger::Command(&[KeySpec::Char('n')]),
        NewWindow,
        "new window"
    ),
    (
        "Ctrl+Shift+= / -",
        Trigger::Keys {
            mods: CTRL_SHIFT,
            keys: &[KeySpec::Char('='), KeySpec::Char('+')]
        },
        PresetNext,
        "cycle preset sizes"
    ),
    (
        "Ctrl+Shift+-",
        Trigger::Keys {
            mods: CTRL_SHIFT,
            keys: &[KeySpec::Char('-'), KeySpec::Char('_')]
        },
        PresetPrev,
        "cycle preset sizes back"
    ),
    (
        "Alt+Enter",
        Trigger::Keys {
            mods: ALT,
            keys: &[KeySpec::Named(NamedKey::Enter)]
        },
        LockResize,
        "toggle resize lock"
    ),
    (
        "Ctrl+Shift+H",
        Trigger::Keys {
            mods: CTRL_SHIFT,
            keys: &[KeySpec::Char('h')]
        },
        ToggleHelp,
        "show this help"
    ),
    (
        "Mouse click on tab",
        Trigger::MouseTab,
        SelectTab,
        "switch tab"
    ),
    (
        "Mouse click on ×",
        Trigger::MouseClose,
        CloseTabAt,
        "close tab"
    ),
);

pub(crate) fn bindings() -> impl Iterator<Item = &'static Binding> {
    BINDINGS.iter()
}

pub(crate) fn lookup_key(mods: ModifiersState, key: &Key) -> Option<Action> {
    BINDINGS
        .iter()
        .find(|binding| match binding.trigger {
            Trigger::Keys { mods: wanted, keys } => {
                mods == wanted && keys.iter().any(|spec| matches_spec(*spec, key))
            }
            Trigger::Command(keys) => {
                crate::cli::is_command_modifier(mods)
                    && keys.iter().any(|spec| matches_spec(*spec, key))
            }
            Trigger::MouseTab | Trigger::MouseClose => false,
        })
        .map(|binding| binding.action)
}

/// Expand a help label into the individual keys it documents, repeating the
/// modifier prefix for bare alternatives (`Ctrl+Shift+= / -`).
#[cfg(test)]
fn label_keys() -> Vec<String> {
    let mut keys = Vec::new();
    for binding in bindings() {
        let parts: Vec<_> = binding.label.split(" / ").collect();
        let prefix = parts[0]
            .rsplit_once('+')
            .map(|(mods, _)| format!("{mods}+"))
            .unwrap_or_default();
        for part in parts {
            keys.push(if part.contains('+') {
                part.to_owned()
            } else {
                format!("{prefix}{part}")
            });
        }
    }
    keys
}

pub(crate) fn mouse_action(close: bool) -> Action {
    if close {
        Action::CloseTabAt
    } else {
        Action::SelectTab
    }
}

fn matches_spec(spec: KeySpec, key: &Key) -> bool {
    match (spec, key) {
        (KeySpec::Char(expected), Key::Character(text)) => {
            text.chars().next().and_then(|ch| ch.to_lowercase().next()) == Some(expected)
        }
        (KeySpec::Named(expected), Key::Named(actual)) => expected == *actual,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_is_generated_from_the_binding_map() {
        assert_eq!(HELP.lines().count(), BINDINGS.len());
        for binding in bindings() {
            assert!(HELP.contains(&format!("`{}`  *{}*", binding.label, binding.description)));
        }
    }

    #[test]
    fn readme_lists_every_binding() {
        let readme = include_str!("../README.md").replace("**", "");
        for key in label_keys() {
            assert!(readme.contains(&key), "README is missing `{key}`");
        }
    }

    #[test]
    fn lookup_returns_actions_for_real_key_events() {
        assert_eq!(
            lookup_key(CTRL_SHIFT, &Key::Character("t".into())),
            Some(Action::NewTab)
        );
        assert_eq!(
            lookup_key(CTRL, &Key::Named(NamedKey::PageDown)),
            Some(Action::NextTab)
        );
        assert_eq!(
            lookup_key(ModifiersState::SUPER, &Key::Character("c".into())),
            Some(Action::Copy)
        );
        assert_eq!(
            lookup_key(ModifiersState::empty(), &Key::Character("t".into())),
            None
        );
    }
}
