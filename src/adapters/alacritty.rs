//! Alacritty, read from `alacritty.toml` on top of Alacritty's built-in defaults.
//!
//! Alacritty runs *every* binding that matches a key press, so several entries
//! for one chord are combined into one binding instead of being reported as
//! duplicates. A binding's `mode` decides when it grabs the key: `~Alt` only
//! outside full-screen apps, `Alt` only inside them, and `Vi` or `Search` only in
//! Alacritty's own modes, where keys don't travel further down anyway.
//!
//! Files pulled in with `general.import` are read too, in the order Alacritty
//! merges them. Not handled yet: bindings given as raw scancodes (`key = 123`).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;
use toml::Spanned;

use super::{Adapter, config_home, on_path, second_word, version_note};
use crate::binding::{Binding, Grabs, Layer, Source};
use crate::chord::{Chord, Key, Mods};

const TOOL: &str = "alacritty";

/// The Alacritty version [`DEFAULTS`] comes from.
const DEFAULTS_VERSION: &str = "0.17";

/// The Linux defaults from `man 5 alacritty-bindings` (Alacritty 0.17), without
/// the macOS-only `Command` bindings and the Windows-only `Alt+Enter`.
const DEFAULTS: &str = r##"
[keyboard]
bindings = [
  { key = "Paste", action = "Paste" },
  { key = "Copy", action = "Copy" },
  { key = "L", mods = "Control", action = "ClearLogNotice" },
  { key = "Paste", mode = "Vi|Search", action = "Paste" },
  { key = "L", mods = "Control", mode = "~Vi|~Search", chars = "\f" },
  { key = "PageUp", mods = "Shift", mode = "~Alt", action = "ScrollPageUp" },
  { key = "PageDown", mods = "Shift", mode = "~Alt", action = "ScrollPageDown" },
  { key = "Home", mods = "Shift", mode = "~Alt", action = "ScrollToTop" },
  { key = "End", mods = "Shift", mode = "~Alt", action = "ScrollToBottom" },
  { key = "Space", mods = "Shift|Control", mode = "~Search", action = "ToggleViMode" },
  { key = "Space", mods = "Shift|Control", mode = "Vi|~Search", action = "ScrollToBottom" },
  { key = "Escape", mode = "Vi|~Search", action = "ClearSelection" },
  { key = "I", mode = "Vi|~Search", action = "ToggleViMode" },
  { key = "I", mode = "Vi|~Search", action = "ScrollToBottom" },
  { key = "C", mods = "Control", mode = "Vi|~Search", action = "ToggleViMode" },
  { key = "Y", mods = "Control", mode = "Vi|~Search", action = "ScrollLineUp" },
  { key = "E", mods = "Control", mode = "Vi|~Search", action = "ScrollLineDown" },
  { key = "G", mode = "Vi|~Search", action = "ScrollToTop" },
  { key = "G", mods = "Shift", mode = "Vi|~Search", action = "ScrollToBottom" },
  { key = "B", mods = "Control", mode = "Vi|~Search", action = "ScrollPageUp" },
  { key = "F", mods = "Control", mode = "Vi|~Search", action = "ScrollPageDown" },
  { key = "U", mods = "Control", mode = "Vi|~Search", action = "ScrollHalfPageUp" },
  { key = "D", mods = "Control", mode = "Vi|~Search", action = "ScrollHalfPageDown" },
  { key = "Y", mode = "Vi|~Search", action = "Copy" },
  { key = "Y", mode = "Vi|~Search", action = "ClearSelection" },
  { key = "Copy", mode = "Vi|~Search", action = "ClearSelection" },
  { key = "Y", mods = "Shift", mode = "Vi|~Search", action = "ToggleNormalSelection" },
  { key = "Y", mods = "Shift", mode = "Vi|~Search", action = "Last" },
  { key = "Y", mods = "Shift", mode = "Vi|~Search", action = "Copy" },
  { key = "Y", mods = "Shift", mode = "Vi|~Search", action = "ClearSelection" },
  { key = "V", mode = "Vi|~Search", action = "ToggleNormalSelection" },
  { key = "V", mods = "Shift", mode = "Vi|~Search", action = "ToggleLineSelection" },
  { key = "V", mods = "Control", mode = "Vi|~Search", action = "ToggleBlockSelection" },
  { key = "V", mods = "Alt", mode = "Vi|~Search", action = "ToggleSemanticSelection" },
  { key = "Enter", mode = "Vi|~Search", action = "Open" },
  { key = "Z", mode = "Vi|~Search", action = "CenterAroundViCursor" },
  { key = "F", mode = "Vi|~Search", action = "InlineSearchForward" },
  { key = "F", mods = "Shift", mode = "Vi|~Search", action = "InlineSearchBackward" },
  { key = "T", mode = "Vi|~Search", action = "InlineSearchForwardShort" },
  { key = "T", mods = "Shift", mode = "Vi|~Search", action = "InlineSearchBackwardShort" },
  { key = ";", mode = "Vi|~Search", action = "InlineSearchNext" },
  { key = ",", mode = "Vi|~Search", action = "InlineSearchPrevious" },
  { key = "*", mods = "Shift", mode = "Vi|~Search", action = "SemanticSearchForward" },
  { key = "#", mods = "Shift", mode = "Vi|~Search", action = "SemanticSearchBackward" },
  { key = "K", mode = "Vi|~Search", action = "Up" },
  { key = "J", mode = "Vi|~Search", action = "Down" },
  { key = "H", mode = "Vi|~Search", action = "Left" },
  { key = "L", mode = "Vi|~Search", action = "Right" },
  { key = "ArrowUp", mode = "Vi|~Search", action = "Up" },
  { key = "ArrowDown", mode = "Vi|~Search", action = "Down" },
  { key = "ArrowLeft", mode = "Vi|~Search", action = "Left" },
  { key = "ArrowRight", mode = "Vi|~Search", action = "Right" },
  { key = "0", mode = "Vi|~Search", action = "First" },
  { key = "$", mods = "Shift", mode = "Vi|~Search", action = "Last" },
  { key = "Home", mode = "Vi|~Search", action = "First" },
  { key = "End", mode = "Vi|~Search", action = "Last" },
  { key = "^", mods = "Shift", mode = "Vi|~Search", action = "FirstOccupied" },
  { key = "H", mods = "Shift", mode = "Vi|~Search", action = "High" },
  { key = "M", mods = "Shift", mode = "Vi|~Search", action = "Middle" },
  { key = "L", mods = "Shift", mode = "Vi|~Search", action = "Low" },
  { key = "B", mode = "Vi|~Search", action = "SemanticLeft" },
  { key = "W", mode = "Vi|~Search", action = "SemanticRight" },
  { key = "E", mode = "Vi|~Search", action = "SemanticRightEnd" },
  { key = "B", mods = "Shift", mode = "Vi|~Search", action = "WordLeft" },
  { key = "W", mods = "Shift", mode = "Vi|~Search", action = "WordRight" },
  { key = "E", mods = "Shift", mode = "Vi|~Search", action = "WordRightEnd" },
  { key = "%", mods = "Shift", mode = "Vi|~Search", action = "Bracket" },
  { key = "{", mods = "Shift", mode = "Vi|~Search", action = "ParagraphUp" },
  { key = "}", mods = "Shift", mode = "Vi|~Search", action = "ParagraphDown" },
  { key = "/", mode = "Vi|~Search", action = "SearchForward" },
  { key = "?", mods = "Shift", mode = "Vi|~Search", action = "SearchBackward" },
  { key = "N", mode = "Vi|~Search", action = "SearchNext" },
  { key = "N", mods = "Shift", mode = "Vi|~Search", action = "SearchPrevious" },
  { key = "Enter", mode = "Search|Vi", action = "SearchConfirm" },
  { key = "Escape", mode = "Search", action = "SearchCancel" },
  { key = "C", mods = "Control", mode = "Search", action = "SearchCancel" },
  { key = "U", mods = "Control", mode = "Search", action = "SearchClear" },
  { key = "W", mods = "Control", mode = "Search", action = "SearchDeleteWord" },
  { key = "P", mods = "Control", mode = "Search", action = "SearchHistoryPrevious" },
  { key = "N", mods = "Control", mode = "Search", action = "SearchHistoryNext" },
  { key = "ArrowUp", mode = "Search", action = "SearchHistoryPrevious" },
  { key = "ArrowDown", mode = "Search", action = "SearchHistoryNext" },
  { key = "Enter", mode = "Search|~Vi", action = "SearchFocusNext" },
  { key = "V", mods = "Control|Shift", mode = "~Vi", action = "Paste" },
  { key = "V", mods = "Control|Shift", mode = "Vi|Search", action = "Paste" },
  { key = "C", mods = "Control|Shift", action = "Copy" },
  { key = "F", mods = "Control|Shift", mode = "~Search", action = "SearchForward" },
  { key = "B", mods = "Control|Shift", mode = "~Search", action = "SearchBackward" },
  { key = "C", mods = "Control|Shift", mode = "Vi|~Search", action = "ClearSelection" },
  { key = "Insert", mods = "Shift", action = "PasteSelection" },
  { key = "0", mods = "Control", action = "ResetFontSize" },
  { key = "=", mods = "Control", action = "IncreaseFontSize" },
  { key = "+", mods = "Control", action = "IncreaseFontSize" },
  { key = "NumpadAdd", mods = "Control", action = "IncreaseFontSize" },
  { key = "-", mods = "Control", action = "DecreaseFontSize" },
  { key = "NumpadSubtract", mods = "Control", action = "DecreaseFontSize" },
]
"##;

