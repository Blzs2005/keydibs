//! zsh, read by asking zsh itself for its key bindings.
//!
//! zsh's line editor (ZLE) is set up by `.zshrc` and the frameworks and plugins
//! it loads, so keydibs starts a throwaway `zsh -i` with your config, as opening
//! a terminal does, and reads what `bindkey -L` prints for the keymaps in use:
//! emacs, or vi's insert (`viins`) and command (`vicmd`) keymaps. A second run
//! without any config (`zsh -f`) tells the built-in defaults apart.
//!
//! zsh can't say where a binding was made, so keydibs looks for a `bindkey`
//! line with the same keys and widget in the files zsh read (listed with the
//! `SOURCE_TRACE` option), and points there.
//!
//! Only read when zsh is your login shell. Skipped: keys that just type
//! themselves, and sequences that aren't a key press keydibs can decode.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, bail};

use super::{Adapter, COMMAND_TIMEOUT, ConfigFiles, escape, is_login_shell, on_path, run, shorten};
use crate::binding::{Binding, Grabs, Layer, Source};

const TOOL: &str = "zsh";

/// Lists which keymap is in use and every binding of the keymaps that matter.
const SCRIPT: &str = "print -r -- KEYDIBS
bindkey -lL main
for keymap in emacs viins vicmd; do bindkey -LM $keymap; done";

pub struct Zsh;

impl Adapter for Zsh {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn layer(&self) -> Layer {
        Layer::Shell
    }

    fn where_to_change(&self) -> &'static str {
        "a bindkey line in ~/.zshrc"
    }

    fn load(&self) -> anyhow::Result<Option<Vec<Binding>>> {
        if !is_login_shell(TOOL) || !on_path(TOOL) {
            return Ok(None);
        }
        let user = run(
            Command::new(TOOL).args(["-o", "sourcetrace", "-i", "-c", SCRIPT]),
            COMMAND_TIMEOUT,
        )
        .context("running zsh")?;
        let defaults = run(
            Command::new(TOOL).args(["-f", "-i", "-c", SCRIPT]),
            COMMAND_TIMEOUT,
        )
        .context("running zsh without your config")?;

        let listing = String::from_utf8_lossy(&user.stdout);
        if !listing.lines().any(|line| line == "KEYDIBS") {
            bail!("zsh didn't list its key bindings ({})", user.status);
        }
        let files = sourced_files(&String::from_utf8_lossy(&user.stderr));
        Ok(Some(parse(
            &listing,
            &String::from_utf8_lossy(&defaults.stdout),
            &ConfigFiles::read(files),
        )))
    }
}

/// One binding as `bindkey -L` prints it.
#[derive(Debug, PartialEq)]
struct Entry {
    keymap: String,
    /// In zsh's notation, as printed: `^X^E`, `^[[A`.
    keys: String,
    /// The widget, or for `bindkey -s` the text it types, in quotes.
    action: String,
}

/// Turns what [`SCRIPT`] prints, with the user's config and without
/// (`defaults`), into bindings. `files` are where to look for their source.
pub(crate) fn parse(listing: &str, defaults: &str, files: &ConfigFiles) -> Vec<Binding> {
    let (_, defaults) = entries(defaults);
    let (main, entries) = entries(listing);

    let mut bindings: Vec<Binding> = Vec::new();
    for entry in entries {
        // The keymap `main` points to is the one in use; with vi's insert
        // keymap, its command keymap is too.
        let mode = match (main.as_str(), entry.keymap.as_str()) {
            ("viins", "viins") => Some("insert"),
            ("viins", "vicmd") => Some("normal"),
            (main, keymap) if main == keymap => None,
            _ => continue,
        };
        if matches!(entry.action.as_str(), "self-insert" | "undefined-key") {
            continue;
        }
        let Some(keys) = unescape(&entry.keys).and_then(|raw| escape::decode_keys(&raw)) else {
            continue;
        };
        let source = if defaults.contains(&entry) {
            Source::Default
        } else {
            source(&entry, files)
        };
        // Several sequences can stand for one key (`^[[A`, `^[OA`), for
        // different terminals. Only one of them is ever sent, so keep one
        // binding, pointing to wherever one of them was found.
        let mode = mode.map(str::to_string);
        if let Some(same) = bindings
            .iter_mut()
            .find(|b| b.mode == mode && b.chord == keys[0] && b.then == keys[1..])
        {
            if same.source == Source::Runtime
                && matches!(source, Source::Config { .. })
                && same.action == shorten(&entry.action)
            {
                same.source = source;
            }
            continue;
        }

        let mut keys = keys.into_iter();
        bindings.push(Binding {
            chord: keys.next().expect("decode_keys never returns no keys"),
            action: shorten(&entry.action),
            tool: TOOL,
            layer: Layer::Shell,
            grabs: Grabs::Always,
            mode,
            then: keys.collect(),
            source,
        });
    }
    bindings
}

