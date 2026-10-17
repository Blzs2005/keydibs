//! Decodes the bytes a terminal sends for a key press back into the key, for
//! tools that bind raw sequences like `\e[1;5C` (Ctrl+Right).
//!
//! Covers the xterm encoding most terminals use (`CSI` with a modifier
//! parameter, `SS3`, `CSI … ~`), the kitty keyboard protocol's `CSI … u`, Alt as
//! an `ESC` prefix, and plain control characters. Anything else is `None`.

use crate::chord::{Chord, Key, Mods};

const ESC: char = '\x1b';

/// The key `sequence` stands for, if it's one key press.
pub(crate) fn decode(sequence: &str) -> Option<Chord> {
    let mut chars = sequence.chars();
    match chars.next()? {
        ESC => {}
        c => return control(c),
    }
    let rest = chars.as_str();
    match rest.chars().next() {
        None => chord(Mods::empty(), "escape"),
        Some('[') => csi(&rest[1..]),
        Some('O') if rest.len() > 1 => ss3(&rest[1..]),
        // Alt sends ESC before the key.
        Some(_) => {
            let mut inner = decode(rest)?;
            inner.mods |= Mods::ALT;
            Some(inner)
        }
    }
}

/// The keys `sequence` stands for when it's several key presses in a row, like
/// `\x18\x05` (Ctrl+X, Ctrl+E). `None` if any of them isn't a key.
pub(crate) fn decode_keys(sequence: &str) -> Option<Vec<Chord>> {
    let mut keys = Vec::new();
    let mut rest = sequence;
    while !rest.is_empty() {
        let len = key_length(rest)?;
        keys.push(decode(&rest[..len])?);
        rest = &rest[len..];
    }
    Some(keys)
}

/// The length in bytes of the first key press in `sequence`.
fn key_length(sequence: &str) -> Option<usize> {
    let first = sequence.chars().next()?;
    if first != ESC {
        return Some(first.len_utf8());
    }
    let rest = &sequence[1..];
    match rest.chars().next() {
        None => Some(1),
        // Parameters, then a final character from `@` to `~`.
        Some('[') => {
            let end = rest[1..].find(|c: char| ('@'..='~').contains(&c))?;
            Some(2 + end + 1)
        }
        Some('O') if rest.len() > 1 => Some(2 + rest[1..].chars().next()?.len_utf8()),
        // Alt and a key that's itself a sequence, like Alt+Up.
        Some(ESC) => Some(1 + key_length(rest)?),
        Some(c) => Some(1 + c.len_utf8()),
    }
}

/// The control character `Ctrl` and `c` type: `a` is `\x01`, `[` is Escape,
/// `?` is Delete.
pub(crate) fn control_char(c: char) -> Option<char> {
    if c == '?' {
        return Some('\x7f');
    }
    let upper = c.to_ascii_uppercase();
    ('@'..='_')
        .contains(&upper)
        .then_some((upper as u8 ^ 0x40) as char)
}

/// A key press that's a single character: a control character, or a printable
/// one (uppercase letters meaning Shift).
fn control(c: char) -> Option<Chord> {
    match c {
        '\r' | '\n' => chord(Mods::empty(), "enter"),
        '\t' => chord(Mods::empty(), "tab"),
        '\x7f' => chord(Mods::empty(), "backspace"),
        '\0' => chord(Mods::CTRL, "space"),
        '\x01'..='\x1a' => chord(Mods::CTRL, &((c as u8 + b'a' - 1) as char).to_string()),
        '\x1c' => chord(Mods::CTRL, "\\"),
        '\x1d' => chord(Mods::CTRL, "]"),
        '\x1e' => chord(Mods::CTRL, "^"),
        '\x1f' => chord(Mods::CTRL, "_"),
        c if c.is_ascii_uppercase() => chord(Mods::SHIFT, &c.to_ascii_lowercase().to_string()),
        c if c.is_ascii_graphic() => chord(Mods::empty(), &c.to_string()),
        _ => None,
    }
}

