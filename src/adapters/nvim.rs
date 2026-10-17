//! Neovim, read by asking a headless Neovim for its keymap.
//!
//! Plugins and Lua configs build the keymap in code, so keydibs starts
//! `nvim --headless` with your config, as opening Neovim does, and runs
//! `nvim.lua` in it to list the mappings of every mode. With `-V1`, Neovim
//! remembers which file and line set each mapping.
//!
//! Vim's built-in commands (`Ctrl+W` for windows, `Ctrl+R` in Insert mode…)
//! aren't mappings, so they come from `:help index` (`doc/index.txt`) of the
//! installed Neovim instead. Only the ones on a modifier or a named key are
//! kept: plain letters can't clash with anything above Neovim. A command that
//! starts with another one's keys (`Ctrl+W, J` after `Ctrl+W`) is left out,
//! since only the first key can be blocked.
//!
//! Skipped: mouse keys, keypad keys, `<Plug>` mappings, and buffer-local
//! mappings, which only exist once a file is open (LSP keys, for example).

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};
use serde::Deserialize;

use super::{Adapter, COMMAND_TIMEOUT, on_path, run, shorten};
use crate::binding::{Binding, Grabs, Layer, Source};
use crate::chord::{Chord, Key, Mods};

const TOOL: &str = "nvim";

/// Prints the keymap as JSON; see the comments in the file.
const SCRIPT: &str = include_str!("nvim.lua");

/// Neovim's modes, in `nvim_get_keymap` letters, with the mode keydibs files
/// them under. Normal mode is where Neovim starts, so it's no mode.
const MODES: &[(char, Option<&str>)] = &[
    ('n', None),
    ('i', Some("insert")),
    ('x', Some("visual")),
    ('s', Some("select")),
    ('o', Some("operator-pending")),
    ('c', Some("command-line")),
    ('t', Some("terminal")),
];

/// The sections of `:help index` keydibs reads, by the tag in their heading.
/// Text objects are skipped: they're all plain keys.
const SECTIONS: &[(&str, char)] = &[
    ("*insert-index*", 'i'),
    ("*normal-index*", 'n'),
    ("*CTRL-W*", 'n'),
    ("*[*", 'n'),
    ("*g*", 'n'),
    ("*z*", 'n'),
    ("*operator-pending-index*", 'o'),
    ("*visual-index*", 'x'),
    ("*ex-edit-index*", 'c'),
    ("*terminal-mode-index*", 't'),
];

#[derive(Default)]
pub struct Nvim {
    /// Set by `load` when `doc/index.txt` is missing.
    no_index: Cell<bool>,
}

impl Adapter for Nvim {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn layer(&self) -> Layer {
        Layer::App
    }

    fn where_to_change(&self) -> &'static str {
        "vim.keymap.set in ~/.config/nvim/init.lua"
    }

    /// Neovim 0.10 and later turn it on in terminals that support it.
    fn kitty_keyboard(&self) -> bool {
        true
    }

    fn notes(&self) -> Vec<String> {
        if self.no_index.get() {
            vec![
                "Neovim's help files aren't installed, so its built-in commands aren't checked, only mappings".to_string(),
            ]
        } else {
            Vec::new()
        }
    }

    fn load(&self) -> anyhow::Result<Option<Vec<Binding>>> {
        if !on_path(TOOL) {
            return Ok(None);
        }
        // No swap file (`-n`) and no ShaDa (`-i NONE`), so nothing is written.
        let output = run(
            Command::new(TOOL)
                .args(["--headless", "-n", "-i", "NONE", "-V1", "-c"])
                .arg("lua assert(load(vim.env.KEYDIBS_NVIM_SCRIPT))()")
                .env("KEYDIBS_NVIM_SCRIPT", SCRIPT),
            COMMAND_TIMEOUT,
        )
        .context("running nvim")?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let Some(dump) = parse(&stdout)? else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            match stderr.lines().find(|line| !line.trim().is_empty()) {
                Some(error) => bail!("nvim didn't list its keymap: {}", error.trim()),
                None => bail!("nvim didn't list its keymap ({})", output.status),
            }
        };
        self.no_index.set(dump.index.is_none());
        Ok(Some(bindings(&dump)))
    }
}

