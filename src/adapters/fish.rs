//! fish, read by asking fish itself for its key bindings.
//!
//! fish builds most of its keymap in code when an interactive session starts
//! (`fish_default_key_bindings`, plugins in `conf.d`, `fish_user_key_bindings`),
//! so reading config files would miss most of it. Instead keydibs runs the same
//! setup in a throwaway `fish -i` and reads what `bind` prints. That runs your
//! fish config, exactly as opening a new terminal does.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, bail};

use super::{Adapter, on_path, shorten};
use crate::binding::{Binding, Grabs, Layer, Source};
use crate::chord::{Chord, Key, Mods};

const TOOL: &str = "fish";

/// Runs the interactive setup without the greeting (which may launch something
/// like fastfetch), then lists every binding in every mode.
const SCRIPT: &str = "function fish_greeting; end; __fish_config_interactive >/dev/null 2>&1; bind";

pub struct Fish;

impl Adapter for Fish {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn layer(&self) -> Layer {
        Layer::Shell
    }

    fn load(&self) -> anyhow::Result<Option<Vec<Binding>>> {
        if !on_path(TOOL) {
            return Ok(None);
        }
        let output = Command::new(TOOL)
            .args(["-i", "-c", SCRIPT])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .context("running fish")?;
        if !output.status.success() {
            bail!("`fish -c bind` failed ({})", output.status);
        }
        Ok(Some(parse(&String::from_utf8_lossy(&output.stdout))))
    }
}

/// One line of `bind` output.
struct Entry {
    key: String,
    mode: String,
    preset: bool,
    commands: Vec<String>,
    /// From the `# Defined in <file>:` comment above it.
    defined_in: Option<String>,
}

/// Turns the output of `bind` into bindings.
pub fn parse(output: &str) -> Vec<Binding> {
    let mut entries = Vec::new();
    let mut defined_in = None;
    for line in output.lines() {
        let line = strip_ansi(line);
        let line = line.trim();
        if let Some(origin) = line.strip_prefix("# Defined in ") {
            defined_in = Some(origin.trim_end_matches(':').to_string());
        } else if let Some(entry) = parse_entry(line, &defined_in) {
            entries.push(entry);
        }
    }

    // A user binding replaces the preset for the same key in the same mode;
    // `bind` lists both.
    let overridden: Vec<(String, String)> = entries
        .iter()
        .filter(|e| !e.preset)
        .map(|e| (e.mode.clone(), e.key.clone()))
        .collect();
    entries.retain(|e| !e.preset || !overridden.contains(&(e.mode.clone(), e.key.clone())));

    // With vi bindings, fish's "default" mode is vi's normal mode.
    let vi = entries.iter().any(|e| e.mode == "insert");

    entries
        .into_iter()
        .filter_map(|entry| {
            let mut chords = parse_keys(&entry.key)?.into_iter();
            let chord = chords.next()?;
            let mode = match entry.mode.as_str() {
                "default" if vi => Some("normal".to_string()),
                "default" => None,
                other => Some(other.to_string()),
            };
            let source = if entry.preset {
                Source::Default
            } else {
                match entry.defined_in {
                    Some(path) if path.starts_with('/') => Source::Config {
                        path: PathBuf::from(path),
                        line: None,
                    },
                    _ => Source::Runtime,
                }
            };
            Some(Binding {
                chord,
                action: shorten(&entry.commands.join(" ")),
                tool: TOOL,
                layer: Layer::Shell,
                grabs: Grabs::Always,
                mode,
                then: chords.collect(),
                source,
            })
        })
        .collect()
}

fn parse_entry(line: &str, defined_in: &Option<String>) -> Option<Entry> {
    let mut words = words(line).into_iter();
    if words.next()? != "bind" {
        return None;
    }
    let mut mode = "default".to_string();
    let mut preset = false;
    let mut key = None;
    while let Some(word) = words.next() {
        match word.as_str() {
            "--preset" => preset = true,
            "--user" | "-s" | "--silent" => {}
            "-M" | "--mode" => mode = words.next()?,
            "-m" | "--sets-mode" => {
                words.next()?;
            }
            flag if flag.starts_with("--mode=") => mode = flag["--mode=".len()..].to_string(),
            flag if flag.starts_with("--sets-mode=") => {}
            _ => {
                key = Some(word);
                break;
            }
        }
    }
    Some(Entry {
        key: key?,
        mode,
        preset,
        commands: words.collect(),
        defined_in: defined_in.clone(),
    })
}

