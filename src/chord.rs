//! Key chords, and the normalization every adapter funnels through so that
//! `Meta+Esc` from KDE and `super+escape` from somewhere else compare equal.

use std::fmt;
use std::str::FromStr;

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct Mods: u8 {
        const SUPER = 1;
        const CTRL = 1 << 1;
        const ALT = 1 << 2;
        const SHIFT = 1 << 3;
    }
}

impl Mods {
    /// Parses one modifier name.
    ///
    /// `meta` means the Super/logo key here, as it does in KDE and most desktop
    /// configs. Terminal and editor configs often use `meta` for Alt instead, so
    /// their adapters must map it themselves before calling this.
    pub fn parse_one(name: &str) -> Option<Mods> {
        match name.trim().to_lowercase().as_str() {
            "ctrl" | "control" => Some(Mods::CTRL),
            "alt" | "option" | "opt" => Some(Mods::ALT),
            "shift" => Some(Mods::SHIFT),
            "super" | "meta" | "win" | "logo" | "cmd" | "command" | "mod4" => Some(Mods::SUPER),
            _ => None,
        }
    }
}

/// A key name in canonical form: lowercase, with common aliases folded together.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key(String);

impl Key {
    pub fn parse(name: &str) -> Option<Key> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let lower = name
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let canonical = match lower.as_str() {
            "esc" | "escape" => "escape",
            "del" | "delete" => "delete",
            "ins" | "insert" => "insert",
            "return" | "enter" => "enter",
            "pgup" | "pageup" | "page_up" | "prior" => "pageup",
            "pgdown" | "pgdn" | "pagedown" | "page_down" | "next" => "pagedown",
            "bksp" | "backspace" => "backspace",
            "arrowup" => "up",
            "arrowdown" => "down",
            "arrowleft" => "left",
            "arrowright" => "right",
            "plus" => "+",
            "minus" => "-",
            "comma" => ",",
            "period" => ".",
            "equal" => "=",
            "grave" => "`",
            "semicolon" => ";",
            "slash" => "/",
            "backslash" => "\\",
            "apostrophe" => "'",
            "bracketleft" => "[",
            "bracketright" => "]",
            other => other,
        };
        Some(Key(canonical.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.as_str() {
            "escape" => return f.write_str("Esc"),
            "pageup" => return f.write_str("PageUp"),
            "pagedown" => return f.write_str("PageDown"),
            _ => {}
        }
        // Capitalize the start of every word: `launch (c)` -> `Launch (C)`, `on/off` -> `On/Off`.
        let mut after_word = false;
        for c in self.0.chars() {
            if after_word {
                write!(f, "{c}")?;
            } else {
                write!(f, "{}", c.to_uppercase())?;
            }
            after_word = c.is_alphanumeric();
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chord {
    pub mods: Mods,
    pub key: Key,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChordError {
    MissingKey,
    UnknownModifier(String),
}

impl fmt::Display for ChordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChordError::MissingKey => f.write_str("chord has no key"),
            ChordError::UnknownModifier(m) => write!(f, "unknown modifier `{m}`"),
        }
    }
}

impl std::error::Error for ChordError {}

impl FromStr for Chord {
    type Err = ChordError;

    /// Parses `Mod+Mod+Key`, case-insensitively. `Ctrl++` is Ctrl and the plus key.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let (head, key) = if s == "+" {
            ("", "+")
        } else if let Some(head) = s.strip_suffix("++") {
            (head, "+")
        } else {
            s.rsplit_once('+').unwrap_or(("", s))
        };

        let mut mods = Mods::empty();
        for part in head.split('+').filter(|p| !p.trim().is_empty()) {
            mods |= Mods::parse_one(part)
                .ok_or_else(|| ChordError::UnknownModifier(part.trim().to_string()))?;
        }
        let key = Key::parse(key).ok_or(ChordError::MissingKey)?;
        Ok(Chord { mods, key })
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (name, flag) in [
            ("Super", Mods::SUPER),
            ("Ctrl", Mods::CTRL),
            ("Alt", Mods::ALT),
            ("Shift", Mods::SHIFT),
        ] {
            if self.mods.contains(flag) {
                write!(f, "{name}+")?;
            }
        }
        write!(f, "{}", self.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(s: &str) -> Chord {
        s.parse().unwrap()
    }

    #[test]
    fn modifier_order_and_case_do_not_matter() {
        assert_eq!(chord("Ctrl+Shift+C"), chord("shift+ctrl+c"));
        assert_eq!(chord("Ctrl+Shift+C").to_string(), "Ctrl+Shift+C");
    }

    #[test]
    fn meta_is_super() {
        assert_eq!(chord("Meta+E"), chord("Super+e"));
        assert_eq!(chord("Meta+Ctrl+Esc").to_string(), "Super+Ctrl+Esc");
    }

    #[test]
    fn key_aliases_fold_together() {
        assert_eq!(chord("Ctrl+Esc"), chord("ctrl+escape"));
        assert_eq!(chord("Alt+PgUp"), chord("alt+Prior"));
        assert_eq!(chord("Ctrl+Return"), chord("ctrl+enter"));
        assert_eq!(chord("Super+Page_Up"), chord("super+pageup"));
    }

    #[test]
    fn plus_key() {
        let c = chord("Ctrl++");
        assert_eq!(c.mods, Mods::CTRL);
        assert_eq!(c.key.as_str(), "+");
        assert_eq!(chord("+").key.as_str(), "+");
        assert_eq!(chord("Ctrl+plus"), c);
    }

    #[test]
    fn multi_word_keys() {
        let c = chord("Shift+Volume  Up");
        assert_eq!(c.key.as_str(), "volume up");
        assert_eq!(c.to_string(), "Shift+Volume Up");
        assert_eq!(chord("Launch (C)").to_string(), "Launch (C)");
        assert_eq!(
            chord("Keyboard Light On/Off").to_string(),
            "Keyboard Light On/Off"
        );
    }

    #[test]
    fn errors() {
        assert_eq!("".parse::<Chord>(), Err(ChordError::MissingKey));
        assert_eq!("Ctrl+".parse::<Chord>(), Err(ChordError::MissingKey));
        assert_eq!(
            "Hyper+X".parse::<Chord>(),
            Err(ChordError::UnknownModifier("Hyper".into()))
        );
    }
}