pub struct Alacritty;

impl Adapter for Alacritty {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn layer(&self) -> Layer {
        Layer::Terminal
    }

    /// Supported since Alacritty 0.13.
    fn kitty_keyboard(&self) -> bool {
        true
    }

    fn notes(&self) -> Vec<String> {
        version_note(
            "Alacritty",
            TOOL,
            &["--version"],
            DEFAULTS_VERSION,
            second_word,
        )
        .into_iter()
        .collect()
    }

    fn load(&self) -> anyhow::Result<Option<Vec<Binding>>> {
        let Some(path) = config_path() else {
            // No config file, but the defaults still apply if Alacritty is installed.
            return on_path(TOOL).then(|| bindings(&[])).transpose();
        };
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let mut files = Vec::new();
        collect_files(text, path, IMPORT_DEPTH, &mut files);
        let files: Vec<(&str, &Path)> = files
            .iter()
            .map(|(text, path)| (text.as_str(), path.as_path()))
            .collect();
        bindings(&files).map(Some)
    }
}

/// How deep imports may nest, as in Alacritty.
const IMPORT_DEPTH: usize = 5;

/// Adds `text` (read from `path`) to `files`, after the files it imports, the
/// way Alacritty merges them: imports first and in order, each with its own
/// imports before it, so later files' bindings come later. Imports that are
/// missing or unreadable are skipped, as Alacritty does.
fn collect_files(text: String, path: PathBuf, depth: usize, files: &mut Vec<(String, PathBuf)>) {
    if depth > 0 {
        for import in imports(&text, &path) {
            if let Ok(imported) = fs::read_to_string(&import) {
                collect_files(imported, import, depth - 1, files);
            }
        }
    }
    files.push((text, path));
}