/// What `nvim.lua` prints.
#[derive(Debug, Deserialize)]
pub struct Dump {
    /// `$VIMRUNTIME`, where Neovim's own files are.
    runtime: String,
    maps: Vec<Map>,
    /// The text of `:help index`, if it's installed.
    index: Option<String>,
}

/// One mapping in one mode.
#[derive(Debug, Deserialize)]
struct Map {
    mode: String,
    /// In key notation, like `<C-W>d` or `<Space>ff`.
    lhs: String,
    desc: Option<String>,
    rhs: Option<String>,
    /// The file that made the mapping.
    script: Option<String>,
    #[serde(default)]
    exists: bool,
    line: Option<usize>,
}

/// Picks the output of `nvim.lua` out of whatever else Neovim printed.
/// `None` if it isn't there.
pub fn parse(output: &str) -> anyhow::Result<Option<Dump>> {
    let Some(json) = output.lines().find_map(|l| l.strip_prefix("KEYDIBS ")) else {
        return Ok(None);
    };
    Ok(Some(
        serde_json::from_str(json).context("reading nvim's keymap")?,
    ))
}

/// One binding before it's turned into a [`Binding`].
#[derive(Debug)]
struct Entry {
    mode: char,
    keys: Vec<Chord>,
    action: String,
    source: Source,
}

/// Neovim's effective keymap: the built-in commands, with mappings on top.
pub fn bindings(dump: &Dump) -> Vec<Binding> {
    let mut entries = dump.index.as_deref().map(builtins).unwrap_or_default();
    for map in &dump.maps {
        let (Some(mode), Some(keys)) = (map.mode.chars().next(), parse_keys(&map.lhs)) else {
            continue;
        };
        // A mapping replaces the built-in command on the same keys.
        let replaced = entries
            .iter()
            .position(|e| e.mode == mode && e.keys == keys)
            .map(|i| entries.remove(i));
        // Neovim's own tweaks of built-in commands are described as
        // `:help i_CTRL-W-default`; what the command does says more.
        let action = match replaced {
            Some(builtin) if map.desc.as_deref().is_some_and(|d| d.starts_with(":help ")) => {
                builtin.action
            }
            _ => map_action(map),
        };
        entries.push(Entry {
            mode,
            keys,
            action,
            source: map_source(map, &dump.runtime),
        });
    }

    let mut bindings: Vec<Binding> = Vec::new();
    for entry in entries {
        let Some(&(_, mode)) = MODES.iter().find(|(letter, _)| *letter == entry.mode) else {
            continue;
        };
        let mut keys = entry.keys.into_iter();
        let Some(chord) = keys.next() else {
            continue;
        };
        let binding = Binding {
            chord,
            action: shorten(&entry.action),
            tool: TOOL,
            layer: Layer::App,
            grabs: Grabs::Always,
            mode: mode.map(str::to_string),
            then: keys.collect(),
            source: entry.source,
        };

        // The same keys often do the same thing in several modes: a mapping made
        // for several at once (`:map`, `:vmap`) is listed once per mode, and so
        // are many built-in commands. Outside Normal mode, keep one binding.
        if let Some(mode) = mode
            && let Some(same) = bindings.iter_mut().find(|b| {
                b.mode.is_some()
                    && b.chord == binding.chord
                    && b.then == binding.then
                    && b.action == binding.action
                    && b.source == binding.source
            })
            && let Some(modes) = &mut same.mode
        {
            modes.push('/');
            modes.push_str(mode);
            continue;
        }
        bindings.push(binding);
    }
    bindings
}

