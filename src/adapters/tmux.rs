//! tmux, read by asking tmux for its key bindings.
//!
//! `tmux list-keys` prints every binding, built-in ones included, but it needs a
//! server. keydibs never touches your sessions: it starts a separate server on a
//! socket of its own, which loads your config, lists the keys and shuts down. A
//! second one started without any config (`-f /dev/null`) lists tmux's built-in
//! bindings, so the ones you changed can be told apart. tmux can't say where a
//! binding was made, so keydibs looks for a `bind` line with the same key and
//! table in your config files and the ones they `source-file`.
//!
//! The key table decides what a binding does to the layers below:
//! - `root` (`bind -n`) bindings take the key from the shell and apps.
//! - `prefix` bindings are sequences: the prefix key (`Ctrl+B` by default), then
//!   the key, and the same after the second prefix key (`prefix2`), if there is
//!   one. The prefix keys themselves are taken, so they're bindings of their own.
//! - Tables that a key switches to (`switch-client -T move`) are sequences too:
//!   the keys that lead there, then the key (`Ctrl+B, G, Up`).
//! - Copy mode and tables nothing switches to only work in their own mode, so
//!   they never take keys from anything. Only the copy mode table `mode-keys`
//!   picks is read.
//!
//! Only read when tmux looks used: there's a config, a server is running, or
//! keydibs runs inside tmux. Otherwise its prefix key would block `Ctrl+B` in
//! your shell for nothing. Skipped: mouse keys, keypad keys and `User` keys.

use std::cell::Cell;
use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, bail};

use super::{Adapter, COMMAND_TIMEOUT, ConfigFiles, config_home, on_path, run, shorten};
use crate::binding::{Binding, Grabs, Layer, Source};
use crate::chord::{Chord, Key, Mods};

const TOOL: &str = "tmux";

/// Once the config is loaded: the options that shape the keymap, the config
/// files tmux read, then every binding.
const SCRIPT: &[&str] = &[
    "start-server",
    ";",
    "display-message",
    "-p",
    "keydibs-prefix #{prefix}",
    ";",
    "display-message",
    "-p",
    "keydibs-prefix2 #{prefix2}",
    ";",
    "display-message",
    "-p",
    "keydibs-mode-keys #{mode-keys}",
    ";",
    "display-message",
    "-p",
    "keydibs-extended-keys #{extended-keys}",
    ";",
    "display-message",
    "-p",
    "keydibs-config-files #{config_files}",
    ";",
    "list-keys",
    ";",
    "kill-server",
];

#[derive(Default)]
pub struct Tmux {
    /// Set by `load` from the `extended-keys` option.
    extended_keys: Cell<bool>,
}

impl Adapter for Tmux {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn layer(&self) -> Layer {
        Layer::Multiplexer
    }

    fn where_to_change(&self) -> &'static str {
        "a bind line in ~/.config/tmux/tmux.conf or ~/.tmux.conf"
    }

    /// tmux never passes the kitty keyboard protocol on: a program inside it that
    /// asks for it still gets `Ctrl+Shift+Z` as `Ctrl+Z`. With `extended-keys`
    /// on, it sends more keys to programs that ask with xterm's modifyOtherKeys,
    /// as long as the terminal sends them to tmux, which keydibs can't tell. So
    /// those shortcuts only count as not arriving with it off, the default.
    fn kitty_keyboard(&self) -> bool {
        self.extended_keys.get()
    }

    fn load(&self) -> anyhow::Result<Option<Vec<Binding>>> {
        if !on_path(TOOL) || !in_use() {
            return Ok(None);
        }
        let listing = list_keys("user", None).context("running tmux")?;
        let defaults =
            list_keys("defaults", Some("/dev/null")).context("running tmux without your config")?;
        if !listing
            .lines()
            .any(|line| line.starts_with("keydibs-prefix "))
        {
            bail!("tmux didn't list its key bindings");
        }
        let options = Options::parse(&listing);
        self.extended_keys.set(options.extended_keys);
        let files = config_files(&options.config_files);
        Ok(Some(parse(&listing, &defaults, &ConfigFiles::read(files))))
    }
}

