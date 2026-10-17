//! micro, read from `bindings.json` on top of micro's built-in defaults.
//!
//! `bindings.json` is JSON5. A top-level `"key": "action"` binds in the editor
//! buffer; `"command": {…}` and `"terminal": {…}` objects bind in micro's command
//! bar and terminal pane, which keydibs treats as modes. Binding to `None`
//! removes a default.
//!
//! Key names follow micro's own parser (`findSingleEvent` in micro's
//! `internal/action/bindings.go`): modifiers are `Ctrl`, `Alt` and `Shift`, with
//! optional dashes (`Ctrl-s`, `CtrlShiftUp`), an uppercase letter means Shift
//! (`Alt-F` is not `Alt-f`), and `<Ctrl-q><Ctrl-q>` is a sequence.
//!
//! Raw escape sequences (`"\u001b[1;5D"`) are decoded to the key they stand for.
//! Mouse bindings are skipped.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::Context;

use super::{Adapter, config_home, escape, on_path, shorten, version_note};
use crate::binding::{Binding, Grabs, Layer, Source};
use crate::chord::{Chord, Key, Mods};

const TOOL: &str = "micro";

/// The micro version the defaults below come from.
const DEFAULTS_VERSION: &str = "2.0.15";

/// micro 2.0.15's Linux defaults, from `internal/action/defaults_other.go`.
const BUFFER_DEFAULTS: &[(&str, &str)] = &[
    ("Up", "CursorUp"),
    ("Down", "CursorDown"),
    ("Right", "CursorRight"),
    ("Left", "CursorLeft"),
    ("ShiftUp", "SelectUp"),
    ("ShiftDown", "SelectDown"),
    ("ShiftLeft", "SelectLeft"),
    ("ShiftRight", "SelectRight"),
    ("CtrlLeft", "WordLeft"),
    ("CtrlRight", "WordRight"),
    ("AltUp", "MoveLinesUp"),
    ("AltDown", "MoveLinesDown"),
    ("CtrlShiftRight", "SelectWordRight"),
    ("CtrlShiftLeft", "SelectWordLeft"),
    ("AltLeft", "StartOfTextToggle"),
    ("AltRight", "EndOfLine"),
    ("AltShiftLeft", "SelectToStartOfTextToggle"),
    ("ShiftHome", "SelectToStartOfTextToggle"),
    ("AltShiftRight", "SelectToEndOfLine"),
    ("ShiftEnd", "SelectToEndOfLine"),
    ("CtrlUp", "CursorStart"),
    ("CtrlDown", "CursorEnd"),
    ("CtrlShiftUp", "SelectToStart"),
    ("CtrlShiftDown", "SelectToEnd"),
    ("Alt-{", "ParagraphPrevious"),
    ("Alt-}", "ParagraphNext"),
    ("Enter", "InsertNewline"),
    ("CtrlH", "Backspace"),
    ("Backspace", "Backspace"),
    ("OldBackspace", "Backspace"),
    ("Alt-CtrlH", "DeleteWordLeft"),
    ("Alt-Backspace", "DeleteWordLeft"),
    ("Tab", "Autocomplete|IndentSelection|InsertTab"),
    (
        "Backtab",
        "CycleAutocompleteBack|OutdentSelection|OutdentLine",
    ),
    ("Ctrl-o", "OpenFile"),
    ("Ctrl-s", "Save"),
    ("Ctrl-f", "Find"),
    ("Alt-F", "FindLiteral"),
    ("Ctrl-n", "FindNext"),
    ("Ctrl-p", "FindPrevious"),
    ("Alt-[", "DiffPrevious|CursorStart"),
    ("Alt-]", "DiffNext|CursorEnd"),
    ("Ctrl-z", "Undo"),
    ("Ctrl-y", "Redo"),
    ("Ctrl-c", "Copy|CopyLine"),
    ("Ctrl-x", "Cut|CutLine"),
    ("Ctrl-k", "CutLine"),
    ("Ctrl-d", "Duplicate|DuplicateLine"),
    ("Ctrl-v", "Paste"),
    ("Ctrl-a", "SelectAll"),
    ("Ctrl-t", "AddTab"),
    ("Alt-,", "PreviousTab|LastTab"),
    ("Alt-.", "NextTab|FirstTab"),
    ("Home", "StartOfTextToggle"),
    ("End", "EndOfLine"),
    ("CtrlHome", "CursorStart"),
    ("CtrlEnd", "CursorEnd"),
    ("PageUp", "CursorPageUp"),
    ("PageDown", "CursorPageDown"),
    ("CtrlPageUp", "PreviousTab|LastTab"),
    ("CtrlPageDown", "NextTab|FirstTab"),
    ("ShiftPageUp", "SelectPageUp"),
    ("ShiftPageDown", "SelectPageDown"),
    ("Ctrl-g", "ToggleHelp"),
    ("Alt-g", "ToggleKeyMenu"),
    ("Ctrl-r", "ToggleRuler"),
    ("Ctrl-l", "command-edit:goto "),
    ("Delete", "Delete"),
    ("Ctrl-b", "ShellMode"),
    ("Ctrl-q", "Quit"),
    ("Ctrl-e", "CommandMode"),
    ("Ctrl-w", "NextSplit|FirstSplit"),
    ("Ctrl-u", "ToggleMacro"),
    ("Ctrl-j", "PlayMacro"),
    ("Insert", "ToggleOverwriteMode"),
    ("Alt-f", "WordRight"),
    ("Alt-b", "WordLeft"),
    ("Alt-a", "StartOfText"),
    ("Alt-e", "EndOfLine"),
    ("F2", "Save"),
    ("F3", "Find"),
    ("F4", "Quit"),
    ("F7", "Find"),
    ("F10", "Quit"),
    (
        "Esc",
        "Escape,Deselect,ClearInfo,RemoveAllMultiCursors,UnhighlightSearch",
    ),
    ("Alt-n", "SpawnMultiCursor"),
    ("Alt-m", "SpawnMultiCursorSelect"),
    ("AltShiftUp", "SpawnMultiCursorUp"),
    ("AltShiftDown", "SpawnMultiCursorDown"),
    ("Alt-p", "RemoveMultiCursor"),
    ("Alt-c", "RemoveAllMultiCursors"),
    ("Alt-x", "SkipMultiCursor"),
];