fn map_action(map: &Map) -> String {
    [&map.desc, &map.rhs]
        .into_iter()
        .flatten()
        .find(|text| !text.is_empty())
        .cloned()
        .unwrap_or_else(|| "Lua function".to_string())
}

/// Neovim's own mappings (from `$VIMRUNTIME`, or compiled in, so with no file)
/// are defaults; the rest point to the file and line that made them.
fn map_source(map: &Map, runtime: &str) -> Source {
    match &map.script {
        Some(script)
            if map.exists && (runtime.is_empty() || !Path::new(script).starts_with(runtime)) =>
        {
            Source::Config {
                path: PathBuf::from(script),
                line: map.line.filter(|&line| line > 0),
            }
        }
        Some(_) => Source::Default,
        None => Source::Runtime,
    }
}

/// Vim's built-in commands on a modifier or a named key, from `:help index`.
fn builtins(index: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut section = None;
    let mut lines = index.lines().peekable();
    while let Some(line) = lines.next() {
        if line.starts_with("=====") {
            let heading = lines.next().unwrap_or("");
            section = SECTIONS
                .iter()
                .find(|(tag, _)| heading.contains(tag))
                .map(|&(_, mode)| mode);
            continue;
        }
        // Entries look like `|i_CTRL-R|	CTRL-R {register}	insert the contents…`,
        // with long descriptions continued on lines indented to the last column.
        let Some(mode) = section else { continue };
        let Some((tag, rest)) = line.strip_prefix('|').and_then(|l| l.split_once('|')) else {
            continue;
        };
        let rest = rest.trim_start();
        let (column, description) = rest
            .split_once('\t')
            .unwrap_or_else(|| after_placeholders(rest));
        let mut description = description.trim().to_string();
        while let Some(more) = lines.next_if(|l| l.starts_with("\t\t\t") && !l.trim().is_empty()) {
            description.push(' ');
            description.push_str(more.trim());
        }
        // The completion menu's keys are listed under Insert mode, but only work
        // while the menu is open.
        if tag.starts_with("complete_") {
            continue;
        }
        let Some(keys) = parse_keys(&index_notation(column)) else {
            continue;
        };
        if keys.is_empty() || entries.iter().any(|e| e.mode == mode && e.keys == keys) {
            continue;
        }
        entries.push(Entry {
            mode,
            keys,
            // Without the bars around links to other help topics.
            action: without_note(description.trim()).replace('|', ""),
            source: Source::Default,
        });
    }

    resolve_aliases(&mut entries);

    // Characters, with or without Shift, are typed text everywhere else.
    entries.retain(|e| {
        let first = &e.keys[0];
        first.key.as_str().chars().count() > 1
            || first.mods.intersects(Mods::CTRL | Mods::ALT | Mods::SUPER)
    });
    let prefixes: Vec<(char, Vec<Chord>)> =
        entries.iter().map(|e| (e.mode, e.keys.clone())).collect();
    entries.retain(|e| {
        !prefixes.iter().any(|(mode, keys)| {
            *mode == e.mode && keys.len() < e.keys.len() && e.keys.starts_with(keys)
        })
    });
    entries
}

/// Splits keys and description where no tab separates them, as in
/// `CTRL-V {number} insert three digit…`: the description follows the placeholders.
fn after_placeholders(line: &str) -> (&str, &str) {
    let mut placeholders = false;
    let mut start = 0;
    for word in line.split(' ') {
        let placeholder = word.contains('{');
        if placeholders && !placeholder && !word.is_empty() {
            return (&line[..start], &line[start..]);
        }
        placeholders |= placeholder;
        start += word.len() + 1;
    }
    (line, "")
}

