//! bash, read by asking bash itself for its key bindings.
//!
//! bash's line editor, readline, reads `~/.inputrc` (or `/etc/inputrc`), and
//! `.bashrc` can add more with `bind`. keydibs starts a throwaway `bash -i` with
//! your config, as opening a terminal does, and reads what `bind` prints for
//! the keymaps of your editing mode: emacs, or vi's insert and command keymaps.
//! A second run without any config tells the built-in defaults apart.
//!
//! bash can't say where a binding was made, so keydibs looks for a line with
//! the same keys and action in the files bash read (traced with `-x`) and the
//! inputrc files, and points there.
//!
//! Only read when bash is your login shell. Skipped: keys that just type
//! themselves, and sequences that aren't a key press keydibs can decode.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

use super::{Adapter, COMMAND_TIMEOUT, ConfigFiles, escape, is_login_shell, on_path, run, shorten};
use crate::binding::{Binding, Grabs, Layer, Source};

const TOOL: &str = "bash";

/// Lists the editing mode and every binding of the keymaps that matter.
const SCRIPT: &str = r#"printf '\nKEYDIBS\n'
bind -v
for keymap in emacs vi-insert vi-command; do
  printf 'KEYMAP %s\n' "$keymap"
  bind -m "$keymap" -p
  bind -m "$keymap" -s
  bind -m "$keymap" -X
done"#;

/// With `-x`, bash prints every command it runs, prefixed with this: the file
/// and line it comes from.
const TRACE_PROMPT: &str = "+${BASH_SOURCE}:${LINENO}> ";

pub struct Bash;

impl Adapter for Bash {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn layer(&self) -> Layer {
        Layer::Shell
    }

    fn where_to_change(&self) -> &'static str {
        "~/.inputrc, or a bind line in ~/.bashrc"
    }

    fn load(&self) -> anyhow::Result<Option<Vec<Binding>>> {
        if !is_login_shell(TOOL) || !on_path(TOOL) {
            return Ok(None);
        }
        let user = run(
            Command::new(TOOL)
                .args(["-x", "-i", "-c", SCRIPT])
                .env("PS4", TRACE_PROMPT),
            COMMAND_TIMEOUT,
        )
        .context("running bash")?;
        let defaults = run(
            Command::new(TOOL)
                .args(["--norc", "--noprofile", "-i", "-c", SCRIPT])
                .env("INPUTRC", "/dev/null"),
            COMMAND_TIMEOUT,
        )
        .context("running bash without your config")?;

        let listing = String::from_utf8_lossy(&user.stdout);
        if !listing.contains("\nKEYDIBS\n") {
            bail!("bash didn't list its key bindings ({})", user.status);
        }
        let mut files = traced_files(&String::from_utf8_lossy(&user.stderr));
        files.extend(inputrc_files());
        Ok(Some(parse(
            &listing,
            &String::from_utf8_lossy(&defaults.stdout),
            &ConfigFiles::read(files),
        )))
    }
}

/// One binding as `bind` prints it.
#[derive(Debug, PartialEq)]
struct Entry {
    keymap: String,
    /// In readline's notation, as printed: `\C-x\C-e`.
    keys: String,
    action: String,
}

