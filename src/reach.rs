//! Shortcuts that can't arrive: keys a terminal can't send to the program inside it.
//!
//! Terminals traditionally send keys as characters and escape sequences, and
//! some combinations come out identical to others: `Ctrl+Shift+Z` is sent as
//! `Ctrl+Z`, `Shift+Enter` as `Enter`, and Super isn't sent at all. A shell or
//! app bound to one of those never sees it. The kitty keyboard protocol fixes
//! this, but only when the terminal supports it *and* the program turns it on.

use crate::binding::{Binding, Layer};
use crate::chord::{Chord, Key, Mods};

/// How a terminal without the kitty keyboard protocol sends a key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Legacy {
    /// As itself.
    Distinct,
    /// The same way as another key press, so programs can't tell them apart.
    SameAs(Chord),
}

/// How the classic (xterm-style) encoding sends `chord`. Only well-known
/// collisions are listed; anything else counts as distinct.
pub fn legacy(chord: &Chord) -> Legacy {
    let key = chord.key.as_str();
    let mods = chord.mods;

    // Terminals don't send Super; the program gets the key without it.
    if mods.contains(Mods::SUPER) {
        return same_as(mods - Mods::SUPER, key);
    }
    // Alt is sent as ESC before the key, so it collides whenever the rest does.
    if mods.contains(Mods::ALT) {
        let without_alt = Chord {
            mods: mods - Mods::ALT,
            key: chord.key.clone(),
        };
        return match legacy(&without_alt) {
            Legacy::SameAs(other) => Legacy::SameAs(Chord {
                mods: other.mods | Mods::ALT,
                key: other.key,
            }),
            Legacy::Distinct => Legacy::Distinct,
        };
    }

    let ctrl_shift = Mods::CTRL | Mods::SHIFT;
    let is_letter = key.len() == 1 && key.chars().all(|c| c.is_ascii_lowercase());
    match (mods, key) {
        // Control characters ignore Shift.
        (m, _) if m == ctrl_shift && is_letter => same_as(Mods::CTRL, key),
        // Enter and Space have no modified forms.
        (m, "enter") if !m.is_empty() => same_as(Mods::empty(), "enter"),
        (Mods::SHIFT, "space") => same_as(Mods::empty(), "space"),
        (m, "space") if m == ctrl_shift => same_as(Mods::CTRL, "space"),
        (Mods::SHIFT, "backspace") => same_as(Mods::empty(), "backspace"),
        (m, "escape") if !m.is_empty() => same_as(Mods::empty(), "escape"),
        // Ctrl is dropped from Tab.
        (Mods::CTRL, "tab") => same_as(Mods::empty(), "tab"),
        (m, "tab") if m == ctrl_shift => same_as(Mods::SHIFT, "tab"),
        // Control characters that are other keys.
        (Mods::CTRL, "m") => same_as(Mods::empty(), "enter"),
        (Mods::CTRL, "i") => same_as(Mods::empty(), "tab"),
        (Mods::CTRL, "[") => same_as(Mods::empty(), "escape"),
        // Punctuation and digits with no control character are sent plain.
        (Mods::CTRL, ";" | "," | "." | "'" | "=" | "1" | "9" | "0") => same_as(Mods::empty(), key),
        _ => Legacy::Distinct,
    }
}

fn same_as(mods: Mods, key: &str) -> Legacy {
    match Key::parse(key) {
        Some(key) => Legacy::SameAs(Chord { mods, key }),
        None => Legacy::Distinct,
    }
}

/// A terminal that's set up, and whether it supports the kitty keyboard protocol.
#[derive(Debug, Clone, Copy)]
pub struct Terminal {
    pub tool: &'static str,
    pub kitty_keyboard: bool,
}

/// A shortcut that can't arrive in some or all of the terminals.
#[derive(Debug)]
pub struct Unreachable<'a> {
    pub binding: &'a Binding,
    /// The key that can't be sent, and what it's sent as instead.
    pub key: Chord,
    pub sent_as: Chord,
    /// Terminals where it arrives anyway, thanks to the kitty keyboard protocol.
    pub works_in: Vec<&'static str>,
    /// Terminals where it doesn't. Empty if no terminal was found.
    pub fails_in: Vec<&'static str>,
    /// Whether the program turns on the kitty keyboard protocol.
    pub program_asks: bool,
}