/// `CSI` sequences, after the `ESC [`: `A`, `1;5C`, `3~`, `15;2~`, `97;5u`.
fn csi(body: &str) -> Option<Chord> {
    let last = body.chars().last()?;
    let params: Vec<u32> = body[..body.len() - last.len_utf8()]
        .split(';')
        .map(|p| {
            if p.is_empty() {
                Some(1)
            } else {
                p.parse().ok()
            }
        })
        .collect::<Option<_>>()?;
    let first = params.first().copied().unwrap_or(1);
    // `CSI 5 D` (rxvt's Ctrl+Left) isn't xterm's encoding; don't guess.
    if params.len() == 1 && first != 1 && !matches!(last, '~' | 'u') {
        return None;
    }
    let mods = modifiers(params.get(1).copied().unwrap_or(1))?;

    let key = match last {
        'A' => "up",
        'B' => "down",
        'C' => "right",
        'D' => "left",
        'H' => "home",
        'F' => "end",
        'P' => "f1",
        'Q' => "f2",
        'R' => "f3",
        'S' => "f4",
        'Z' => return chord(mods | Mods::SHIFT, "tab"),
        '~' => tilde_key(first)?,
        'u' => return kitty_key(first, mods),
        _ => return None,
    };
    chord(mods, key)
}

/// `SS3` sequences, after the `ESC O`: arrows and F1 to F4 in some modes.
fn ss3(body: &str) -> Option<Chord> {
    let key = match body {
        "A" => "up",
        "B" => "down",
        "C" => "right",
        "D" => "left",
        "H" => "home",
        "F" => "end",
        "P" => "f1",
        "Q" => "f2",
        "R" => "f3",
        "S" => "f4",
        _ => return None,
    };
    chord(Mods::empty(), key)
}

/// The key of `CSI <n> ~`.
fn tilde_key(n: u32) -> Option<&'static str> {
    Some(match n {
        1 | 7 => "home",
        2 => "insert",
        3 => "delete",
        4 | 8 => "end",
        5 => "pageup",
        6 => "pagedown",
        11 => "f1",
        12 => "f2",
        13 => "f3",
        14 => "f4",
        15 => "f5",
        17 => "f6",
        18 => "f7",
        19 => "f8",
        20 => "f9",
        21 => "f10",
        23 => "f11",
        24 => "f12",
        _ => return None,
    })
}

/// The kitty keyboard protocol's `CSI <codepoint> ; <mods> u`.
fn kitty_key(codepoint: u32, mods: Mods) -> Option<Chord> {
    let key = match codepoint {
        9 => "tab".to_string(),
        13 => "enter".to_string(),
        27 => "escape".to_string(),
        32 => "space".to_string(),
        127 => "backspace".to_string(),
        c => char::from_u32(c)
            .filter(char::is_ascii_graphic)?
            .to_string(),
    };
    chord(mods, &key)
}

/// The modifier parameter: 1 plus a bit mask of Shift (1), Alt (2), Ctrl (4)
/// and Super (8, as in the kitty protocol). Higher bits (Hyper, Meta, Caps
/// Lock, Num Lock) aren't modeled.
fn modifiers(param: u32) -> Option<Mods> {
    let bits = param.checked_sub(1)?;
    if bits & !0b1111 != 0 {
        return None;
    }
    let mut mods = Mods::empty();
    for (bit, flag) in [
        (1, Mods::SHIFT),
        (2, Mods::ALT),
        (4, Mods::CTRL),
        (8, Mods::SUPER),
    ] {
        if bits & bit != 0 {
            mods |= flag;
        }
    }
    Some(mods)
}