/// The files `text` imports, from `general.import` or the older top-level `import`.
fn imports(text: &str, path: &Path) -> Vec<PathBuf> {
    let Ok(table) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    let list = table.get("import").or_else(|| {
        table
            .get("general")
            .and_then(|general| general.get("import"))
    });
    let Some(toml::Value::Array(list)) = list else {
        return Vec::new();
    };
    list.iter()
        .filter_map(toml::Value::as_str)
        .map(|import| resolve_import(path, import))
        .collect()
}

/// `~/` means the home directory; other relative paths start from the importing
/// file's directory.
fn resolve_import(importer: &Path, import: &str) -> PathBuf {
    let mut resolved = PathBuf::from(import);
    if let (Some(rest), Some(home)) = (import.strip_prefix("~/"), std::env::var_os("HOME")) {
        resolved = PathBuf::from(home).join(rest);
    }
    match importer.parent() {
        Some(dir) if resolved.is_relative() => dir.join(resolved),
        _ => resolved,
    }
}

/// The first config file Alacritty itself would load.
fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let candidates = [
        config_home().map(|dir| dir.join("alacritty/alacritty.toml")),
        config_home().map(|dir| dir.join("alacritty.toml")),
        home.map(|home| home.join(".alacritty.toml")),
        Some(PathBuf::from("/etc/alacritty/alacritty.toml")),
    ];
    candidates.into_iter().flatten().find(|path| path.is_file())
}