/// The keymap `main` points to, and the bindings after the marker.
fn entries(listing: &str) -> (String, Vec<Entry>) {
    let mut main = "emacs".to_string();
    let mut entries = Vec::new();
    let lines = listing.lines().skip_while(|line| *line != "KEYDIBS");
    for line in lines {
        if let Some(rest) = line.strip_prefix("bindkey -A ")
            && let Some((keymap, "main")) = rest.split_once(' ')
        {
            main = keymap.to_string();
        } else if let Some(entry) = parse_line(line) {
            entries.push(entry);
        }
    }
    (main, entries)
}

/// `bindkey -M emacs "^A" beginning-of-line`, `bindkey -a "^D" list-choices`
/// (`vicmd`) or `bindkey -s -M emacs "^[l" "ls^J"` (text to type). Ranges
/// (`bindkey -R`) are left out: they're only ever `self-insert`.
fn parse_line(line: &str) -> Option<Entry> {
    let mut rest = line.strip_prefix("bindkey ")?;
    let mut keymap = None;
    let mut text = false;
    while !rest.starts_with('"') {
        let (option, after) = rest.split_once(' ')?;
        rest = after;
        match option {
            "-s" => text = true,
            "-a" => keymap = Some("vicmd"),
            "-M" => {
                let (name, after) = rest.split_once(' ')?;
                keymap = Some(name);
                rest = after;
            }
            _ => return None,
        }
    }
    let (keys, after) = quoted(rest)?;
    let after = after.trim();
    let action = if text {
        format!("\"{}\"", quoted(after)?.0)
    } else {
        after.to_string()
    };
    Some(Entry {
        keymap: keymap?.to_string(),
        keys: keys.to_string(),
        action,
    })
}

/// The text inside the double quotes `text` starts with, as written, and
/// what follows.
fn quoted(text: &str) -> Option<(&str, &str)> {
    let inner = text.strip_prefix('"')?;
    let mut escaped = false;
    for (i, c) in inner.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => return Some((&inner[..i], &inner[i + 1..])),
            _ => {}
        }
    }
    None
}

/// The bytes zsh's notation stands for: `^X` for control characters (`^[` is
/// Escape, `^?` Delete), `\M-` for Meta, and backslash-escaped characters.
fn unescape(notation: &str) -> Option<String> {
    let mut chars = notation.chars();
    let mut bytes = String::new();
    while let Some(c) = chars.next() {
        match c {
            '^' => match chars.next() {
                Some(c) => bytes.push(escape::control_char(c)?),
                None => bytes.push('^'),
            },
            '\\' => match chars.next()? {
                // Meta is sent as Escape first, like Alt.
                'M' if chars.as_str().starts_with('-') => {
                    chars.next();
                    bytes.push('\x1b');
                }
                escaped => bytes.push(escaped),
            },
            c => bytes.push(c),
        }
    }
    Some(bytes)
}

/// Where a binding was most likely made: a `bindkey` line in `files` with the
/// same keys and widget, for the same keymap.
fn source(entry: &Entry, files: &ConfigFiles) -> Source {
    let spellings = [entry.keys.clone(), other_spelling(&entry.keys)];
    files.find(|line| {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.first() != Some(&"bindkey") || !spellings.iter().any(|keys| line.contains(keys)) {
            return false;
        }
        // Text to type may be quoted differently than zsh prints it.
        if !entry.action.starts_with('"') && !words.contains(&entry.action.as_str()) {
            return false;
        }
        match words.iter().position(|w| *w == "-M") {
            Some(i) => words.get(i + 1) == Some(&entry.keymap.as_str()),
            None => !words.contains(&"-a") || entry.keymap == "vicmd",
        }
    })
}