/// Whether tmux is in use, not just installed.
fn in_use() -> bool {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let configs = [
        home.as_ref().map(|home| home.join(".tmux.conf")),
        config_home().map(|dir| dir.join("tmux/tmux.conf")),
        home.as_ref()
            .map(|home| home.join(".config/tmux/tmux.conf")),
    ];
    std::env::var_os("TMUX").is_some()
        || default_socket().is_some_and(|socket| socket.exists())
        || configs.iter().flatten().any(|path| path.is_file())
}

/// Where tmux puts the socket of the server you normally use.
fn default_socket() -> Option<PathBuf> {
    let uid = std::fs::metadata("/proc/self").ok()?.uid();
    let dir = std::env::var_os("TMUX_TMPDIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    Some(dir.join(format!("tmux-{uid}/default")))
}

/// Runs [`SCRIPT`] on a server of its own, with `config` instead of the user's
/// config if given. `name` keeps the sockets of the two runs apart.
fn list_keys(name: &str, config: Option<&str>) -> anyhow::Result<String> {
    let socket = std::env::temp_dir().join(format!("keydibs-tmux-{}-{name}", std::process::id()));
    let mut command = Command::new(TOOL);
    // Inside tmux, $TMUX would point the command at the user's server.
    command.env_remove("TMUX").arg("-S").arg(&socket);
    if let Some(config) = config {
        command.args(["-f", config]);
    }
    let result = run(command.args(SCRIPT), COMMAND_TIMEOUT);
    if result.is_err() {
        // A config that hangs leaves the server running.
        let _ = run(
            Command::new(TOOL).arg("-S").arg(&socket).arg("kill-server"),
            COMMAND_TIMEOUT,
        );
    }
    // tmux leaves its socket behind when the server exits.
    let _ = std::fs::remove_file(&socket);

    let output = result?;
    if !output.status.success() {
        bail!(
            "`tmux list-keys` failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The config files tmux read, and the ones they `source-file`, as far as they
/// can be followed: paths with wildcards or `#{formats}` are left out.
fn config_files(read: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut seen = Vec::new();
    for path in read {
        follow(path.clone(), &mut files, &mut seen);
    }
    files
}

/// Adds `path` and the files it sources to `files`, in the order tmux reads
/// them. `seen` holds their canonical paths, so files that source each other
/// are only read once.
fn follow(path: PathBuf, files: &mut Vec<PathBuf>, seen: &mut Vec<PathBuf>) {
    let Ok(canonical) = std::fs::canonicalize(&path) else {
        return;
    };
    if seen.contains(&canonical) {
        return;
    }
    seen.push(canonical);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    files.push(path.clone());

    let home = std::env::var_os("HOME").map(PathBuf::from);
    for line in text.lines() {
        let Some((command, mut rest)) = next_word(line) else {
            continue;
        };
        if command != "source-file" && command != "source" {
            continue;
        }
        while let Some((word, after)) = next_word(rest) {
            rest = after;
            if word.starts_with('-') {
                continue;
            }
            if word.contains(['*', '?', '[', '#']) {
                break;
            }
            let sourced = match (word.strip_prefix("~/"), &home) {
                (Some(relative), Some(home)) => home.join(relative),
                _ => PathBuf::from(&word),
            };
            // Relative paths are relative to the file that sources them.
            let sourced = match path.parent() {
                Some(dir) if sourced.is_relative() => dir.join(sourced),
                _ => sourced,
            };
            follow(sourced, files, seen);
            break;
        }
    }
}

/// The options [`SCRIPT`] prints before the bindings.
#[derive(Debug, Default)]
struct Options {
    prefix: Option<Chord>,
    prefix2: Option<Chord>,
    vi: bool,
    extended_keys: bool,
    config_files: Vec<PathBuf>,
}

impl Options {
    fn parse(listing: &str) -> Options {
        let mut options = Options::default();
        for line in listing.lines() {
            let Some((name, value)) = line
                .strip_prefix("keydibs-")
                .and_then(|l| l.split_once(' '))
            else {
                continue;
            };
            match name {
                "prefix" => options.prefix = parse_key(value),
                "prefix2" => options.prefix2 = parse_key(value),
                "mode-keys" => options.vi = value == "vi",
                "extended-keys" => options.extended_keys = value != "off",
                "config-files" => {
                    options.config_files = value
                        .split(',')
                        .filter(|path| !path.is_empty())
                        .map(PathBuf::from)
                        .collect()
                }
                _ => {}
            }
        }
        options
    }
}

/// One binding as `list-keys` prints it, or a `bind` line in a config.
#[derive(Debug, PartialEq)]
struct Entry {
    table: String,
    /// Unquoted, as tmux names it: `C-b`, `M-Left`, `"`.
    key: String,
    command: String,
}

/// Turns what [`SCRIPT`] prints, with the user's config and without
/// (`defaults`), into bindings. `files` are where to look for their source.
pub(crate) fn parse(listing: &str, defaults: &str, files: &ConfigFiles) -> Vec<Binding> {
    let options = Options::parse(listing);
    let default_options = Options::parse(defaults);
    let defaults: Vec<Entry> = defaults.lines().filter_map(entry).collect();
    let copy_table = if options.vi {
        "copy-mode-vi"
    } else {
        "copy-mode"
    };

    // The prefix keys come first: they're what takes the key, before any
    // sequence that starts with them.
    let mut bindings = Vec::new();
    for (prefix, default, option, action) in [
        (
            &options.prefix,
            &default_options.prefix,
            "prefix",
            "prefix key",
        ),
        (
            &options.prefix2,
            &default_options.prefix2,
            "prefix2",
            "second prefix key",
        ),
    ] {
        let Some(prefix) = prefix else {
            continue;
        };
        let source = if Some(prefix) == default.as_ref() {
            Source::Default
        } else {
            option_source(option, files)
        };
        bindings.push(binding(
            prefix.clone(),
            Vec::new(),
            action,
            Grabs::Always,
            None,
            source,
        ));
    }

    let entries: Vec<Entry> = listing.lines().filter_map(entry).collect();
    let paths = paths(&entries, &options);
    for entry in &entries {
        let Some(key) = parse_key(&entry.key) else {
            continue;
        };
        let source = if defaults.contains(entry) {
            Source::Default
        } else {
            source(entry, files)
        };
        // Copy mode commands are all `send-keys -X <what it does>`.
        let action = entry
            .command
            .strip_prefix("send-keys -X ")
            .unwrap_or(&entry.command);

        let table = entry.table.as_str();
        if let Some(paths) = paths.get(table) {
            for path in paths {
                let mut keys = path.iter().cloned().chain([key.clone()]);
                let chord = keys.next().expect("there's always the key itself");
                let then = keys.collect();
                bindings.push(binding(
                    chord,
                    then,
                    action,
                    Grabs::Always,
                    None,
                    source.clone(),
                ));
            }
            continue;
        }
        let mode = match table {
            "copy-mode" | "copy-mode-vi" if table != copy_table => continue,
            "copy-mode" | "copy-mode-vi" => "copy",
            other => other,
        };
        bindings.push(binding(
            key,
            Vec::new(),
            action,
            Grabs::Never,
            Some(mode),
            source,
        ));
    }
    bindings
}

/// The longest sequence of keys followed into a key table.
const MAX_KEYS: usize = 4;

/// The keys that lead into each key table: none into `root`, the prefix keys
/// into `prefix`, and into other tables the keys bound to `switch-client -T
/// <table>`, after the keys that lead to the table they're in. Tables nothing
/// switches to, like copy mode, aren't in it.
fn paths(entries: &[Entry], options: &Options) -> HashMap<String, Vec<Vec<Chord>>> {
    let mut paths: HashMap<String, Vec<Vec<Chord>>> = HashMap::new();
    paths.insert("root".to_string(), vec![Vec::new()]);
    let prefixes: Vec<Vec<Chord>> = [&options.prefix, &options.prefix2]
        .into_iter()
        .flatten()
        .map(|prefix| vec![prefix.clone()])
        .collect();
    if !prefixes.is_empty() {
        paths.insert("prefix".to_string(), prefixes);
    }

    let switches: Vec<(&str, &str, Chord)> = entries
        .iter()
        .filter_map(|e| {
            Some((
                e.table.as_str(),
                switches_to(&e.command)?,
                parse_key(&e.key)?,
            ))
        })
        .filter(|(_, to, _)| !matches!(*to, "root" | "copy-mode" | "copy-mode-vi"))
        .collect();
    loop {
        let mut added = false;
        for (from, to, key) in &switches {
            let Some(from_paths) = paths.get(*from).cloned() else {
                continue;
            };
            let into = paths.entry(to.to_string()).or_default();
            for mut path in from_paths {
                path.push(key.clone());
                // A way in that goes through the table again, like a table that
                // switches back to itself to repeat, isn't a new way in.
                if path.len() <= MAX_KEYS && !into.iter().any(|known| path.starts_with(known)) {
                    into.push(path);
                    added = true;
                }
            }
        }
        if !added {
            return paths;
        }
    }
}

/// The key table a command switches to: `move` for `switch-client -T move`.
fn switches_to(command: &str) -> Option<&str> {
    let mut words = command.split_whitespace();
    words.find(|word| matches!(*word, "switch-client" | "switchc"))?;
    let mut args = words.take_while(|word| !matches!(*word, ";" | "\\;" | "}"));
    args.find(|word| *word == "-T")?;
    args.next()
}

fn binding(
    chord: Chord,
    then: Vec<Chord>,
    action: &str,
    grabs: Grabs,
    mode: Option<&str>,
    source: Source,
) -> Binding {
    Binding {
        chord,
        action: shorten(action),
        tool: TOOL,
        layer: Layer::Multiplexer,
        grabs,
        mode: mode.map(str::to_string),
        then,
        source,
    }
}

/// Reads a `bind-key` line: `bind-key -r -T prefix Up select-pane -U`. Config
/// lines may also use `bind`, `-n` for the root table and grouped flags.
fn entry(line: &str) -> Option<Entry> {
    let (command, mut rest) = next_word(line)?;
    if command != "bind-key" && command != "bind" {
        return None;
    }
    let mut table = "prefix".to_string();
    loop {
        let (word, after) = next_word(rest)?;
        rest = after;
        let Some(flags) = word.strip_prefix('-').filter(|flags| !flags.is_empty()) else {
            return Some(Entry {
                table,
                key: word,
                command: rest.split_whitespace().collect::<Vec<_>>().join(" "),
            });
        };
        for flag in flags.chars() {
            match flag {
                'n' => table = "root".to_string(),
                'T' => (table, rest) = next_word(rest)?,
                'N' => (_, rest) = next_word(rest)?,
                _ => {}
            }
        }
    }
}

/// Where a binding was most likely made: a `bind` line in `files` for the
/// same key in the same table.
fn source(listed: &Entry, files: &ConfigFiles) -> Source {
    let key = parse_key(&listed.key);
    files.find(|line| {
        entry(line).is_some_and(|found| found.table == listed.table && parse_key(&found.key) == key)
    })
}

/// Where an option like `prefix` was set: `set -g prefix C-a`.
fn option_source(option: &str, files: &ConfigFiles) -> Source {
    files.find(|line| {
        let mut words = line.split_whitespace();
        matches!(words.next(), Some("set" | "set-option"))
            && words.find(|word| !word.starts_with('-')) == Some(option)
    })
}

/// The first word of `s` and what follows it, with tmux's quotes and escapes
/// undone: `\#`, `'"'`, `"C-'"`, `C-\\`.
fn next_word(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start();
    let mut word = String::new();
    let mut chars = s.char_indices();
    let mut quote = None;
    while let Some((i, c)) = chars.next() {
        match (quote, c) {
            (None, ' ' | '\t') => return Some((word, &s[i..])),
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None | Some('"'), '\\') => {
                if let Some((_, escaped)) = chars.next() {
                    word.push(escaped);
                }
            }
            (_, c) => word.push(c),
        }
    }
    (!s.is_empty()).then_some((word, ""))
}

/// Parses a key as tmux names it: `C-b`, `M-Left`, `C-M-S-F5`, `^x`, `BTab`.
/// Returns `None` for mouse, keypad and `User` keys.
fn parse_key(name: &str) -> Option<Chord> {
    let mut mods = Mods::empty();
    let mut rest = name;
    // `C--` is Ctrl and the minus key, so never strip the last character.
    loop {
        let flag = match rest.as_bytes() {
            [b'C', b'-', _, ..] => Mods::CTRL,
            [b'M', b'-', _, ..] => Mods::ALT,
            [b'S', b'-', _, ..] => Mods::SHIFT,
            [b'^', _, ..] => {
                mods |= Mods::CTRL;
                rest = &rest[1..];
                continue;
            }
            _ => break,
        };
        mods |= flag;
        rest = &rest[2..];
    }

    let mut chars = rest.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        // An uppercase letter is the letter with Shift: `M-H` is Alt+Shift+H.
        if c.is_ascii_uppercase() {
            mods |= Mods::SHIFT;
        }
        let key = if c == ' ' {
            "space".to_string()
        } else {
            c.to_ascii_lowercase().to_string()
        };
        return Some(Chord {
            mods,
            key: Key::parse(&key)?,
        });
    }

    // tmux ignores case in key names.
    let lower = rest.to_ascii_lowercase();
    let key = match lower.as_str() {
        "btab" => {
            mods |= Mods::SHIFT;
            "tab"
        }
        "bspace" => "backspace",
        "ic" => "insert",
        "dc" => "delete",
        "ppage" | "pgup" | "pageup" => "pageup",
        "npage" | "pgdn" | "pagedown" => "pagedown",
        named @ ("enter" | "escape" | "space" | "tab" | "up" | "down" | "left" | "right"
        | "home" | "end") => named,
        f if is_function_key(f) => f,
        _ => return None,
    };
    Some(Chord {
        mods,
        key: Key::parse(key)?,
    })
}

fn is_function_key(name: &str) -> bool {
    name.strip_prefix('f')
        .is_some_and(|n| !n.is_empty() && n.len() <= 2 && n.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from real tmux 3.8 output, with a config that sets the prefix
    /// to `C-a`, turns on vi copy mode and adds a few bindings.
    const LISTING: &str = r#"keydibs-prefix C-a
keydibs-prefix2 None
keydibs-mode-keys vi
keydibs-extended-keys off
keydibs-config-files /etc/tmux.conf,/home/me/.tmux.conf,/home/me/.config/tmux/tmux.conf
bind-key    -T copy-mode    C-Space                   send-keys -X begin-selection
bind-key    -T copy-mode    q                         send-keys -X cancel
bind-key    -T copy-mode-vi C-[                       send-keys -X clear-selection
bind-key    -T copy-mode-vi Escape                    send-keys -X clear-selection
bind-key    -T copy-mode-vi v                         send-keys -X begin-selection
bind-key    -T copy-mode-vi V                         send-keys -X select-line
bind-key    -T custom       x                         kill-pane
bind-key    -T move         Up                        move-pane -P top-centre
bind-key    -T orphan       y                         kill-window
bind-key -r -T resize       h                         resize-pane -L \; switch-client -T resize
bind-key    -T prefix       \"                        split-window
bind-key    -T prefix       \#                        list-buffers
bind-key    -T prefix       |                         split-window -h
bind-key    -T prefix       c                         new-window
bind-key    -T prefix       g                         switch-client -T move
bind-key    -T prefix       r                         switch-client -T resize
bind-key    -T prefix       C-a                       send-prefix
bind-key -r -T prefix       Up                        select-pane -U
bind-key    -T prefix       BTab                      new-pane -E -X 0 -Y 0 -x "75%" -y "30%" \; move-pane -P bottom-centre \; switch-mode -ks
bind-key -r -T prefix       DC                        refresh-client -c
bind-key    -T root         MouseDown1Pane            select-pane -t = \; send-keys -M
bind-key    -T root         KP7                       display-message f
bind-key    -T root         F12                       switch-client -T custom
bind-key    -T root         M-H                       previous-window
bind-key    -T root         M-h                       select-pane -L
bind-key    -T root         "C-'"                     last-window
bind-key    -T root         'M-"'                     display-message h
bind-key    -T root         C-\\                      display-message g
bind-key    -T root         C-PPage                   display-message q
bind-key    -T root         C-S-Left                  swap-window -t -1
bind-key    -T root         C-y                       display-message hi
"#;

    /// The same, trimmed from tmux 3.8 started with `-f /dev/null`.
    const DEFAULTS: &str = r#"keydibs-prefix C-b
keydibs-prefix2 None
keydibs-mode-keys emacs
keydibs-config-files /dev/null
bind-key    -T copy-mode    C-Space                   send-keys -X begin-selection
bind-key    -T copy-mode    q                         send-keys -X cancel
bind-key    -T copy-mode-vi C-[                       send-keys -X clear-selection
bind-key    -T copy-mode-vi Escape                    send-keys -X clear-selection
bind-key    -T copy-mode-vi v                         send-keys -X rectangle-toggle
bind-key    -T copy-mode-vi V                         send-keys -X select-line
bind-key    -T prefix       \"                        split-window
bind-key    -T prefix       \#                        list-buffers
bind-key    -T prefix       c                         new-window
bind-key    -T prefix       C-b                       send-prefix
bind-key -r -T prefix       Up                        select-pane -U
bind-key    -T prefix       BTab                      new-pane -E -X 0 -Y 0 -x "75%" -y "30%" \; move-pane -P bottom-centre \; switch-mode -ks
bind-key -r -T prefix       DC                        refresh-client -c
bind-key    -T root         MouseDown1Pane            select-pane -t = \; send-keys -M
"#;

    const CONFIG: &str = r#"# My tmux config
set -g prefix C-a
unbind C-b
bind C-a send-prefix
bind | split-window -h
bind -n M-h select-pane -L
bind-key -n M-H previous-window
bind -n "C-'" last-window
bind -n M-'"' display h
bind -n C-\\ display g
bind -n C-PageUp display q
bind -n C-S-Left swap-window -t -1
bind -T custom x kill-pane
bind -n F12 switch-client -T custom
set -g mode-keys vi
bind -T copy-mode-vi v send -X begin-selection
"#;

    fn files() -> ConfigFiles {
        ConfigFiles::fixture(&[
            ("/home/me/.config/tmux/tmux.conf", CONFIG),
            (
                "/home/me/.config/tmux/extra.conf",
                "bind -n C-y display hi\n",
            ),
        ])
    }

    fn parsed() -> Vec<Binding> {
        parse(LISTING, DEFAULTS, &files())
    }

    fn find<'a>(bindings: &'a [Binding], keys: &str) -> &'a Binding {
        bindings
            .iter()
            .find(|b| b.keys() == keys)
            .unwrap_or_else(|| panic!("no binding for {keys}"))
    }

    fn config(line: usize) -> Source {
        Source::Config {
            path: "/home/me/.config/tmux/tmux.conf".into(),
            line: Some(line),
        }
    }

    #[test]
    fn prefix_key_is_taken_first() {
        let bs = parsed();
        let prefix = &bs[0];
        assert_eq!(prefix.keys(), "Ctrl+A");
        assert_eq!(prefix.action, "prefix key");
        assert_eq!(prefix.grabs, Grabs::Always);
        assert_eq!(prefix.source, config(2));
        // tmux's own prefix is a built-in default.
        let bs = parse(DEFAULTS, DEFAULTS, &ConfigFiles::fixture(&[]));
        assert_eq!(bs[0].keys(), "Ctrl+B");
        assert_eq!(bs[0].source, Source::Default);
    }

    #[test]
    fn prefix_table_bindings_are_sequences() {
        let bs = parsed();
        let new_window = find(&bs, "Ctrl+A, C");
        assert_eq!(new_window.chord, "ctrl+a".parse().unwrap());
        assert_eq!(new_window.action, "new-window");
        assert_eq!(new_window.mode, None);
        assert_eq!(new_window.source, Source::Default);
        assert_eq!(find(&bs, "Ctrl+A, Up").action, "select-pane -U");
        assert_eq!(find(&bs, "Ctrl+A, Shift+Tab").source, Source::Default);
        assert_eq!(find(&bs, "Ctrl+A, Delete").action, "refresh-client -c");
        assert_eq!(find(&bs, "Ctrl+A, Ctrl+A").source, config(4));
    }

    #[test]
    fn root_bindings_take_the_key() {
        let bs = parsed();
        let pane = find(&bs, "Alt+H");
        assert_eq!(pane.action, "select-pane -L");
        assert_eq!(pane.grabs, Grabs::Always);
        assert_eq!(pane.source, config(6));
        // An uppercase letter means Shift.
        assert_eq!(find(&bs, "Alt+Shift+H").source, config(7));
        assert_eq!(find(&bs, "Ctrl+Shift+Left").source, config(12));
    }

    #[test]
    fn quoted_and_escaped_keys() {
        let bs = parsed();
        assert_eq!(find(&bs, "Ctrl+A, \"").action, "split-window");
        assert_eq!(find(&bs, "Ctrl+A, #").action, "list-buffers");
        assert_eq!(find(&bs, "Ctrl+A, |").source, config(5));
        assert_eq!(find(&bs, "Ctrl+'").source, config(8));
        assert_eq!(find(&bs, "Alt+\"").source, config(9));
        assert_eq!(find(&bs, "Ctrl+\\").source, config(10));
    }

    #[test]
    fn key_names_are_translated() {
        let bs = parsed();
        // Written `C-PageUp` in the config, printed as `C-PPage`.
        assert_eq!(find(&bs, "Ctrl+PageUp").source, config(11));
        assert_eq!(find(&bs, "F12").action, "switch-client -T custom");
        assert_eq!(parse_key("^x"), parse_key("C-x"));
        assert_eq!(parse_key("M- "), parse_key("M-Space"));
        assert_eq!(parse_key("C--").unwrap().to_string(), "Ctrl+-");
        assert_eq!(
            parse_key("C-M-S-F5").unwrap().to_string(),
            "Ctrl+Alt+Shift+F5"
        );
        assert_eq!(parse_key("bspace"), parse_key("BSpace"));
    }

    #[test]
    fn mouse_and_keypad_keys_are_skipped() {
        let bs = parsed();
        assert!(bs.iter().all(|b| !b.action.contains("send-keys -M")));
        assert!(bs.iter().all(|b| b.action != "display-message f"));
        assert_eq!(parse_key("User0"), None);
    }

    #[test]
    fn only_the_copy_mode_in_use_is_read() {
        let bs = parsed();
        assert!(bs.iter().all(|b| b.keys() != "Ctrl+Space"));
        let select = find(&bs, "V");
        assert_eq!(select.describe(), "begin-selection (copy mode)");
        assert_eq!(select.grabs, Grabs::Never);
        assert_eq!(select.source, config(16));
        assert_eq!(find(&bs, "Shift+V").source, Source::Default);
    }

    #[test]
    fn tables_keys_switch_to_are_sequences() {
        let bs = parsed();
        // `F12` in the root table switches to `custom`.
        let kill = find(&bs, "F12, X");
        assert_eq!(kill.chord, "f12".parse().unwrap());
        assert_eq!(kill.grabs, Grabs::Always);
        assert_eq!(kill.mode, None);
        assert_eq!(kill.source, config(13));
        // `g` after the prefix switches to `move`.
        assert_eq!(find(&bs, "Ctrl+A, G, Up").action, "move-pane -P top-centre");
    }

    #[test]
    fn a_table_that_switches_to_itself_is_one_sequence() {
        let bs = parsed();
        let resize: Vec<_> = bs
            .iter()
            .filter(|b| b.action.starts_with("resize-pane -L"))
            .map(|b| b.keys())
            .collect();
        assert_eq!(resize, ["Ctrl+A, R, H"]);
    }

    #[test]
    fn tables_nothing_switches_to_are_modes() {
        let bs = parsed();
        let kill = find(&bs, "Y");
        assert_eq!(kill.describe(), "kill-window (orphan mode)");
        assert_eq!(kill.grabs, Grabs::Never);
    }

    #[test]
    fn switch_targets() {
        assert_eq!(switches_to("switch-client -T move"), Some("move"));
        assert_eq!(
            switches_to("resize-pane -L \\; switch-client -T resize"),
            Some("resize")
        );
        assert_eq!(switches_to("switch-client -l \\; new-window -T x"), None);
        assert_eq!(switches_to("select-pane -T title"), None);
    }

    #[test]
    fn bindings_from_sourced_files() {
        let bs = parsed();
        assert_eq!(
            find(&bs, "Ctrl+Y").source,
            Source::Config {
                path: "/home/me/.config/tmux/extra.conf".into(),
                line: Some(1),
            }
        );
    }

    #[test]
    fn without_a_prefix_the_second_one_starts_sequences() {
        let listing = "keydibs-prefix None\nkeydibs-prefix2 C-s\n\
                       bind-key -T prefix c new-window\n";
        let bs = parse(listing, DEFAULTS, &ConfigFiles::fixture(&[]));
        assert_eq!(bs[0].action, "second prefix key");
        assert_eq!(find(&bs, "Ctrl+S, C").action, "new-window");
    }

    #[test]
    fn prefix_bindings_follow_both_prefixes() {
        let listing = "keydibs-prefix C-a\nkeydibs-prefix2 C-s\n\
                       bind-key -T prefix c new-window\n\
                       bind-key -T prefix g switch-client -T move\n\
                       bind-key -T move Up move-pane -P top-centre\n";
        let bs = parse(listing, DEFAULTS, &ConfigFiles::fixture(&[]));
        let prefixes: Vec<_> = bs.iter().take(2).map(|b| b.keys()).collect();
        assert_eq!(prefixes, ["Ctrl+A", "Ctrl+S"]);
        assert_eq!(find(&bs, "Ctrl+A, C").action, "new-window");
        assert_eq!(find(&bs, "Ctrl+S, C").action, "new-window");
        assert_eq!(find(&bs, "Ctrl+S, G, Up").action, "move-pane -P top-centre");
    }

    #[test]
    fn a_root_key_can_lead_to_the_prefix_table() {
        let listing = "keydibs-prefix C-b\nkeydibs-prefix2 None\n\
                       bind-key -T root M-a switch-client -T prefix\n\
                       bind-key -T prefix c new-window\n";
        let bs = parse(listing, DEFAULTS, &ConfigFiles::fixture(&[]));
        assert_eq!(find(&bs, "Alt+A").action, "switch-client -T prefix");
        assert_eq!(find(&bs, "Alt+A, C").action, "new-window");
        assert_eq!(find(&bs, "Ctrl+B, C").action, "new-window");
    }

    #[test]
    fn prefix_takes_fish_backward_char() {
        let mut bs = parse(DEFAULTS, DEFAULTS, &ConfigFiles::fixture(&[]));
        bs.push(Binding {
            chord: "ctrl+b".parse().unwrap(),
            action: "backward-char".into(),
            tool: "fish",
            layer: Layer::Shell,
            grabs: Grabs::Always,
            mode: None,
            then: Vec::new(),
            source: Source::Default,
        });
        let conflicts = crate::conflict::find(&bs);
        assert_eq!(conflicts.len(), 1);
        assert!(matches!(
            conflicts[0],
            crate::conflict::Conflict::Shadowed { winner, loser }
                if winner.action == "prefix key" && loser.tool == "fish"
        ));
    }

    #[test]
    fn options_are_read() {
        let options = Options::parse(LISTING);
        assert_eq!(options.prefix, parse_key("C-a"));
        assert_eq!(options.prefix2, None);
        assert!(options.vi);
        assert!(!options.extended_keys);
        assert_eq!(options.config_files.len(), 3);
        assert!(Options::parse("keydibs-extended-keys always").extended_keys);
    }

    #[test]
    fn sourced_files_are_followed() {
        let dir = std::env::temp_dir().join(format!("keydibs-tmux-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("conf.d")).unwrap();
        let main = dir.join("tmux.conf");
        std::fs::write(
            &main,
            "source-file -q conf.d/keys.conf\nsource ~/nowhere/*.conf\n",
        )
        .unwrap();
        std::fs::write(dir.join("conf.d/keys.conf"), "source-file ../tmux.conf\n").unwrap();

        let files = config_files(std::slice::from_ref(&main));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(files, [main, dir.join("conf.d/keys.conf")]);
    }
}