/// Turns what [`SCRIPT`] prints, with the user's config and without
/// (`defaults`), into bindings. `files` are where to look for their source.
pub(crate) fn parse(listing: &str, defaults: &str, files: &ConfigFiles) -> Vec<Binding> {
    let (_, defaults) = entries(defaults);
    let (vi, entries) = entries(listing);

    let mut bindings: Vec<Binding> = Vec::new();
    for entry in entries {
        let mode = match (vi, entry.keymap.as_str()) {
            (false, "emacs") => None,
            (true, "vi-insert") => Some("insert"),
            (true, "vi-command") => Some("normal"),
            _ => continue,
        };
        if matches!(
            entry.action.as_str(),
            "self-insert" | "do-lowercase-version"
        ) {
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
        // Several sequences can stand for one key (`\e[1;5C`, `\e[5C`), for
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

/// The bindings after the marker, and whether the editing mode is vi.
fn entries(listing: &str) -> (bool, Vec<Entry>) {
    match listing.split_once("\nKEYDIBS\n") {
        Some((_, after)) => entries_of(after),
        None => (false, Vec::new()),
    }
}

fn entries_of(listing: &str) -> (bool, Vec<Entry>) {
    let mut vi = false;
    let mut keymap = String::new();
    let mut entries = Vec::new();
    for line in listing.lines() {
        if let Some(mode) = line.strip_prefix("set editing-mode ") {
            vi = mode.trim() == "vi";
        } else if let Some(name) = line.strip_prefix("KEYMAP ") {
            keymap = name.trim().to_string();
        } else if let Some((keys, action)) = parse_line(line) {
            entries.push(Entry {
                keymap: keymap.clone(),
                keys,
                action,
            });
        }
    }
    (vi, entries)
}

/// `"\C-a": beginning-of-line` (`bind -p`), `"\ew": "text"` (a macro, `bind -s`)
/// or `"\C-x\C-r" "command"` (a shell command, `bind -X`).
fn parse_line(line: &str) -> Option<(String, String)> {
    let (keys, rest) = quoted(line)?;
    let rest = rest.trim_start();
    let action = match rest.strip_prefix(':') {
        Some(value) => match quoted(value.trim_start()) {
            Some((text, _)) => format!("\"{text}\""),
            None => value.trim().to_string(),
        },
        None => quoted(rest)?.0.to_string(),
    };
    (!action.is_empty()).then(|| (keys.to_string(), action))
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

/// The bytes readline's notation stands for: `\C-x`, `\M-x`, `\e`, `\t`,
/// `\177`, `\x7f` and backslash-escaped characters.
fn unescape(notation: &str) -> Option<String> {
    let mut chars = notation.chars().peekable();
    let mut bytes = String::new();
    while chars.peek().is_some() {
        bytes.push_str(&unit(&mut chars)?);
    }
    Some(bytes)
}

/// One character, or one escape, of readline's notation.
fn unit(chars: &mut std::iter::Peekable<std::str::Chars>) -> Option<String> {
    let c = chars.next()?;
    if c != '\\' {
        return Some(c.to_string());
    }
    let escaped = chars.next()?;
    Some(match escaped {
        'C' | 'M' if chars.peek() == Some(&'-') => {
            chars.next();
            let inner = unit(chars)?;
            if escaped == 'M' {
                // Meta is sent as Escape first, like Alt.
                format!("\x1b{inner}")
            } else {
                let mut inner = inner.chars();
                let (Some(c), None) = (inner.next(), inner.next()) else {
                    return None;
                };
                escape::control_char(c)?.to_string()
            }
        }
        'e' => "\x1b".to_string(),
        'a' => "\x07".to_string(),
        'b' => "\x08".to_string(),
        'd' => "\x7f".to_string(),
        'f' => "\x0c".to_string(),
        'n' => "\n".to_string(),
        'r' => "\r".to_string(),
        't' => "\t".to_string(),
        'v' => "\x0b".to_string(),
        '0'..='7' => number(escaped.to_digit(8)?, chars, 8, 2)?,
        'x' => number(0, chars, 16, 2)?,
        other => other.to_string(),
    })
}

/// The character of an octal (`\177`) or hex (`\x7f`) escape: `value` so far,
/// followed by up to `digits` more digits.
fn number(
    mut value: u32,
    chars: &mut std::iter::Peekable<std::str::Chars>,
    radix: u32,
    digits: usize,
) -> Option<String> {
    for _ in 0..digits {
        match chars.peek().and_then(|c| c.to_digit(radix)) {
            Some(digit) => {
                value = value * radix + digit;
                chars.next();
            }
            None => break,
        }
    }
    char::from_u32(value).map(|c| c.to_string())
}

/// Where a binding was most likely made: a line in `files` with the same keys
/// and action.
fn source(entry: &Entry, files: &ConfigFiles) -> Source {
    let keys = format!("\"{}\"", entry.keys);
    // A macro or command may be quoted differently in the file than `bind`
    // prints it, so only its first word has to match.
    let action = entry
        .action
        .trim_matches('"')
        .split_whitespace()
        .next()
        .unwrap_or_default();
    files.find(|line| line.contains(&keys) && line.contains(action))
}

/// The files bash read at startup, from its `-x` trace on `stderr`.
fn traced_files(stderr: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    for line in stderr.lines() {
        let Some(place) = line
            .trim_start_matches('+')
            .split_once("> ")
            .map(|(place, _)| place)
        else {
            continue;
        };
        if let Some((path, _)) = place.rsplit_once(':')
            && path.starts_with('/')
            && !files.iter().any(|f| f == Path::new(path))
        {
            files.push(PathBuf::from(path));
        }
    }
    files
}

/// The inputrc readline reads, `$INPUTRC`, `~/.inputrc` or `/etc/inputrc`, and
/// the files it pulls in with `$include`.
fn inputrc_files() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let main = std::env::var_os("INPUTRC")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            home.as_ref()
                .map(|home| home.join(".inputrc"))
                .filter(|path| path.exists())
        })
        .unwrap_or_else(|| PathBuf::from("/etc/inputrc"));

    let mut files = vec![main.clone()];
    let text = std::fs::read_to_string(&main).unwrap_or_default();
    for line in text.lines() {
        if let Some(include) = line.trim().strip_prefix("$include") {
            let include = include.trim();
            files.push(match (include.strip_prefix("~/"), &home) {
                (Some(rest), Some(home)) => home.join(rest),
                _ => PathBuf::from(include),
            });
        }
    }
    files
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
    fn readline_notation() {
        assert_eq!(keys(r"\C-a").as_deref(), Some("Ctrl+A"));
        assert_eq!(keys(r"\C-x\C-e").as_deref(), Some("Ctrl+X, Ctrl+E"));
        assert_eq!(keys(r"\C-xe").as_deref(), Some("Ctrl+X, E"));
        assert_eq!(keys(r"\eb").as_deref(), Some("Alt+B"));
        assert_eq!(keys(r"\M-b").as_deref(), Some("Alt+B"));
        assert_eq!(keys(r"\e\C-h").as_deref(), Some("Ctrl+Alt+H"));
        assert_eq!(keys(r"\e[1;5C").as_deref(), Some("Ctrl+Right"));
        assert_eq!(keys(r"\e\e[D").as_deref(), Some("Alt+Left"));
        assert_eq!(keys(r"\C-?").as_deref(), Some("Backspace"));
        assert_eq!(keys(r"\e\\").as_deref(), Some("Alt+\\"));
        assert_eq!(keys(r"\C-\\").as_deref(), Some("Ctrl+\\"));
        assert_eq!(keys(r"\t").as_deref(), Some("Tab"));
        assert_eq!(keys(r"\177").as_deref(), Some("Backspace"));
        assert_eq!(keys(r"\x18\x05").as_deref(), Some("Ctrl+X, Ctrl+E"));
        assert_eq!(keys(r"\e[200~"), None);
    }

    #[test]
    fn lines_of_bind_output() {
        assert_eq!(
            parse_line(r#""\C-a": beginning-of-line"#),
            Some((r"\C-a".into(), "beginning-of-line".into()))
        );
        assert_eq!(
            parse_line(r#""\ew": "macro \"text\"""#),
            Some((r"\ew".into(), r#""macro \"text\"""#.into()))
        );
        assert_eq!(
            parse_line(r#""\C-x\C-r" "echo hi""#),
            Some((r"\C-x\C-r".into(), "echo hi".into()))
        );
        assert_eq!(
            parse_line(r#""\"": self-insert"#),
            Some((r#"\""#.into(), "self-insert".into()))
        );
        assert_eq!(parse_line("# abort (not bound)"), None);
        assert_eq!(parse_line("set editing-mode emacs"), None);
    }

    /// Trimmed from bash 5.3's output with Arch's /etc/inputrc.
    const LISTING: &str = r#"some greeting from .bashrc

KEYDIBS
set bell-style audible
set editing-mode emacs
KEYMAP emacs
"\C-g": abort
"\C-x\C-g": abort
"\e[D": backward-char
"\e[1;5D": backward-word
"\e[5D": backward-word
"\eb": backward-word
"\C-a": beginning-of-line
"\e[1~": beginning-of-line
"\C-x\C-e": edit-and-execute-command
"a": self-insert
"\eA": do-lowercase-version
"\C-r": reverse-search-history
"\e[200~": bracketed-paste-begin
"\ew": "macro text"
"\C-x\C-r" "fzf-history"
KEYMAP vi-insert
"\C-w": unix-word-rubout
KEYMAP vi-command
"h": backward-char
"#;

    const DEFAULTS: &str = r#"
KEYDIBS
set editing-mode emacs
KEYMAP emacs
"\C-g": abort
"\C-x\C-g": abort
"\e[D": backward-char
"\eb": backward-word
"\C-a": beginning-of-line
"\C-x\C-e": edit-and-execute-command
"\C-r": reverse-search-history
"#;

    fn parsed() -> Vec<Binding> {
        let files = ConfigFiles::fixture(&[
            (
                "/etc/inputrc",
                "$if mode=emacs\n\"\\e[1~\": beginning-of-line\n\"\\e[1;5D\": backward-word\n$endif\n",
            ),
            (
                "/home/me/.bashrc",
                "# keys\nbind '\"\\ew\": \"macro text\"'\nbind -x '\"\\C-x\\C-r\": fzf-history'\n",
            ),
        ]);
        parse(LISTING, DEFAULTS, &files)
    }

    fn find<'a>(bs: &'a [Binding], keys: &str) -> &'a Binding {
        bs.iter()
            .find(|b| b.keys() == keys)
            .unwrap_or_else(|| panic!("no {keys} binding"))
    }

    #[test]
    fn emacs_keymap_with_sources() {
        let bs = parsed();
        assert_eq!(find(&bs, "Ctrl+A").action, "beginning-of-line");
        assert_eq!(find(&bs, "Ctrl+A").source, Source::Default);
        assert_eq!(find(&bs, "Ctrl+X, Ctrl+E").mode, None);
        assert_eq!(
            find(&bs, "Home").source,
            Source::Config {
                path: "/etc/inputrc".into(),
                line: Some(2)
            }
        );
        assert_eq!(
            find(&bs, "Ctrl+Left").source,
            Source::Config {
                path: "/etc/inputrc".into(),
                line: Some(3)
            }
        );
        assert_eq!(find(&bs, "Alt+W").action, "\"macro text\"");
        assert_eq!(
            find(&bs, "Alt+W").source,
            Source::Config {
                path: "/home/me/.bashrc".into(),
                line: Some(2)
            }
        );
        assert_eq!(
            find(&bs, "Ctrl+X, Ctrl+R").source,
            Source::Config {
                path: "/home/me/.bashrc".into(),
                line: Some(3)
            }
        );
    }

    #[test]
    fn typing_keys_and_other_keymaps_are_left_out() {
        let bs = parsed();
        let all: Vec<String> = bs.iter().map(Binding::keys).collect();
        for left_out in ["A", "Alt+Shift+A", "H", "Ctrl+W"] {
            assert!(!all.contains(&left_out.to_string()), "{left_out}");
        }
        // `\e[5D` isn't decoded, `\e[200~` neither.
        assert_eq!(all.iter().filter(|k| *k == "Ctrl+Left").count(), 1);
    }

    #[test]
    fn vi_mode_uses_the_vi_keymaps() {
        let listing = LISTING.replace("set editing-mode emacs", "set editing-mode vi");
        let bs = parse(&listing, DEFAULTS, &ConfigFiles::fixture(&[]));
        assert_eq!(find(&bs, "Ctrl+W").mode.as_deref(), Some("insert"));
        assert_eq!(find(&bs, "H").mode.as_deref(), Some("normal"));
        assert!(!bs.iter().any(|b| b.keys() == "Ctrl+A"));
    }

    #[test]
    fn files_from_the_trace() {
        let trace = "+/etc/bash.bashrc:9> [[ hxB == *i* ]]\n\
                     ++/usr/share/bash-completion/bash_completion:12> shopt -s extglob\n\
                     +/etc/bash.bashrc:20> PS1='$ '\n\
                     +:1> printf '\\nKEYDIBS\\n'\n\
                     bash: no job control in this shell\n";
        assert_eq!(
            traced_files(trace),
            [
                PathBuf::from("/etc/bash.bashrc"),
                PathBuf::from("/usr/share/bash-completion/bash_completion")
            ]
        );
    }
}