/// Parses a key like `ctrl-alt-h`, `\$` (already unescaped to `$`) or a
/// sequence like `ctrl-x,ctrl-e`. Returns `None` for the self-insert catch-all
/// (`''`) and for raw escape sequences like `\e[1;9A`, which are fallbacks for
/// old terminals that duplicate the named keys.
fn parse_keys(token: &str) -> Option<Vec<Chord>> {
    if token.is_empty() || token.chars().any(|c| c.is_control()) {
        return None;
    }
    let parts: Vec<&str> = token.split(',').collect();
    if parts.len() > 1 && parts.iter().all(|p| !p.is_empty()) {
        parts.into_iter().map(parse_key).collect()
    } else {
        parse_key(token).map(|chord| vec![chord])
    }
}

fn parse_key(name: &str) -> Option<Chord> {
    let mut mods = Mods::empty();
    let mut rest = name;
    'strip: loop {
        for (prefix, flag) in [
            ("ctrl-", Mods::CTRL),
            ("alt-", Mods::ALT),
            ("shift-", Mods::SHIFT),
            ("super-", Mods::SUPER),
        ] {
            // `alt--` is Alt and the minus key, so never strip the last character.
            if rest.len() > prefix.len()
                && let Some(after) = rest.strip_prefix(prefix)
            {
                mods |= flag;
                rest = after;
                continue 'strip;
            }
        }
        break;
    }
    Some(Chord {
        mods,
        key: Key::parse(rest)?,
    })
}

/// Splits a line of `bind` output into words, undoing fish's quotes and escapes.
fn words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '\'' | '"' => {
                in_word = true;
                // Inside single quotes only \' and \\ are escapes; double quotes add \" and \$.
                while let Some(c2) = chars.next() {
                    if c2 == c {
                        break;
                    }
                    let escapable = c2 == '\\'
                        && matches!(chars.peek(), Some(&next) if next == c || next == '\\' || (c == '"' && next == '$'));
                    word.push(if escapable { chars.next().unwrap() } else { c2 });
                }
            }
            '\\' => {
                in_word = true;
                if let Some(escaped) = chars.next() {
                    unescape(escaped, &mut chars, &mut word);
                }
            }
            _ => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

/// Handles the character after a backslash outside quotes.
fn unescape(escaped: char, chars: &mut std::iter::Peekable<std::str::Chars>, word: &mut String) {
    let hex = |chars: &mut std::iter::Peekable<std::str::Chars>, max: usize| {
        let mut digits = String::new();
        while digits.len() < max
            && let Some(&d) = chars.peek()
            && d.is_ascii_hexdigit()
        {
            digits.push(d);
            chars.next();
        }
        u32::from_str_radix(&digits, 16)
            .ok()
            .and_then(char::from_u32)
            .unwrap_or(char::REPLACEMENT_CHARACTER)
    };
    match escaped {
        'e' => word.push('\x1b'),
        'n' => word.push('\n'),
        't' => word.push('\t'),
        'r' => word.push('\r'),
        'a' => word.push('\x07'),
        'b' => word.push('\x08'),
        'x' | 'X' => word.push(hex(chars, 2)),
        'u' => word.push(hex(chars, 4)),
        'U' => word.push(hex(chars, 8)),
        'c' => {
            if let Some(c) = chars.next() {
                word.push(((c as u8) & 0x1f) as char);
            }
        }
        other => word.push(other),
    }
}

/// Removes color codes; `fish -i` colors the `# Defined in` comments.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.next() == Some('[') {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::MAX_ACTION_LEN;

    /// Trimmed from real `bind` output on CachyOS, with the autopair plugin.
    const SAMPLE: &str = r#"bind --preset '' self-insert