/// Alacritty's effective key bindings: the defaults, with the config files (their
/// text and path, imports first) applied on top.
pub fn bindings(configs: &[(&str, &Path)]) -> anyhow::Result<Vec<Binding>> {
    let mut entries = parse(DEFAULTS, |_| Source::Default).expect("built-in defaults parse");

    let mut user = Vec::new();
    for (text, path) in configs {
        let file = parse(text, |line| Source::Config {
            path: path.to_path_buf(),
            line: Some(line),
        })
        .with_context(|| format!("parsing {}", path.display()))?;
        user.extend(file);
    }
    // A config binding replaces any default with the same trigger.
    entries.retain(|d| !user.iter().any(|u| u.chord == d.chord && u.mode == d.mode));
    entries.extend(user);
    // `ReceiveChar` means "no binding, let the key through".
    entries.retain(
        |e| !matches!(&e.action, Action::Named(name) if name.eq_ignore_ascii_case("ReceiveChar")),
    );

    Ok(combine(entries))
}

/// One `[[keyboard.bindings]]` entry.
struct Entry {
    chord: Chord,
    /// The `mode` field, normalized so equal modes compare equal.
    mode: String,
    action: Action,
    source: Source,
}

enum Action {
    Named(String),
    Chars(String),
    Command(String),
}

impl Action {
    fn label(&self) -> String {
        match self {
            Action::Named(name) => name.clone(),
            Action::Chars(chars) => format!("send {}", caret_notation(chars)),
            Action::Command(program) => format!("run {program}"),
        }
    }
}

#[derive(Deserialize, Default)]
struct File {
    #[serde(default)]
    keyboard: Keyboard,
}

#[derive(Deserialize, Default)]
struct Keyboard {
    #[serde(default)]
    bindings: Vec<Spanned<RawBinding>>,
}

#[derive(Deserialize)]
struct RawBinding {
    key: toml::Value,
    #[serde(default)]
    mods: String,
    #[serde(default)]
    mode: String,
    action: Option<String>,
    chars: Option<String>,
    command: Option<toml::Value>,
}

fn parse(text: &str, source: impl Fn(usize) -> Source) -> anyhow::Result<Vec<Entry>> {
    let file: File = toml::from_str(text)?;
    let mut entries = Vec::new();
    for spanned in file.keyboard.bindings {
        let line = text[..spanned.span().start].matches('\n').count() + 1;
        let raw = spanned.into_inner();

        // Skips what we can't model yet (scancodes, unknown modifiers) rather than guessing.
        let Some(key) = raw.key.as_str().and_then(parse_key) else {
            continue;
        };
        let Some(mods) = parse_mods(&raw.mods) else {
            continue;
        };
        let action = if let Some(chars) = raw.chars {
            Action::Chars(chars)
        } else if let Some(command) = raw.command {
            Action::Command(program(&command))
        } else if let Some(name) = raw.action {
            Action::Named(name)
        } else {
            continue;
        };

        entries.push(Entry {
            chord: Chord { mods, key },
            mode: normalize_mode(&raw.mode),
            action,
            source: source(line),
        });
    }
    Ok(entries)
}

fn parse_key(name: &str) -> Option<Key> {
    // Names from before Alacritty 0.13 that configs still carry around.
    let name = match name {
        "Back" => "Backspace",
        legacy if legacy.len() == 4 && legacy.starts_with("Key") => &legacy[3..],
        other => other,
    };
    Key::parse(name)
}

fn parse_mods(mods: &str) -> Option<Mods> {
    let mut parsed = Mods::empty();
    for part in mods.split('|').map(str::trim) {
        if part.is_empty() || part.eq_ignore_ascii_case("none") {
            continue;
        }
        parsed |= Mods::parse_one(part)?;
    }
    Some(parsed)
}

fn normalize_mode(mode: &str) -> String {
    let mut parts: Vec<String> = mode
        .split('|')
        .map(|part| part.trim().to_lowercase())
        .filter(|part| !part.is_empty())
        .collect();
    parts.sort();
    parts.join("|")
}

/// `command = "foo"` or `command = { program = "foo", args = [...] }`.
fn program(command: &toml::Value) -> String {
    command
        .as_str()
        .or_else(|| command.get("program").and_then(toml::Value::as_str))
        .unwrap_or("a command")
        .to_string()
}

/// When a binding with this (normalized) `mode` fires, as far as the layers below care.
#[derive(Clone, PartialEq, Eq)]
enum When {
    Screen(Grabs),
    /// Only in Alacritty's own vi or search mode.
    OwnMode(String),
}