/// Drops the Normal and Visual mode note (`1` for motions, `2` for undoable
/// commands) from the start of a description.
fn without_note(description: &str) -> &str {
    match description.split_once(char::is_whitespace) {
        Some((note, rest))
            if !note.is_empty() && note.chars().all(|c| matches!(c, '1' | '2' | ',')) =>
        {
            rest.trim_start()
        }
        _ => description,
    }
}

/// Replaces descriptions like `same as <Tab>` with what that key does, so that
/// two keys that do the same thing say so.
fn resolve_aliases(entries: &mut [Entry]) {
    // A few hops at most: `<S-Down>` is the same as `CTRL-F`, which may itself be an alias.
    for _ in 0..3 {
        let mut changes = Vec::new();
        for (i, entry) in entries.iter().enumerate() {
            let Some(target) = alias_of(&entry.action) else {
                continue;
            };
            if let Some(other) = entries
                .iter()
                .find(|o| o.mode == entry.mode && o.keys == target && alias_of(&o.action).is_none())
            {
                changes.push((i, other.action.clone()));
            }
        }
        if changes.is_empty() {
            break;
        }
        for (i, action) in changes {
            entries[i].action = action;
        }
    }
}

/// The keys in `same as CTRL-F`, `same as "CTRL-W b"` or ``same as `g<Tab>` ``.
fn alias_of(description: &str) -> Option<Vec<Chord>> {
    let rest = description.strip_prefix("same as ")?;
    let keys = match rest.chars().next()? {
        quote @ ('"' | '`') => rest[1..].split(quote).next()?,
        _ => rest
            .split_whitespace()
            .next()?
            .trim_end_matches([',', ';', '.']),
    };
    parse_keys(&index_notation(keys))
}

/// Rewrites keys as `:help index` writes them (`CTRL-W CTRL-B`, `CTRL-V {char}`)
/// in the notation mappings use (`<C-W><C-B>`), up to the first placeholder.
fn index_notation(column: &str) -> String {
    let mut notation = String::new();
    for word in column.split(' ').filter(|w| !w.is_empty()) {
        // `{char}`, `{motion}` and `["x]` stand for whatever comes next.
        let placeholder = [word.find('{'), word.find("[\"")]
            .into_iter()
            .flatten()
            .min();
        let word = &word[..placeholder.unwrap_or(word.len())];
        match word.strip_prefix("CTRL-") {
            Some(key) => {
                let (shift, key) = match key.strip_prefix("SHIFT-") {
                    Some(key) => ("S-", key),
                    None => ("", key),
                };
                let key = key
                    .strip_prefix('<')
                    .and_then(|k| k.strip_suffix('>'))
                    .unwrap_or(key);
                notation.push_str(&format!("<C-{shift}{key}>"));
            }
            None => notation.push_str(word),
        }
        if placeholder.is_some() {
            break;
        }
    }
    notation
}