/// The command bar's defaults (`infodefaults` in micro).
const COMMAND_DEFAULTS: &[(&str, &str)] = &[
    ("Up", "HistoryUp"),
    ("Down", "HistoryDown"),
    ("Right", "CursorRight"),
    ("Left", "CursorLeft"),
    ("ShiftUp", "SelectUp"),
    ("ShiftDown", "SelectDown"),
    ("ShiftLeft", "SelectLeft"),
    ("ShiftRight", "SelectRight"),
    ("AltLeft", "StartOfTextToggle"),
    ("AltRight", "EndOfLine"),
    ("AltUp", "CursorStart"),
    ("AltDown", "CursorEnd"),
    ("AltShiftRight", "SelectWordRight"),
    ("AltShiftLeft", "SelectWordLeft"),
    ("CtrlLeft", "WordLeft"),
    ("CtrlRight", "WordRight"),
    ("CtrlShiftLeft", "SelectToStartOfTextToggle"),
    ("ShiftHome", "SelectToStartOfTextToggle"),
    ("CtrlShiftRight", "SelectToEndOfLine"),
    ("ShiftEnd", "SelectToEndOfLine"),
    ("CtrlUp", "CursorStart"),
    ("CtrlDown", "CursorEnd"),
    ("CtrlShiftUp", "SelectToStart"),
    ("CtrlShiftDown", "SelectToEnd"),
    ("Enter", "ExecuteCommand"),
    ("CtrlH", "Backspace"),
    ("Backspace", "Backspace"),
    ("OldBackspace", "Backspace"),
    ("Alt-CtrlH", "DeleteWordLeft"),
    ("Alt-Backspace", "DeleteWordLeft"),
    ("Tab", "CommandComplete"),
    ("Backtab", "CycleAutocompleteBack"),
    ("Ctrl-z", "Undo"),
    ("Ctrl-y", "Redo"),
    ("Ctrl-c", "Copy|CopyLine"),
    ("Ctrl-x", "Cut|CutLine"),
    ("Ctrl-k", "CutLine"),
    ("Ctrl-v", "Paste"),
    ("Home", "StartOfTextToggle"),
    ("End", "EndOfLine"),
    ("CtrlHome", "CursorStart"),
    ("CtrlEnd", "CursorEnd"),
    ("Delete", "Delete"),
    ("Ctrl-q", "AbortCommand"),
    ("Ctrl-e", "EndOfLine"),
    ("Ctrl-a", "StartOfLine"),
    ("Ctrl-w", "DeleteWordLeft"),
    ("Insert", "ToggleOverwriteMode"),
    ("Ctrl-b", "WordLeft"),
    ("Ctrl-f", "WordRight"),
    ("Ctrl-d", "DeleteWordLeft"),
    ("Ctrl-m", "ExecuteCommand"),
    ("Ctrl-n", "HistoryDown"),
    ("Ctrl-p", "HistoryUp"),
    ("Ctrl-u", "SelectToStart"),
    ("Alt-f", "WordRight"),
    ("Alt-b", "WordLeft"),
    ("Alt-a", "StartOfText"),
    ("Alt-e", "EndOfLine"),
    ("F10", "AbortCommand"),
    ("Esc", "AbortCommand"),
];