fn chord(mods: Mods, key: &str) -> Option<Chord> {
    Some(Chord {
        mods,
        key: Key::parse(key)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded(sequence: &str) -> Option<String> {
        decode(sequence).map(|c| c.to_string())
    }

    #[test]
    fn csi_with_modifiers() {
        assert_eq!(decoded("\x1b[A").as_deref(), Some("Up"));
        assert_eq!(decoded("\x1b[1;5C").as_deref(), Some("Ctrl+Right"));
        assert_eq!(decoded("\x1b[1;3D").as_deref(), Some("Alt+Left"));
        assert_eq!(decoded("\x1b[1;9A").as_deref(), Some("Super+Up"));
        assert_eq!(decoded("\x1b[3~").as_deref(), Some("Delete"));
        assert_eq!(decoded("\x1b[5;5~").as_deref(), Some("Ctrl+PageUp"));
        assert_eq!(decoded("\x1b[15;2~").as_deref(), Some("Shift+F5"));
        assert_eq!(decoded("\x1b[Z").as_deref(), Some("Shift+Tab"));
    }

    #[test]
    fn ss3_and_kitty() {
        assert_eq!(decoded("\x1bOA").as_deref(), Some("Up"));
        assert_eq!(decoded("\x1bOP").as_deref(), Some("F1"));
        assert_eq!(decoded("\x1b[97;5u").as_deref(), Some("Ctrl+A"));
        assert_eq!(decoded("\x1b[13;2u").as_deref(), Some("Shift+Enter"));
    }

    #[test]
    fn alt_prefix_and_control_characters() {
        assert_eq!(decoded("\x1bb").as_deref(), Some("Alt+B"));
        assert_eq!(decoded("\x1bB").as_deref(), Some("Alt+Shift+B"));
        assert_eq!(decoded("\x1b\x7f").as_deref(), Some("Alt+Backspace"));
        assert_eq!(decoded("\x1b\x1b[A").as_deref(), Some("Alt+Up"));
        assert_eq!(decoded("\x12").as_deref(), Some("Ctrl+R"));
        assert_eq!(decoded("\x1b").as_deref(), Some("Esc"));
    }

    #[test]
    fn several_key_presses() {
        let keys = |sequence: &str| {
            decode_keys(sequence).map(|keys| {
                keys.iter()
                    .map(Chord::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
        };
        assert_eq!(keys("\x18\x05").as_deref(), Some("Ctrl+X, Ctrl+E"));
        assert_eq!(keys("\x18e").as_deref(), Some("Ctrl+X, E"));
        assert_eq!(keys("\x1b[1;5C").as_deref(), Some("Ctrl+Right"));
        assert_eq!(keys("\x1b\x1b[D").as_deref(), Some("Alt+Left"));
        assert_eq!(keys("\x1b\x1b").as_deref(), Some("Alt+Esc"));
        assert_eq!(keys("\x1bOH").as_deref(), Some("Home"));
        assert_eq!(keys("\x1b\x07").as_deref(), Some("Ctrl+Alt+G"));
        assert_eq!(keys("\x18\x1b[A").as_deref(), Some("Ctrl+X, Up"));
        assert_eq!(keys("\x1b["), None);
        assert_eq!(keys("\x1b[200~"), None);
    }

    #[test]
    fn control_characters() {
        assert_eq!(control_char('a'), Some('\x01'));
        assert_eq!(control_char('X'), Some('\x18'));
        assert_eq!(control_char('['), Some('\x1b'));
        assert_eq!(control_char('@'), Some('\0'));
        assert_eq!(control_char('?'), Some('\x7f'));
        assert_eq!(control_char('1'), None);
    }

    #[test]
    fn unknown_sequences() {
        assert_eq!(decoded("\x1b[5D"), None);
        assert_eq!(decoded("\x1b\u{f50e}ep"), None);
        assert_eq!(decoded("\x1b[1;99A"), None);
        assert_eq!(decoded("\x1b[?1049h"), None);
        assert_eq!(decoded("\x1b[200~"), None);
    }
}