fn when(mode: &str) -> When {
    let mut own = Vec::new();
    let mut grabs = Grabs::Always;
    for part in mode.split('|').filter(|p| !p.is_empty()) {
        match part {
            "vi" | "search" => own.push(part),
            "alt" => grabs = Grabs::AltScreen,
            "~alt" => grabs = Grabs::NormalScreen,
            // `~vi`, `~search` are the normal state; `appcursor`/`appkeypad`
            // change what a key sends, not whether it's grabbed.
            _ => {}
        }
    }
    if own.is_empty() {
        When::Screen(grabs)
    } else {
        When::OwnMode(own.join("+"))
    }
}

/// Merges entries that fire together (same chord, same condition) into one binding.
fn combine(entries: Vec<Entry>) -> Vec<Binding> {
    let mut groups: Vec<(Chord, When, Vec<Entry>)> = Vec::new();
    for entry in entries {
        let when = when(&entry.mode);
        match groups
            .iter_mut()
            .find(|(chord, w, _)| *chord == entry.chord && *w == when)
        {
            Some((_, _, group)) => group.push(entry),
            None => groups.push((entry.chord.clone(), when, vec![entry])),
        }
    }

    groups
        .into_iter()
        .map(|(chord, when, group)| {
            let mut labels: Vec<String> = Vec::new();
            for entry in &group {
                let label = entry.action.label();
                if !labels.contains(&label) {
                    labels.push(label);
                }
            }
            let action = labels.join(" + ");

            let passes_key_on = group
                .iter()
                .any(|e| matches!(&e.action, Action::Chars(c) if is_plain_encoding(&chord, c)));
            let (grabs, mode) = match when {
                When::OwnMode(mode) => (Grabs::Never, Some(mode)),
                When::Screen(_) if passes_key_on => (Grabs::Never, None),
                When::Screen(grabs) => (grabs, None),
            };

            let source = group
                .iter()
                .map(|e| &e.source)
                .find(|s| matches!(s, Source::Config { .. }))
                .unwrap_or(&Source::Default)
                .clone();

            Binding {
                chord,
                action,
                tool: TOOL,
                layer: Layer::Terminal,
                grabs,
                mode,
                then: Vec::new(),
                source,
            }
        })
        .collect()
}

/// Whether `chars` is exactly what the terminal would send for `chord` anyway,
/// like `Ctrl+L` sending form feed. Such a binding acts and then passes the key on.
fn is_plain_encoding(chord: &Chord, chars: &str) -> bool {
    let mut key = chord.key.as_str().chars();
    match (key.next(), key.next()) {
        (Some(letter), None) if chord.mods == Mods::CTRL && letter.is_ascii_alphabetic() => {
            let control = (letter.to_ascii_uppercase() as u8 & 0x1f) as char;
            chars.chars().eq([control])
        }
        _ => false,
    }
}