/// The terminal pane's defaults (`termdefaults` in micro).
const TERMINAL_DEFAULTS: &[(&str, &str)] = &[
    ("<Ctrl-q><Ctrl-q>", "Exit"),
    ("<Ctrl-e><Ctrl-e>", "CommandMode"),
    ("<Ctrl-w><Ctrl-w>", "NextSplit|FirstSplit"),
];

/// A pane's default `(key, action)` pairs.
type Defaults = &'static [(&'static str, &'static str)];

/// micro's panes, with the mode keydibs files them under.
const PANES: &[(&str, Option<&str>, Defaults)] = &[
    ("buffer", None, BUFFER_DEFAULTS),
    ("command", Some("command"), COMMAND_DEFAULTS),
    ("terminal", Some("terminal"), TERMINAL_DEFAULTS),
];

pub struct Micro;

impl Adapter for Micro {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn layer(&self) -> Layer {
        Layer::App
    }

    fn where_to_change(&self) -> &'static str {
        "~/.config/micro/bindings.json"
    }

    fn notes(&self) -> Vec<String> {
        version_note("micro", TOOL, &["-version"], DEFAULTS_VERSION, version_line)
            .into_iter()
            .collect()
    }

    fn load(&self) -> anyhow::Result<Option<Vec<Binding>>> {
        let path = config_dir().map(|dir| dir.join("bindings.json"));
        let config = match &path {
            Some(path) => match fs::read_to_string(path) {
                Ok(text) => Some(text),
                Err(e) if e.kind() == ErrorKind::NotFound => None,
                Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
            },
            None => None,
        };
        if config.is_none() && !on_path(TOOL) {
            return Ok(None);
        }
        let config = config.as_deref().zip(path.as_deref());
        bindings(config).map(Some)
    }
}

/// Picks `2.0.15` out of `micro -version`, which prints `Version: 2.0.15` among other lines.
fn version_line(output: &str) -> Option<&str> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("Version:"))
}