bind --preset tab complete
bind --preset ctrl-r history-pager
bind --preset alt-\< beginning-of-buffer
bind --preset ctrl-alt-h backward-kill-word
bind --preset \e\[1\;9A history-token-search-backward
bind --preset alt-s 'for cmd in sudo doas please run0; if command -q $cmd; fish_commandline_prepend $cmd; break; end; end'
bind --preset alt-- something
bind --preset ctrl-x,ctrl-e edit_command_buffer
[31m# Defined in /usr/share/cachyos-fish-config/cachyos-config.fish:[39m
bind \$ __history_previous_command_arguments
[31m# Defined in /usr/share/fish/vendor_conf.d/autopair.fish:[39m
bind tab _autopair_tab
bind \' "_autopair_insert_same \\'"
"#;

    fn parsed() -> Vec<Binding> {
        // Real output has ESC before the color codes; a raw string can't hold it.
        parse(
            &SAMPLE
                .replace("[31m", "\x1b[31m")
                .replace("[39m", "\x1b[39m"),
        )
    }

    fn find<'a>(bindings: &'a [Binding], keys: &str) -> &'a Binding {
        bindings
            .iter()
            .find(|b| b.keys() == keys)
            .unwrap_or_else(|| panic!("no binding for {keys}"))
    }

    #[test]
    fn named_keys_and_modifiers() {
        let bs = parsed();
        assert_eq!(find(&bs, "Ctrl+R").action, "history-pager");
        assert_eq!(find(&bs, "Alt+<").action, "beginning-of-buffer");
        assert_eq!(find(&bs, "Ctrl+Alt+H").action, "backward-kill-word");
        assert_eq!(find(&bs, "Alt+-").action, "something");
    }

    #[test]
    fn self_insert_and_raw_sequences_are_skipped() {
        let bs = parsed();
        assert!(bs.iter().all(|b| b.action != "self-insert"));
        assert!(
            bs.iter()
                .all(|b| b.action != "history-token-search-backward")
        );
    }

    #[test]
    fn user_binding_replaces_preset_and_knows_its_file() {
        let bs = parsed();
        let tabs: Vec<_> = bs.iter().filter(|b| b.keys() == "Tab").collect();
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].action, "_autopair_tab");
        assert_eq!(
            tabs[0].source,
            Source::Config {
                path: "/usr/share/fish/vendor_conf.d/autopair.fish".into(),
                line: None
            }
        );
        assert_eq!(find(&bs, "Ctrl+R").source, Source::Default);
    }

    #[test]
    fn escaped_and_quoted_keys() {
        let bs = parsed();
        assert_eq!(
            find(&bs, "$").action,
            "__history_previous_command_arguments"
        );
        assert_eq!(find(&bs, "'").action, r"_autopair_insert_same \'");
    }

    #[test]
    fn sequences() {
        let bs = parsed();
        let edit = find(&bs, "Ctrl+X, Ctrl+E");
        assert_eq!(edit.chord, "ctrl+x".parse().unwrap());
        assert_eq!(edit.action, "edit_command_buffer");
    }

    #[test]
    fn long_commands_are_shortened() {
        let bs = parsed();
        let action = &find(&bs, "Alt+S").action;
        assert_eq!(action.chars().count(), MAX_ACTION_LEN);
        assert!(action.starts_with("for cmd in sudo doas please run0;") && action.ends_with('…'));
    }

    #[test]
    fn vi_modes() {
        let bs = parse(
            "bind --preset -M insert ctrl-r history-pager\n\
             bind --preset ctrl-r redo\n\
             bind --preset -M visual -m default y fish_clipboard_copy\n",
        );
        assert_eq!(
            find(&bs, "Ctrl+R").describe(),
            "history-pager (insert mode)"
        );
        let normal = bs.iter().find(|b| b.action == "redo").unwrap();
        assert_eq!(normal.describe(), "redo (normal mode)");
        assert_eq!(
            find(&bs, "Y").describe(),
            "fish_clipboard_copy (visual mode)"
        );
    }

    #[test]
    fn emacs_style_default_mode_has_no_label() {
        assert_eq!(find(&parsed(), "Ctrl+R").mode, None);
    }
}