/// Parses keys in Vim's notation, like `<C-W>d`, `<Space>ff` or `<M-Up>`.
/// `None` if any of them is a key keydibs doesn't model.
fn parse_keys(notation: &str) -> Option<Vec<Chord>> {
    let mut keys = Vec::new();
    let mut rest = notation;
    while let Some(c) = rest.chars().next() {
        let special = rest
            .strip_prefix('<')
            .filter(|r| r.contains('>'))
            .map(|inner| {
                // `<C->>` is Ctrl and `>`, so the name can end in `>` itself.
                [inner.find('>'), inner.find(">>").map(|i| i + 1)]
                    .into_iter()
                    .flatten()
                    .find_map(|end| Some((parse_special(&inner[..end])?, end + 2)))
            });
        match special {
            Some(Some((chord, len))) => {
                keys.push(chord);
                rest = &rest[len..];
            }
            // Something in angle brackets that isn't a key: `<Plug>`, `<LeftMouse>`…
            Some(None) => return None,
            None => {
                keys.push(character(c, Mods::empty())?);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    Some(keys)
}

/// One key in angle brackets, without them: `C-S-c`, `M-Up`, `F5`, `lt`.
fn parse_special(name: &str) -> Option<Chord> {
    let mut mods = Mods::empty();
    let mut rest = name;
    // Modifiers are a letter and a dash. The key itself can be a dash: `C--`.
    while rest.len() > 2 && rest.as_bytes()[1] == b'-' {
        mods |= match rest.as_bytes()[0].to_ascii_lowercase() {
            b'c' => Mods::CTRL,
            b's' => Mods::SHIFT,
            b'm' | b'a' => Mods::ALT,
            b'd' => Mods::SUPER,
            _ => return None,
        };
        rest = &rest[2..];
    }

    let lower = rest.to_ascii_lowercase();
    let key = match lower.as_str() {
        "cr" | "return" | "enter" => "enter",
        "esc" => "escape",
        "bs" => "backspace",
        "del" => "delete",
        "lt" => "<",
        "bar" => "|",
        "bslash" => "\\",
        "tab" | "space" | "up" | "down" | "left" | "right" | "home" | "end" | "pageup"
        | "pagedown" | "insert" | "help" | "undo" => lower.as_str(),
        name if is_function_key(name) => name,
        _ => {
            let mut chars = rest.chars();
            return match (chars.next(), chars.next()) {
                (Some(c), None) => character(c, mods),
                _ => None,
            };
        }
    };
    Some(Chord {
        mods,
        key: Key::parse(key)?,
    })
}

/// A key that types `c`, with `mods` held. An uppercase letter means Shift,
/// except with Ctrl, where Vim ignores case (`<C-A>` is `<C-a>`).
fn character(c: char, mut mods: Mods) -> Option<Chord> {
    if c.is_ascii_uppercase() && !mods.contains(Mods::CTRL) {
        mods |= Mods::SHIFT;
    }
    // Shift on a symbol (`<S-+>`) is a keypad key, or meaningless.
    if mods.contains(Mods::SHIFT) && !c.is_ascii_alphabetic() {
        return None;
    }
    Some(Chord {
        mods,
        key: Key::parse(&c.to_ascii_lowercase().to_string())?,
    })
}

fn is_function_key(name: &str) -> bool {
    name.strip_prefix('f')
        .is_some_and(|n| !n.is_empty() && n.len() <= 2 && n.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(s: &str) -> Chord {
        s.parse().unwrap()
    }

    fn keys(notation: &str) -> Option<String> {
        parse_keys(notation).map(|keys| {
            keys.iter()
                .map(Chord::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        })
    }

    fn find<'a>(bs: &'a [Binding], keys: &str, mode: Option<&str>) -> &'a Binding {
        bs.iter()
            .find(|b| b.keys() == keys && b.mode.as_deref() == mode)
            .unwrap_or_else(|| panic!("no {keys} binding in {mode:?}"))
    }

    #[test]
    fn key_notation() {
        assert_eq!(keys("<C-S-C>").as_deref(), Some("Ctrl+Shift+C"));
        assert_eq!(keys("<C-c>").as_deref(), Some("Ctrl+C"));
        assert_eq!(keys("<M-Up>").as_deref(), Some("Alt+Up"));
        assert_eq!(keys("<A-j>").as_deref(), Some("Alt+J"));
        assert_eq!(keys("<M-J>").as_deref(), Some("Alt+Shift+J"));
        assert_eq!(keys("<D-s>").as_deref(), Some("Super+S"));
        assert_eq!(keys("<Space>ff").as_deref(), Some("Space, F, F"));
        assert_eq!(keys("Y").as_deref(), Some("Shift+Y"));
        assert_eq!(keys("[<C-T>").as_deref(), Some("[, Ctrl+T"));
        assert_eq!(keys("<C-W>d").as_deref(), Some("Ctrl+W, D"));
        assert_eq!(keys("<lt>").as_deref(), Some("<"));
        assert_eq!(keys("<C->>").as_deref(), Some("Ctrl+>"));
        assert_eq!(keys("<C-->").as_deref(), Some("Ctrl+-"));
        assert_eq!(keys("<C-]>").as_deref(), Some("Ctrl+]"));
        assert_eq!(keys("<S-Tab>").as_deref(), Some("Shift+Tab"));
        assert_eq!(keys("<CR>").as_deref(), Some("Enter"));
        assert_eq!(keys("<F12>").as_deref(), Some("F12"));
        assert_eq!(keys("<Plug>(MatchitNormalForward)"), None);
        assert_eq!(keys("<LeftMouse>"), None);
        assert_eq!(keys("<S-+>"), None);
    }

    #[test]
    fn index_notation_becomes_key_notation() {
        assert_eq!(index_notation("CTRL-W CTRL-B"), "<C-W><C-B>");
        assert_eq!(index_notation("CTRL-SHIFT-V {char}"), "<C-S-V>");
        assert_eq!(index_notation("CTRL-<Tab>"), "<C-Tab>");
        assert_eq!(index_notation("CTRL-\\ CTRL-N"), "<C-\\><C-N>");
        assert_eq!(index_notation("CTRL-W g CTRL-]"), "<C-W>g<C-]>");
        assert_eq!(index_notation("<S-Left>"), "<S-Left>");
        assert_eq!(index_notation("[\"x]p"), "");
    }

    /// A trimmed copy of `:help index` from Neovim 0.12.
    const INDEX: &str = "\
==============================================================================
1. Insert mode\t\t\t\t\t\t*insert-index*

Tag\t\tChar\t\tInsert-mode action\t~
------------------------------------------------------------------------------ ~
|i_CTRL-@|\tCTRL-@\t\tinsert previously inserted text and stop
\t\t\t\tinsert
|i_CTRL-E|\tCTRL-E\t\tinsert the character which is below the cursor
\t\tCTRL-F\t\tnot used (but by default it's in 'cinkeys' to
\t\t\t\tre-indent the current line)
|i_CTRL-G_j|\tCTRL-G CTRL-J\tline down, to column where inserting started
|i_<BS>|\t<BS>\t\tdelete character before the cursor
|i_CTRL-H|\tCTRL-H\t\tsame as <BS>
|i_CTRL-R|\tCTRL-R {register}
\t\t\t\tinsert the contents of a register
|i_CTRL-R_CTRL-R| CTRL-R CTRL-R {register}
\t\t\t\tinsert the contents of a register literally
|i_CTRL-SHIFT-V|  CTRL-SHIFT-V {char}
\t\t\t\tlike CTRL-V unless |tui-modifyOtherKeys| is active
|i_CTRL-V|\tCTRL-V {char}\tinsert next non-digit literally
|i_CTRL-V_digit| CTRL-V {number} insert three digit decimal number as a single
\t\t\t\tbyte.
|i_CTRL-W|\tCTRL-W\t\tdelete word before the cursor
|i_<S-Left>|\t<S-Left>\tcursor one word left
|i_<LeftMouse>|\t<LeftMouse>\tcursor at mouse click

commands in completion mode (see |popupmenu-keys|)

|complete_CTRL-E| CTRL-E\tstop completion and go back to original text

==============================================================================
2. Normal mode\t\t\t\t\t\t*normal-index*

Tag\t\tChar\t      Note Normal-mode action\t~
------------------------------------------------------------------------------ ~
|CTRL-A|\tCTRL-A\t\t2  add N to number at/after cursor
|CTRL-F|\tCTRL-F\t\t1  scroll N screens Forward
|CTRL-L|\tCTRL-L\t\t   redraw screen
|CTRL-U|\tCTRL-U\t\t   scroll N lines Upwards (default: half a
\t\t\t\t   screen)
|CTRL-W|\tCTRL-W {char}\t   window commands, see |CTRL-W|
|<S-Down>|\t<S-Down>\t1  same as CTRL-F
|<S-Left>|\t<S-Left>\t1  same as \"b\"
|a|\t\ta\t\t2  append text after the cursor N times
|A|\t\tA\t\t2  append text after the end of the line N times

==============================================================================
2.1 Text objects\t\t\t\t\t\t*objects*

|v_aw|\t\taw\t\t   \"a word\" (with white space)

==============================================================================
2.2 Window commands\t\t\t\t\t\t*CTRL-W*

|CTRL-W_CTRL-B|\tCTRL-W CTRL-B\t   same as \"CTRL-W b\"
|CTRL-W_+|\tCTRL-W +\t   increase current window height N lines

==============================================================================
4. Command-line editing\t\t\t\t\t*ex-edit-index*

|c_<S-Left>|\t<S-Left>\tcursor one word left
|c_CTRL-\\_e|\tCTRL-\\ e {expr} replace the command line with the result of
\t\t\t\t{expr}

==============================================================================
6. EX commands\t\t\t\t*Ex-command* *ex-cmd-index* *:index*

|:|\t\t:\t\tnothing
";

    fn dump(maps: &str) -> Dump {
        let json = format!(
            r#"KEYDIBS {{"runtime":"/usr/share/nvim/runtime","index":{},"maps":{}}}"#,
            serde_json::to_string(INDEX).unwrap(),
            maps.replace('\n', " ")
        );
        parse(&format!("some message\n{json}\n")).unwrap().unwrap()
    }

    #[test]
    fn builtin_commands_come_from_the_index() {
        let bs = bindings(&dump("[]"));
        let action = |keys: &str, mode: Option<&str>| find(&bs, keys, mode).action.clone();

        assert_eq!(action("Ctrl+A", None), "add N to number at/after cursor");
        assert_eq!(action("Ctrl+L", None), "redraw screen");
        assert_eq!(
            action("Ctrl+U", None),
            "scroll N lines Upwards (default: half a screen)"
        );
        assert_eq!(
            action("Ctrl+@", Some("insert")),
            "insert previously inserted text and stop insert"
        );
        assert_eq!(
            action("Ctrl+Shift+V", Some("insert")),
            "like CTRL-V unless tui-modifyOtherKeys is active"
        );
        assert_eq!(
            action("Ctrl+V", Some("insert")),
            "insert next non-digit literally"
        );
        assert_eq!(
            action("Ctrl+G, Ctrl+J", Some("insert")),
            "line down, to column where inserting started"
        );
        assert_eq!(find(&bs, "Ctrl+E", Some("insert")).source, Source::Default);

        assert_eq!(
            action("Ctrl+\\, E", Some("command-line")),
            "replace the command line with the result of {expr}"
        );

        // Aliases say what the key they stand for does.
        assert_eq!(
            action("Ctrl+H", Some("insert")),
            "delete character before the cursor"
        );
        assert_eq!(action("Shift+Down", None), "scroll N screens Forward");

        // The same action in two modes is one binding.
        assert_eq!(
            find(&bs, "Shift+Left", Some("insert/command-line")).action,
            "cursor one word left"
        );

        let all: Vec<String> = bs.iter().map(Binding::keys).collect();
        // Only the first key of a family of commands.
        assert!(all.contains(&"Ctrl+W".to_string()));
        assert!(!all.iter().any(|k| k.starts_with("Ctrl+W, ")));
        assert!(!all.iter().any(|k| k.starts_with("Ctrl+R, ")));
        // Plain keys, mouse keys, text objects, the completion menu and Ex
        // commands are left out.
        for left_out in ["A", "Shift+A", "A, W", ":"] {
            assert!(!all.contains(&left_out.to_string()), "{left_out}");
        }
        assert!(!all.iter().any(|k| k.contains("Mouse")));
        assert_eq!(
            bs.iter()
                .filter(|b| b.keys() == "Ctrl+E" && b.mode.as_deref() == Some("insert"))
                .count(),
            1
        );
    }

    #[test]
    fn mappings_replace_builtins_and_point_to_their_file() {
        let bs = bindings(&dump(
            r#"[
            {"mode":"i","lhs":"<C-W>","rhs":"<C-G>u<C-W>","desc":":help i_CTRL-W-default",
             "script":"/home/me/vim/_core/defaults","exists":false,"line":114},
            {"mode":"n","lhs":"<C-L>","rhs":"<Cmd>nohlsearch<CR>",
             "script":"/usr/share/nvim/runtime/lua/vim/_defaults.lua","exists":true,"line":106},
            {"mode":"n","lhs":"<C-S>","rhs":":w<CR>",
             "script":"/home/me/.config/nvim/init.lua","exists":true,"line":5},
            {"mode":"n","lhs":"<Space>ff","desc":"Find files",
             "script":"/home/me/.config/nvim/init.lua","exists":true,"line":2},
            {"mode":"i","lhs":"<M-j>","script":"/home/me/.config/nvim/init.lua","exists":true,"line":0},
            {"mode":"n","lhs":"<Plug>(MatchitNormalForward)","rhs":":<C-U>call matchit#Match_wrapper()<CR>"}
        ]"#,
        ));

        let ctrl_w = find(&bs, "Ctrl+W", Some("insert"));
        assert_eq!(ctrl_w.action, "delete word before the cursor");
        assert_eq!(ctrl_w.source, Source::Default);
        assert_eq!(
            bs.iter()
                .filter(|b| b.keys() == "Ctrl+W" && b.mode.as_deref() == Some("insert"))
                .count(),
            1
        );
        assert_eq!(find(&bs, "Ctrl+L", None).source, Source::Default);

        let save = find(&bs, "Ctrl+S", None);
        assert_eq!(save.action, ":w<CR>");
        assert_eq!(
            save.source,
            Source::Config {
                path: "/home/me/.config/nvim/init.lua".into(),
                line: Some(5)
            }
        );
        assert_eq!(find(&bs, "Space, F, F", None).action, "Find files");

        let lua = find(&bs, "Alt+J", Some("insert"));
        assert_eq!(lua.action, "Lua function");
        assert!(matches!(lua.source, Source::Config { line: None, .. }));

        assert!(!bs.iter().any(|b| b.action.contains("matchit")));
    }

    #[test]
    fn a_mapping_for_several_modes_is_listed_once_outside_normal_mode() {
        let map = |mode: &str| {
            format!(
                r#"{{"mode":"{mode}","lhs":"<F5>","rhs":":make<CR>","script":"/home/me/init.vim","exists":true,"line":6}}"#
            )
        };
        let maps: Vec<String> = ["n", "x", "s", "o"].into_iter().map(map).collect();
        let bs = bindings(&dump(&format!("[{}]", maps.join(","))));
        let f5: Vec<_> = bs.iter().filter(|b| b.chord == chord("F5")).collect();
        assert_eq!(f5.len(), 2);
        assert_eq!(f5[0].mode, None);
        assert_eq!(
            f5[1].mode.as_deref(),
            Some("visual/select/operator-pending")
        );
    }

    #[test]
    fn output_without_the_keymap() {
        assert!(
            parse("E5113: Error while calling lua chunk\n")
                .unwrap()
                .is_none()
        );
        assert!(parse("KEYDIBS {not json").is_err());
    }

    #[test]
    fn missing_index_still_gives_mappings() {
        let dump: Dump = serde_json::from_str(
            r#"{"runtime":"/rt","maps":[{"mode":"n","lhs":"<C-S>","rhs":":w<CR>"}]}"#,
        )
        .unwrap();
        let bs = bindings(&dump);
        assert_eq!(bs.len(), 1);
        assert_eq!(bs[0].source, Source::Runtime);
    }
}