/// `$MICRO_CONFIG_HOME`, else `micro` in the XDG config directory.
fn config_dir() -> Option<PathBuf> {
    std::env::var_os("MICRO_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| config_home().map(|dir| dir.join("micro")))
}

/// One binding before it's turned into a [`Binding`].
struct Entry {
    pane: &'static str,
    keys: Vec<Chord>,
    action: String,
    source: Source,
}

/// micro's effective bindings: the defaults with `config` (the text and path of
/// `bindings.json`) applied on top.
pub fn bindings(config: Option<(&str, &Path)>) -> anyhow::Result<Vec<Binding>> {
    let mut entries: Vec<Entry> = Vec::new();
    for &(pane, _, defaults) in PANES {
        for &(key, action) in defaults {
            if let Some(keys) = parse_keys(key) {
                entries.push(Entry {
                    pane,
                    keys,
                    action: action.to_string(),
                    source: Source::Default,
                });
            }
        }
    }

    if let Some((text, path)) = config {
        for user in
            parse_config(text, path).with_context(|| format!("parsing {}", path.display()))?
        {
            entries.retain(|e| !(e.pane == user.pane && e.keys == user.keys));
            entries.push(user);
        }
    }
    entries.retain(|e| e.action != "None");

    Ok(entries.into_iter().map(to_binding).collect())
}

fn to_binding(entry: Entry) -> Binding {
    let mode = PANES
        .iter()
        .find(|(pane, _, _)| *pane == entry.pane)
        .and_then(|(_, mode, _)| *mode)
        .map(str::to_string);
    let mut keys = entry.keys.into_iter();
    Binding {
        chord: keys
            .next()
            .expect("parse_keys never returns an empty sequence"),
        action: shorten(&entry.action),
        tool: TOOL,
        layer: Layer::App,
        grabs: Grabs::Always,
        mode,
        then: keys.collect(),
        source: entry.source,
    }
}

/// Reads the user's `bindings.json`.
fn parse_config(text: &str, path: &Path) -> anyhow::Result<Vec<Entry>> {
    let parsed: serde_json::Map<String, serde_json::Value> = json5::from_str(text)?;
    let mut entries = Vec::new();
    for (key, value) in &parsed {
        match value {
            serde_json::Value::String(action) => {
                push_user_entry(&mut entries, text, path, "buffer", None, key, action)
            }
            serde_json::Value::Object(pane_bindings) => {
                // Unknown pane names are ignored, as micro does (after a warning).
                let Some(&(pane, _, _)) = PANES.iter().find(|(pane, _, _)| pane == key) else {
                    continue;
                };
                for (key_in_pane, action) in pane_bindings {
                    if let Some(action) = action.as_str() {
                        push_user_entry(
                            &mut entries,
                            text,
                            path,
                            pane,
                            Some(key),
                            key_in_pane,
                            action,
                        );
                    }
                }
            }
            _ => {}
        }
    }
    Ok(entries)
}

fn push_user_entry(
    entries: &mut Vec<Entry>,
    text: &str,
    path: &Path,
    pane: &'static str,
    pane_key: Option<&str>,
    key: &str,
    action: &str,
) {
    let Some(keys) = parse_keys(key) else { return };
    entries.push(Entry {
        pane,
        keys,
        action: action.to_string(),
        source: Source::Config {
            path: path.to_path_buf(),
            line: line_of(text, pane_key, key),
        },
    });
}

/// The JSON5 parser doesn't report positions, so find the line by searching for
/// the quoted key (after its pane's key, for pane bindings). Good enough to point
/// a person at the right place.
fn line_of(text: &str, pane_key: Option<&str>, key: &str) -> Option<usize> {
    let start = match pane_key {
        Some(pane) => text.find(&format!("\"{pane}\""))?,
        None => 0,
    };
    let offset = start + text[start..].find(&format!("\"{key}\""))?;
    Some(text[..offset].matches('\n').count() + 1)
}

/// Parses a key, or a `<…><…>` sequence of keys.
fn parse_keys(name: &str) -> Option<Vec<Chord>> {
    if name.starts_with('<') && name.ends_with('>') && name.len() > 2 {
        name[1..name.len() - 1].split("><").map(parse_key).collect()
    } else {
        parse_key(name).map(|chord| vec![chord])
    }
}

/// One key, the way micro's `findSingleEvent` reads it.
fn parse_key(name: &str) -> Option<Chord> {
    let mut mods = Mods::empty();
    let mut rest = name;
    loop {
        if let Some(after) = rest.strip_prefix('-').filter(|_| rest != "-") {
            rest = after;
        } else if let Some(after) = rest.strip_prefix("Ctrl").filter(|_| rest != "CtrlH") {
            // `CtrlH` is a key name of its own: what the terminal sends for Ctrl+H.
            mods |= Mods::CTRL;
            rest = after;
        } else if let Some(after) = rest.strip_prefix("Alt") {
            mods |= Mods::ALT;
            rest = after;
        } else if let Some(after) = rest.strip_prefix("Shift") {
            mods |= Mods::SHIFT;
            rest = after;
        } else {
            break;
        }
    }
    // A raw escape sequence, which micro matches as is, whatever came before it.
    if rest.starts_with('\x1b') {
        return escape::decode(rest);
    }
    if rest.is_empty() || rest.starts_with("Mouse") {
        return None;
    }

    if mods.contains(Mods::CTRL)
        && let Some(key) = ctrl_key(rest)
    {
        return Some(Chord {
            mods,
            key: Key::parse(key)?,
        });
    }

    let key = match rest {
        "CtrlH" | "OldBackspace" => {
            mods |= Mods::CTRL;
            "h"
        }
        "Backtab" => {
            mods |= Mods::SHIFT;
            "tab"
        }
        single if single.chars().count() == 1 => {
            let c = single.chars().next()?;
            if c.is_ascii_uppercase() {
                mods |= Mods::SHIFT;
            }
            return Some(Chord {
                mods,
                key: Key::parse(&c.to_ascii_lowercase().to_string())?,
            });
        }
        named if NAMED_KEYS.contains(&named) || is_function_key(named) => named,
        _ => return None,
    };
    Some(Chord {
        mods,
        key: Key::parse(key)?,
    })
}

/// micro's `Ctrl…` key names (`CtrlA`, `CtrlSpace`, `CtrlLeftSq`…) for a key
/// written after a `Ctrl` modifier, as the key they mean.
fn ctrl_key(rest: &str) -> Option<&str> {
    let mut chars = rest.chars();
    let first = chars.next()?;
    if first.is_ascii_alphabetic() && chars.as_str().is_empty() {
        return Some(rest);
    }
    match rest {
        "Space" | "space" => Some("space"),
        "LeftSq" | "leftSq" => Some("["),
        "Backslash" | "backslash" => Some("\\"),
        "RightSq" | "rightSq" => Some("]"),
        "Carat" | "carat" => Some("^"),
        "Underscore" | "underscore" => Some("_"),
        _ => None,
    }
}

/// micro's named keys (`keyEvents`), apart from the `Ctrl…` ones and F-keys.
const NAMED_KEYS: &[&str] = &[
    "Up",
    "Down",
    "Right",
    "Left",
    "UpLeft",
    "UpRight",
    "DownLeft",
    "DownRight",
    "Center",
    "PageUp",
    "PageDown",
    "Home",
    "End",
    "Insert",
    "Delete",
    "Help",
    "Exit",
    "Clear",
    "Cancel",
    "Print",
    "Pause",
    "Tab",
    "Esc",
    "Escape",
    "Enter",
    "Backspace",
];

fn is_function_key(name: &str) -> bool {
    name.strip_prefix('F')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=64).contains(&n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(s: &str) -> Chord {
        s.parse().unwrap()
    }

    fn defaults() -> Vec<Binding> {
        bindings(None).unwrap()
    }

    fn find<'a>(bs: &'a [Binding], keys: &str, mode: Option<&str>) -> &'a Binding {
        bs.iter()
            .find(|b| b.keys() == keys && b.mode.as_deref() == mode)
            .unwrap_or_else(|| panic!("no {keys} binding in {mode:?}"))
    }

    #[test]
    fn key_names_follow_micros_parser() {
        assert_eq!(parse_key("Ctrl-s"), Some(chord("ctrl+s")));
        assert_eq!(parse_key("CtrlShiftUp"), Some(chord("ctrl+shift+up")));
        assert_eq!(parse_key("AltShiftLeft"), Some(chord("alt+shift+left")));
        assert_eq!(parse_key("Alt-{"), Some(chord("alt+{")));
        assert_eq!(parse_key("Alt-CtrlH"), Some(chord("ctrl+alt+h")));
        assert_eq!(parse_key("OldBackspace"), Some(chord("ctrl+h")));
        assert_eq!(parse_key("Backtab"), Some(chord("shift+tab")));
        assert_eq!(parse_key("CtrlPageUp"), Some(chord("ctrl+pageup")));
        assert_eq!(parse_key("Ctrl-Space"), Some(chord("ctrl+space")));
        assert_eq!(parse_key("F10"), Some(chord("f10")));
        assert_eq!(parse_key("Ctrl-MouseLeft"), None);
        assert_eq!(parse_key("\u{1b}[1;5D"), Some(chord("ctrl+left")));
        assert_eq!(parse_key("\u{1b}[weird"), None);
        assert_eq!(parse_key("Ctrl-Nonsense"), None);
    }

    #[test]
    fn uppercase_letters_mean_shift() {
        assert_eq!(parse_key("Alt-F"), Some(chord("alt+shift+f")));
        let bs = defaults();
        assert_eq!(find(&bs, "Alt+Shift+F", None).action, "FindLiteral");
        assert_eq!(find(&bs, "Alt+F", None).action, "WordRight");
    }

    #[test]
    fn defaults_cover_every_pane() {
        let bs = defaults();
        assert_eq!(find(&bs, "Ctrl+S", None).action, "Save");
        assert_eq!(find(&bs, "Ctrl+Q", Some("command")).action, "AbortCommand");
        let exit = find(&bs, "Ctrl+Q, Ctrl+Q", Some("terminal"));
        assert_eq!(exit.action, "Exit");
        assert_eq!(exit.chord, chord("ctrl+q"));
        assert!(bs.iter().all(|b| b.source == Source::Default));
    }

    #[test]
    fn no_false_duplicates_in_the_defaults() {
        assert!(crate::conflict::find(&defaults()).is_empty());
    }

    #[test]
    fn config_overrides_unbinds_and_adds() {
        let config = r#"{
    // micro allows comments and trailing commas
    "Ctrl-s": "SaveAll",
    "Ctrl-q": "None",
    "Alt-/": "lua:comment.comment",
    "command": {
        "Ctrl-s": "AbortCommand",
    },
}"#;
        let bs = bindings(Some((config, Path::new("bindings.json")))).unwrap();
        let save = find(&bs, "Ctrl+S", None);
        assert_eq!(save.action, "SaveAll");
        assert_eq!(
            save.source,
            Source::Config {
                path: "bindings.json".into(),
                line: Some(3)
            }
        );
        assert!(!bs.iter().any(|b| b.keys() == "Ctrl+Q" && b.mode.is_none()));
        assert_eq!(find(&bs, "Alt+/", None).action, "lua:comment.comment");
        let command = find(&bs, "Ctrl+S", Some("command"));
        assert_eq!(command.action, "AbortCommand");
        assert_eq!(
            command.source,
            Source::Config {
                path: "bindings.json".into(),
                line: Some(7)
            }
        );
    }
}