/// Shows control characters the way terminals document them: `^L`, `^[`.
fn caret_notation(chars: &str) -> String {
    chars
        .chars()
        .map(|c| match c as u32 {
            0..=0x1f => format!("^{}", (c as u8 + 0x40) as char),
            0x7f => "^?".to_string(),
            _ => c.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed copy of the alacritty.toml CachyOS ships.
    const CACHYOS: &str = r#"[window]
opacity = 0.8

[keyboard]
bindings = [
{ key = "L", mods = "Control", action = "ClearLogNotice" },
{ key = "L", mods = "Control", mode = "~Vi", chars = "\f" },
{ key = "PageUp", mods = "Shift", mode = "~Alt", action = "ScrollPageUp" },
{ key = "C", mods = "Control|Shift", action = "Copy" },
{ key = "C", mods = "Control|Shift", mode = "Vi", action = "ClearSelection" },
{ key = "Key0", mods = "Control", action = "ResetFontSize" },
{ key = "Insert", mods = "Shift", action = "ReceiveChar" },
{ key = "T", mods = "Control|Shift", command = { program = "konsole", args = [] } },
{ key = 38, mods = "Control", action = "Copy" },
]
"#;

    fn resolved() -> Vec<Binding> {
        bindings(&[(CACHYOS, Path::new("alacritty.toml"))]).unwrap()
    }

    fn find<'a>(bindings: &'a [Binding], chord: &str, grabs: Grabs) -> &'a Binding {
        let chord: Chord = chord.parse().unwrap();
        bindings
            .iter()
            .find(|b| b.chord == chord && b.grabs == grabs)
            .unwrap_or_else(|| panic!("no {chord} binding that {grabs}"))
    }

    fn config_line(line: usize) -> Source {
        Source::Config {
            path: "alacritty.toml".into(),
            line: Some(line),
        }
    }

    #[test]
    fn defaults_parse_completely() {
        let entries = parse(DEFAULTS, |_| Source::Default).unwrap();
        assert_eq!(entries.len(), DEFAULTS.matches("{ key =").count());
    }

    #[test]
    fn config_overrides_default_with_same_trigger() {
        let bs = resolved();
        let copy = find(&bs, "Ctrl+Shift+C", Grabs::Always);
        assert_eq!(copy.action, "Copy");
        assert_eq!(copy.source, config_line(9));
    }

    #[test]
    fn untouched_defaults_remain() {
        let bs = resolved();
        let paste = find(&bs, "Ctrl+Shift+V", Grabs::Always);
        assert_eq!(paste.action, "Paste");
        assert_eq!(paste.source, Source::Default);
    }

    #[test]
    fn ctrl_l_acts_then_passes_the_key_on() {
        let bs = resolved();
        // The default `send ^L` (mode `~Vi|~Search`) and the config's (`~Vi`)
        // both fire in the normal state, so they merge with ClearLogNotice.
        let ctrl_l = find(&bs, "Ctrl+L", Grabs::Never);
        assert_eq!(ctrl_l.action, "send ^L + ClearLogNotice");
        assert_eq!(ctrl_l.source, config_line(6));
    }

    #[test]
    fn modes_map_to_screens() {
        let bs = resolved();
        assert_eq!(
            find(&bs, "Shift+PageUp", Grabs::NormalScreen).action,
            "ScrollPageUp"
        );
        assert_eq!(
            find(&bs, "Ctrl+Shift+Space", Grabs::Never).describe(),
            "ScrollToBottom (vi mode)"
        );
    }

    #[test]
    fn legacy_key_names_receive_char_and_commands() {
        let bs = resolved();
        assert_eq!(find(&bs, "Ctrl+0", Grabs::Always).source, config_line(11));
        assert!(
            !bs.iter()
                .any(|b| b.chord == "Shift+Insert".parse().unwrap())
        );
        assert_eq!(
            find(&bs, "Ctrl+Shift+T", Grabs::Always).action,
            "run konsole"
        );
    }

    #[test]
    fn imports_come_first_and_nest() {
        let dir = std::env::temp_dir().join(format!("keydibs-alacritty-{}", std::process::id()));
        fs::create_dir_all(dir.join("themes")).unwrap();
        let main = dir.join("alacritty.toml");
        fs::write(
            &main,
            r#"[general]
import = ["themes/keys.toml", "missing.toml"]

[keyboard]
bindings = [{ key = "N", mods = "Control|Shift", action = "SpawnNewInstance" }]
"#,
        )
        .unwrap();
        fs::write(
            dir.join("themes/keys.toml"),
            r#"import = ["nested.toml"]
[keyboard]
bindings = [{ key = "N", mods = "Control|Shift", action = "CreateNewWindow" }]
"#,
        )
        .unwrap();
        fs::write(
            dir.join("themes/nested.toml"),
            r#"[keyboard]
bindings = [{ key = "J", mods = "Control|Shift", action = "ScrollLineDown" }]
"#,
        )
        .unwrap();

        let mut files = Vec::new();
        let text = fs::read_to_string(&main).unwrap();
        collect_files(text, main.clone(), IMPORT_DEPTH, &mut files);
        let names: Vec<_> = files
            .iter()
            .map(|(_, path)| path.file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["nested.toml", "keys.toml", "alacritty.toml"]);

        let files: Vec<(&str, &Path)> = files
            .iter()
            .map(|(text, path)| (text.as_str(), path.as_path()))
            .collect();
        let bs = bindings(&files).unwrap();
        assert_eq!(
            find(&bs, "Ctrl+Shift+J", Grabs::Always).action,
            "ScrollLineDown"
        );
        // Both fire, the imported one first, since Alacritty runs every match.
        assert_eq!(
            find(&bs, "Ctrl+Shift+N", Grabs::Always).action,
            "CreateNewWindow + SpawnNewInstance"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn caret_notation_for_control_chars() {
        assert_eq!(caret_notation("\x0c"), "^L");
        assert_eq!(caret_notation("\x1b[A"), "^[[A");
    }
}