/// Every shortcut of a shell, multiplexer or app that can't arrive in at least
/// one of `terminals`. `kitty_programs` are the tools that turn on the kitty
/// keyboard protocol.
pub fn unreachable<'a>(
    bindings: &'a [Binding],
    terminals: &[Terminal],
    kitty_programs: &[&str],
) -> Vec<Unreachable<'a>> {
    let mut found = Vec::new();
    for binding in bindings.iter().filter(|b| b.layer > Layer::Terminal) {
        let mut keys = std::iter::once(&binding.chord).chain(&binding.then);
        let Some((key, sent_as)) = keys.find_map(|key| match legacy(key) {
            Legacy::SameAs(other) if !does_the_same(bindings, binding, &other) => {
                Some((key.clone(), other))
            }
            _ => None,
        }) else {
            continue;
        };

        let asks = kitty_programs.contains(&binding.tool);
        let (works_in, fails_in): (Vec<&Terminal>, Vec<&Terminal>) = terminals
            .iter()
            .partition(|terminal| asks && terminal.kitty_keyboard);
        if !works_in.is_empty() && fails_in.is_empty() {
            continue;
        }
        found.push(Unreachable {
            binding,
            key,
            sent_as,
            works_in: works_in.iter().map(|t| t.tool).collect(),
            fails_in: fails_in.iter().map(|t| t.tool).collect(),
            program_asks: asks,
        });
    }
    found
}

/// Whether the tool also binds `other` to the same action in the same mode, in
/// which case it doesn't matter that the two arrive the same way.
fn does_the_same(bindings: &[Binding], binding: &Binding, other: &Chord) -> bool {
    bindings.iter().any(|b| {
        b.tool == binding.tool
            && b.mode == binding.mode
            && b.action == binding.action
            && &b.chord == other
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binding::{Grabs, Source};

    fn chord(s: &str) -> Chord {
        s.parse().unwrap()
    }

    fn sent_as(s: &str) -> Option<String> {
        match legacy(&chord(s)) {
            Legacy::SameAs(other) => Some(other.to_string()),
            Legacy::Distinct => None,
        }
    }

    #[test]
    fn known_collisions() {
        assert_eq!(sent_as("Ctrl+Shift+Z").as_deref(), Some("Ctrl+Z"));
        assert_eq!(sent_as("Ctrl+Alt+Shift+Z").as_deref(), Some("Ctrl+Alt+Z"));
        assert_eq!(sent_as("Shift+Enter").as_deref(), Some("Enter"));
        assert_eq!(sent_as("Ctrl+Enter").as_deref(), Some("Enter"));
        assert_eq!(sent_as("Super+E").as_deref(), Some("E"));
        assert_eq!(sent_as("Ctrl+Tab").as_deref(), Some("Tab"));
        assert_eq!(sent_as("Ctrl+;").as_deref(), Some(";"));
        assert_eq!(sent_as("Ctrl+M").as_deref(), Some("Enter"));
        assert_eq!(sent_as("Shift+Space").as_deref(), Some("Space"));
    }

    #[test]
    fn keys_that_arrive_as_themselves() {
        for key in [
            "Ctrl+Z",
            "Alt+Z",
            "Alt+Shift+F",
            "Ctrl+Shift+Up",
            "Shift+Tab",
            "Ctrl+Space",
            "Alt+Enter",
            "Ctrl+Alt+H",
            "Shift+PageUp",
            "F5",
        ] {
            assert_eq!(sent_as(key), None, "{key}");
        }
    }

    fn bind(keys: &str, action: &str, tool: &'static str, layer: Layer) -> Binding {
        Binding {
            chord: chord(keys),
            action: action.into(),
            tool,
            layer,
            grabs: Grabs::Always,
            mode: None,
            then: Vec::new(),
            source: Source::Default,
        }
    }

    const ALACRITTY: Terminal = Terminal {
        tool: "alacritty",
        kitty_keyboard: true,
    };
    const OLD_TERM: Terminal = Terminal {
        tool: "oldterm",
        kitty_keyboard: false,
    };

    #[test]
    fn kitty_protocol_needs_the_terminal_and_the_program() {
        let bs = [bind("Ctrl+Shift+Z", "redo", "fish", Layer::Shell)];

        // fish turns the protocol on, and Alacritty supports it: fine.
        assert!(unreachable(&bs, &[ALACRITTY], &["fish"]).is_empty());

        // A terminal without it: reported, saying where it does work.
        let found = unreachable(&bs, &[ALACRITTY, OLD_TERM], &["fish"]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].sent_as, chord("Ctrl+Z"));
        assert_eq!(found[0].works_in, ["alacritty"]);
        assert_eq!(found[0].fails_in, ["oldterm"]);

        // A program that doesn't turn it on can't get it anywhere.
        let found = unreachable(&bs, &[ALACRITTY], &[]);
        assert_eq!(found[0].fails_in, ["alacritty"]);
    }

    #[test]
    fn same_action_on_the_colliding_key_is_fine() {
        let bs = [
            bind("Ctrl+M", "ExecuteCommand", "micro", Layer::App),
            bind("Enter", "ExecuteCommand", "micro", Layer::App),
        ];
        assert!(unreachable(&bs, &[OLD_TERM], &[]).is_empty());
    }

    #[test]
    fn terminal_and_desktop_shortcuts_are_not_checked() {
        let bs = [
            bind("Ctrl+Shift+C", "Copy", "konsole", Layer::Terminal),
            bind("Super+E", "Dolphin", "kde", Layer::Compositor),
        ];
        assert!(unreachable(&bs, &[OLD_TERM], &[]).is_empty());
    }
}