/// Keys as configs often write them instead: `\C-x\C-e` for `^X^E`, `\el` for `^[l`.
fn other_spelling(keys: &str) -> String {
    let mut spelled = String::new();
    let mut chars = keys.chars();
    while let Some(c) = chars.next() {
        match (c, chars.as_str().chars().next()) {
            ('^', Some('[')) => {
                chars.next();
                spelled.push_str("\\e");
            }
            ('^', Some(letter)) if letter.is_ascii_alphabetic() => {
                chars.next();
                spelled.push_str("\\C-");
                spelled.push(letter.to_ascii_lowercase());
            }
            _ => spelled.push(c),
        }
    }
    spelled
}

/// The files zsh read at startup, from the `SOURCE_TRACE` lines on `stderr`:
/// `+/home/me/.zshrc:1> <sourcetrace>`.
fn sourced_files(stderr: &str) -> Vec<PathBuf> {
    stderr
        .lines()
        .filter_map(|line| line.strip_suffix("> <sourcetrace>"))
        .filter_map(|place| place.trim_start_matches('+').rsplit_once(':'))
        .map(|(path, _)| PathBuf::from(path))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chord::Chord;

    fn keys(notation: &str) -> Option<String> {
        let raw = unescape(notation)?;
        escape::decode_keys(&raw).map(|keys| {
            keys.iter()
                .map(Chord::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        })
    }

    #[test]
    fn zsh_notation() {
        assert_eq!(keys("^A").as_deref(), Some("Ctrl+A"));
        assert_eq!(keys("^X^E").as_deref(), Some("Ctrl+X, Ctrl+E"));
        assert_eq!(keys("^Xr").as_deref(), Some("Ctrl+X, R"));
        assert_eq!(keys("^[[A").as_deref(), Some("Up"));
        assert_eq!(keys("^[OA").as_deref(), Some("Up"));
        assert_eq!(keys("^[[1;5C").as_deref(), Some("Ctrl+Right"));
        assert_eq!(keys("^[^H").as_deref(), Some("Ctrl+Alt+H"));
        assert_eq!(keys("^[\\$").as_deref(), Some("Alt+$"));
        assert_eq!(keys("^[\\\"").as_deref(), Some("Alt+\""));
        assert_eq!(keys("^?").as_deref(), Some("Backspace"));
        assert_eq!(keys("\\M-b").as_deref(), Some("Alt+B"));
        assert_eq!(keys("^[[200~"), None);
    }

    #[test]
    fn lines_of_bindkey_output() {
        let entry = |keymap: &str, keys: &str, action: &str| {
            Some(Entry {
                keymap: keymap.into(),
                keys: keys.into(),
                action: action.into(),
            })
        };
        assert_eq!(
            parse_line(r#"bindkey -M emacs "^A" beginning-of-line"#),
            entry("emacs", "^A", "beginning-of-line")
        );
        assert_eq!(
            parse_line(r#"bindkey -a "^D" list-choices"#),
            entry("vicmd", "^D", "list-choices")
        );
        assert_eq!(
            parse_line(r#"bindkey -s -M emacs "^[l" "^Q ls^J""#),
            entry("emacs", "^[l", r#""^Q ls^J""#)
        );
        assert_eq!(
            parse_line(r#"bindkey -M emacs "^[\"" quote-region"#),
            entry("emacs", r#"^[\""#, "quote-region")
        );
        assert_eq!(
            parse_line(r#"bindkey -R -M emacs " "-"~" self-insert"#),
            None
        );
    }

    /// Trimmed from zsh 5.9's output with CachyOS's oh-my-zsh setup and fzf.
    const LISTING: &str = r#"fastfetch output
KEYDIBS
bindkey -A emacs main
bindkey -M emacs "^A" beginning-of-line
bindkey -M emacs "^R" fzf-history-widget
bindkey -M emacs "^X^E" edit-command-line
bindkey -M emacs "^[OA" up-line-or-beginning-search
bindkey -M emacs "^[[A" up-line-or-beginning-search
bindkey -R -M emacs " "-"~" self-insert
bindkey -M emacs "^[[200~" bracketed-paste
bindkey -s -M emacs "^[l" "^Q ls^J"
bindkey -M viins "^W" backward-kill-word
bindkey -a "^D" list-choices
bindkey -a "h" vi-backward-char
"#;

    const DEFAULTS: &str = r#"KEYDIBS
bindkey -A emacs main
bindkey -M emacs "^A" beginning-of-line
bindkey -M emacs "^R" history-incremental-search-backward
bindkey -M emacs "^[[A" up-line-or-history
bindkey -M emacs "^[l" down-case-word
"#;

    fn files() -> ConfigFiles {
        ConfigFiles::fixture(&[
            (
                "/usr/share/oh-my-zsh/lib/key-bindings.zsh",
                "# Start typing + [Up-Arrow]\n\
                 bindkey -M viins \"^[[A\" up-line-or-beginning-search\n\
                 bindkey -M emacs \"^[[A\" up-line-or-beginning-search\n\
                 bindkey '\\C-x\\C-e' edit-command-line\n\
                 bindkey -M emacs '^X^E' edit-command-line\n",
            ),
            ("/home/me/.zshrc", "bindkey -s '^[l' '^q ls^j'\n"),
        ])
    }

    fn find<'a>(bs: &'a [Binding], keys: &str) -> &'a Binding {
        bs.iter()
            .find(|b| b.keys() == keys)
            .unwrap_or_else(|| panic!("no {keys} binding"))
    }

    #[test]
    fn emacs_keymap_with_sources() {
        let bs = parse(LISTING, DEFAULTS, &files());
        assert_eq!(find(&bs, "Ctrl+A").source, Source::Default);
        assert_eq!(find(&bs, "Ctrl+A").mode, None);
        // Changed from the default, but made somewhere keydibs can't see.
        assert_eq!(find(&bs, "Ctrl+R").action, "fzf-history-widget");
        assert_eq!(find(&bs, "Ctrl+R").source, Source::Runtime);
        // The line for the same keymap, not the viins one above it.
        assert_eq!(
            find(&bs, "Up").source,
            Source::Config {
                path: "/usr/share/oh-my-zsh/lib/key-bindings.zsh".into(),
                line: Some(3)
            }
        );
        // Written as `\C-x\C-e` in the file.
        assert_eq!(
            find(&bs, "Ctrl+X, Ctrl+E").source,
            Source::Config {
                path: "/usr/share/oh-my-zsh/lib/key-bindings.zsh".into(),
                line: Some(4)
            }
        );
        assert_eq!(
            find(&bs, "Alt+L").source,
            Source::Config {
                path: "/home/me/.zshrc".into(),
                line: Some(1)
            }
        );
        assert_eq!(find(&bs, "Alt+L").action, r#""^Q ls^J""#);

        let all: Vec<String> = bs.iter().map(Binding::keys).collect();
        assert_eq!(all.iter().filter(|k| *k == "Up").count(), 1);
        for left_out in ["Ctrl+W", "Ctrl+D", "H"] {
            assert!(!all.contains(&left_out.to_string()), "{left_out}");
        }
    }

    #[test]
    fn vi_mode_uses_the_vi_keymaps() {
        let listing = LISTING.replace("bindkey -A emacs main", "bindkey -A viins main");
        let bs = parse(&listing, DEFAULTS, &ConfigFiles::fixture(&[]));
        assert_eq!(find(&bs, "Ctrl+W").mode.as_deref(), Some("insert"));
        assert_eq!(find(&bs, "H").mode.as_deref(), Some("normal"));
        assert!(!bs.iter().any(|b| b.keys() == "Ctrl+A"));
    }

    #[test]
    fn other_spellings() {
        assert_eq!(other_spelling("^X^E"), r"\C-x\C-e");
        assert_eq!(other_spelling("^[l"), r"\el");
        assert_eq!(other_spelling("^[[1;5C"), r"\e[1;5C");
        assert_eq!(other_spelling("^?"), "^?");
    }

    #[test]
    fn files_from_the_source_trace() {
        let stderr = "+/home/me/.zshrc:1> <sourcetrace>\n\
                      zsh: can't change option: zle\n\
                      +/usr/share/oh-my-zsh/oh-my-zsh.sh:1> <sourcetrace>\n";
        assert_eq!(
            sourced_files(stderr),
            [
                PathBuf::from("/home/me/.zshrc"),
                PathBuf::from("/usr/share/oh-my-zsh/oh-my-zsh.sh")
            ]
        );
    }
}
