//! Konsole, from its built-in shortcuts plus the ones changed in its settings.
//!
//! Konsole compiles its default shortcuts in; changed ones are saved by KXMLGUI
//! as `<Action name="…" shortcut="…"/>` in `konsoleui.rc` and `sessionui.rc`
//! under `$XDG_DATA_HOME/kxmlgui5/konsole/`. A `shortcut` attribute replaces all
//! of an action's defaults, and an empty one removes them.
//!
//! Some actions are switched off unless there are several tabs or the view is
//! split. They still take the key then, so they're kept, with that condition in
//! the action's name. Find Next/Previous only work while the search bar is
//! open, so they live in a `search` mode.
//!
//! Not read: the quick-access shortcuts of the Quick Commands and SSH Manager
//! plugins when changed in the plugins' own settings, and KDE-wide overrides of
//! standard shortcuts in `kdeglobals`.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use anyhow::Context;

use super::{Adapter, data_home, on_path};
use crate::binding::{Binding, Grabs, Layer, Source};
use crate::chord::Chord;

const TOOL: &str = "konsole";

/// When an action is switched on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Needs {
    Nothing,
    SeveralTabs,
    SplitView,
    Searching,
}

/// Konsole 26.08's defaults on Linux, where its `ACCEL` modifier is Ctrl+Shift.
/// From `MainWindow.cpp`, `ViewManager.cpp`, `session/SessionController.cpp`,
/// `BookmarkMenu.cpp`, the plugins, and KDE Frameworks 6.31's standard shortcuts
/// for the actions Konsole doesn't override.
const DEFAULTS: &[(&str, &str, &[&str], Needs)] = &[
    // Main window
    ("new-tab", "New Tab", &["Ctrl+Shift+T"], Needs::Nothing),
    (
        "new-window",
        "New Window",
        &["Ctrl+Shift+N"],
        Needs::Nothing,
    ),
    (
        "close-window",
        "Close Window",
        &["Ctrl+Shift+Q"],
        Needs::Nothing,
    ),
    (
        "options_show_menubar",
        "Show Menubar",
        &["Ctrl+Shift+M"],
        Needs::Nothing,
    ),
    ("fullscreen", "Full Screen Mode", &["F11"], Needs::Nothing),
    (
        "options_configure_keybinding",
        "Configure Keyboard Shortcuts",
        &["Ctrl+Alt+,"],
        Needs::Nothing,
    ),
    (
        "options_configure",
        "Configure Konsole",
        &["Ctrl+Shift+,"],
        Needs::Nothing,
    ),
    (
        "activate-menu",
        "Activate Menu",
        &["Ctrl+Shift+F10"],
        Needs::Nothing,
    ),
    (
        "hamburger_menu",
        "Open Main Menu",
        &["Shift+F10"],
        Needs::Nothing,
    ),
    (
        "help_whats_this",
        "What's This?",
        &["Shift+F1"],
        Needs::Nothing,
    ),
    (
        "add_bookmark",
        "Add Bookmark",
        &["Ctrl+Shift+B"],
        Needs::Nothing,
    ),
    // Tabs and split views
    (
        "split-view-left-right",
        "Split View Left/Right",
        &["Ctrl+("],
        Needs::Nothing,
    ),
    (
        "split-view-top-bottom",
        "Split View Top/Bottom",
        &["Ctrl+)"],
        Needs::Nothing,
    ),
    (
        "split-view-auto",
        "Split View Automatically",
        &["Ctrl+*"],
        Needs::Nothing,
    ),
    (
        "expand-active-view",
        "Expand View",
        &["Ctrl+Shift+]"],
        Needs::SplitView,
    ),
    (
        "shrink-active-view",
        "Shrink View",
        &["Ctrl+Shift+["],
        Needs::SplitView,
    ),
    (
        "detach-view",
        "Detach Current View",
        &["Ctrl+Shift+H"],
        Needs::SplitView,
    ),
    (
        "next-tab",
        "Next Tab",
        &["Shift+Right", "Ctrl+PgDown"],
        Needs::SeveralTabs,
    ),
    (
        "previous-tab",
        "Previous Tab",
        &["Shift+Left", "Ctrl+PgUp"],
        Needs::SeveralTabs,
    ),
    (
        "focus-view-above",
        "Focus Above Terminal",
        &["Ctrl+Shift+Up"],
        Needs::SplitView,
    ),
    (
        "focus-view-below",
        "Focus Below Terminal",
        &["Ctrl+Shift+Down"],
        Needs::SplitView,
    ),
    (
        "focus-view-left",
        "Focus Left Terminal",
        &["Ctrl+Shift+Left"],
        Needs::SplitView,
    ),
    (
        "focus-view-right",
        "Focus Right Terminal",
        &["Ctrl+Shift+Right"],
        Needs::SplitView,
    ),
    (
        "focus-view-next",
        "Focus Next Terminal",
        &["Ctrl+F11"],
        Needs::SplitView,
    ),
    (
        "focus-view-prev",
        "Focus Previous Terminal",
        &["Ctrl+Shift+F11"],
        Needs::SplitView,
    ),
    (
        "last-used-tab",
        "Switch to Last Used Tab",
        &["Ctrl+Tab"],
        Needs::Nothing,
    ),
    (
        "last-used-tab-reverse",
        "Switch to Last Used Tab (Reverse)",
        &["Ctrl+Shift+Tab"],
        Needs::Nothing,
    ),
    (
        "toggle-maximize-current-view",
        "Toggle Maximize Current View",
        &["Ctrl+Shift+E"],
        Needs::SplitView,
    ),
    (
        "toggle-zoom-current-view",
        "Toggle Zoom-Maximize Current View",
        &["Ctrl+Shift+Z"],
        Needs::SplitView,
    ),
    (
        "move-tab-to-right",
        "Move Tab Right",
        &["Ctrl+Alt+Right"],
        Needs::SeveralTabs,
    ),
    (
        "move-tab-to-left",
        "Move Tab Left",
        &["Ctrl+Alt+Left"],
        Needs::SeveralTabs,
    ),
    (
        "semantic-setup-bash",
        "Set Up Semantic Integration (bash)",
        &["Ctrl+Alt+]"],
        Needs::Nothing,
    ),
    (
        "toggle-semantic-hints",
        "Toggle Semantic Hints Display",
        &["Ctrl+Alt+["],
        Needs::Nothing,
    ),
    (
        "toggle-line-numbers",
        "Toggle Line Numbers Display",
        &["Ctrl+Alt+\\"],
        Needs::Nothing,
    ),
    (
        "equal-size-view",
        "Equal Size to All Views",
        &["Ctrl+Shift+\\"],
        Needs::SplitView,
    ),
    (
        "switch-to-tab-0",
        "Switch to Tab 1",
        &["Alt+1"],
        Needs::SeveralTabs,
    ),
    (
        "switch-to-tab-1",
        "Switch to Tab 2",
        &["Alt+2"],
        Needs::SeveralTabs,
    ),
    (
        "switch-to-tab-2",
        "Switch to Tab 3",
        &["Alt+3"],
        Needs::SeveralTabs,
    ),
    (
        "switch-to-tab-3",
        "Switch to Tab 4",
        &["Alt+4"],
        Needs::SeveralTabs,
    ),
    (
        "switch-to-tab-4",
        "Switch to Tab 5",
        &["Alt+5"],
        Needs::SeveralTabs,
    ),
    (
        "switch-to-tab-5",
        "Switch to Tab 6",
        &["Alt+6"],
        Needs::SeveralTabs,
    ),
    (
        "switch-to-tab-6",
        "Switch to Tab 7",
        &["Alt+7"],
        Needs::SeveralTabs,
    ),
    (
        "switch-to-tab-7",
        "Switch to Tab 8",
        &["Alt+8"],
        Needs::SeveralTabs,
    ),
    (
        "switch-to-tab-8",
        "Switch to Tab 9",
        &["Alt+9"],
        Needs::SeveralTabs,
    ),
    (
        "switch-to-tab-9",
        "Switch to Tab 10",
        &["Alt+0"],
        Needs::SeveralTabs,
    ),
    // The terminal session
    (
        "close-session",
        "Close Session",
        &["Ctrl+Shift+W"],
        Needs::Nothing,
    ),
    (
        "edit_copy",
        "Copy",
        &["Ctrl+Shift+C", "Ctrl+Ins"],
        Needs::Nothing,
    ),
    (
        "edit_paste",
        "Paste",
        &["Ctrl+Shift+V", "Shift+Ins"],
        Needs::Nothing,
    ),
    (
        "paste-selection",
        "Paste Selection",
        &["Ctrl+Shift+Ins"],
        Needs::Nothing,
    ),
    (
        "select-mode",
        "Select Mode",
        &["Ctrl+Shift+D"],
        Needs::Nothing,
    ),
    (
        "file_save_as",
        "Save Output As",
        &["Ctrl+Shift+S"],
        Needs::Nothing,
    ),
    (
        "file_print",
        "Print Screen",
        &["Ctrl+Shift+P"],
        Needs::Nothing,
    ),
    (
        "clear-history-and-reset",
        "Clear Scrollback and Reset",
        &["Ctrl+Shift+K"],
        Needs::Nothing,
    ),
    (
        "next-profile",
        "Next Profile",
        &["Ctrl+Alt+N"],
        Needs::Nothing,
    ),
    (
        "prev-profile",
        "Previous Profile",
        &["Ctrl+Alt+M"],
        Needs::Nothing,
    ),
    ("edit_find", "Find", &["Ctrl+Shift+F"], Needs::Nothing),
    ("edit_find_next", "Find Next", &["F3"], Needs::Searching),
    (
        "edit_find_prev",
        "Find Previous",
        &["Shift+F3"],
        Needs::Searching,
    ),
    (
        "rename-session",
        "Configure or Rename Tab",
        &["Ctrl+Alt+S"],
        Needs::Nothing,
    ),
    (
        "copy-input-to-selected-tabs",
        "Copy Input to Selected Tabs",
        &["Ctrl+Shift+."],
        Needs::Nothing,
    ),
    (
        "copy-input-to-none",
        "Copy Input to None",
        &["Ctrl+Shift+/"],
        Needs::Nothing,
    ),
    (
        "zmodem-upload",
        "ZModem Upload",
        &["Ctrl+Alt+U"],
        Needs::Nothing,
    ),
    (
        "monitor-prompt",
        "Monitor for Prompt",
        &["Ctrl+Shift+R"],
        Needs::Nothing,
    ),
    (
        "monitor-activity",
        "Monitor for Activity",
        &["Ctrl+Shift+A"],
        Needs::Nothing,
    ),
    (
        "monitor-silence",
        "Monitor for Silence",
        &["Ctrl+Shift+I"],
        Needs::Nothing,
    ),
    (
        "enlarge-font",
        "Enlarge Font",
        &["Ctrl++", "Ctrl+="],
        Needs::Nothing,
    ),
    ("shrink-font", "Shrink Font", &["Ctrl+-"], Needs::Nothing),
    (
        "reset-font-size",
        "Reset Font Size",
        &["Ctrl+Alt+0"],
        Needs::Nothing,
    ),
    // Plugins
    (
        "quick-commands-show",
        "Show Quick Commands",
        &["Ctrl+Shift+F1"],
        Needs::Nothing,
    ),
    (
        "quick-commands-access",
        "Quick Commands: Quick Access",
        &["Ctrl+Alt+G"],
        Needs::Nothing,
    ),
    (
        "ssh-manager-show",
        "Show SSH Manager",
        &["Ctrl+Shift+F2"],
        Needs::Nothing,
    ),
    (
        "ssh-manager-access",
        "SSH Manager: Quick Access",
        &["Ctrl+Alt+H"],
        Needs::Nothing,
    ),
];

/// The KXMLGUI files Konsole saves changed shortcuts to.
const OVERRIDE_FILES: &[&str] = &["konsoleui.rc", "sessionui.rc"];

pub struct Konsole;

impl Adapter for Konsole {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn layer(&self) -> Layer {
        Layer::Terminal
    }

    fn load(&self) -> anyhow::Result<Option<Vec<Binding>>> {
        let dir = data_home().map(|dir| dir.join("kxmlgui5/konsole"));
        let mut overrides = Vec::new();
        for name in OVERRIDE_FILES {
            let Some(path) = dir.as_ref().map(|dir| dir.join(name)) else {
                continue;
            };
            match fs::read_to_string(&path) {
                Ok(text) => overrides.push((text, path)),
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
            }
        }
        if overrides.is_empty() && !on_path(TOOL) {
            return Ok(None);
        }
        let overrides: Vec<(&str, &Path)> = overrides
            .iter()
            .map(|(text, path)| (text.as_str(), path.as_path()))
            .collect();
        Ok(Some(bindings(&overrides)))
    }
}

/// One action and its current shortcuts.
struct Action {
    id: String,
    label: String,
    shortcuts: Vec<String>,
    needs: Needs,
    source: Source,
}

/// Konsole's effective shortcuts: the defaults with each override file (its
/// text and path) applied on top.
pub fn bindings(overrides: &[(&str, &Path)]) -> Vec<Binding> {
    let mut actions: Vec<Action> = DEFAULTS
        .iter()
        .map(|&(id, label, shortcuts, needs)| Action {
            id: id.to_string(),
            label: label.to_string(),
            shortcuts: shortcuts.iter().map(|s| s.to_string()).collect(),
            needs,
            source: Source::Default,
        })
        .collect();

    for (text, path) in overrides {
        for (id, shortcut, line) in action_overrides(text) {
            let source = Source::Config {
                path: path.to_path_buf(),
                line: Some(line),
            };
            let shortcuts = split_shortcuts(&shortcut);
            match actions.iter_mut().find(|a| a.id == id) {
                Some(action) => {
                    action.shortcuts = shortcuts;
                    action.source = source;
                }
                // An action without a default shortcut that was given one.
                None => actions.push(Action {
                    label: id.clone(),
                    id,
                    shortcuts,
                    needs: Needs::Nothing,
                    source,
                }),
            }
        }
    }

    actions
        .iter()
        .flat_map(|action| {
            action.shortcuts.iter().filter_map(move |shortcut| {
                let mut chords = parse_sequence(shortcut)?.into_iter();
                Some(Binding {
                    chord: chords.next()?,
                    action: label(action),
                    tool: TOOL,
                    layer: Layer::Terminal,
                    // Find Next/Previous only work while Konsole's search bar is open,
                    // which is its own mode: the key wasn't going to the program anyway.
                    grabs: if action.needs == Needs::Searching {
                        Grabs::Never
                    } else {
                        Grabs::Always
                    },
                    mode: (action.needs == Needs::Searching).then(|| "search".to_string()),
                    then: chords.collect(),
                    source: action.source.clone(),
                })
            })
        })
        .collect()
}

fn label(action: &Action) -> String {
    match action.needs {
        Needs::Nothing | Needs::Searching => action.label.clone(),
        Needs::SeveralTabs => format!("{} (with 2+ tabs)", action.label),
        Needs::SplitView => format!("{} (with split views)", action.label),
    }
}

/// Qt writes several shortcuts as `Ctrl+Shift+C; Ctrl+Ins`.
fn split_shortcuts(text: &str) -> Vec<String> {
    text.split("; ")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// A shortcut can be a sequence of up to four chords: `Ctrl+K, Ctrl+C`.
fn parse_sequence(shortcut: &str) -> Option<Vec<Chord>> {
    shortcut
        .split(", ")
        .map(|chord| chord.parse().ok())
        .collect()
}

/// `(action id, shortcut attribute, line)` for every `<Action>` element that
/// sets a shortcut. Menus in the same files list `<Action name="…"/>` without
/// one, and those are skipped.
fn action_overrides(xml: &str) -> Vec<(String, String, usize)> {
    let mut found = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<Action ") {
        let tag_start = xml.len() - rest.len() + start;
        let tag = &rest[start..];
        let end = tag.find('>').unwrap_or(tag.len());
        let tag = &tag[..end];
        if let (Some(name), Some(shortcut)) = (attribute(tag, "name"), attribute(tag, "shortcut")) {
            let line = xml[..tag_start].matches('\n').count() + 1;
            found.push((name, shortcut, line));
        }
        rest = &rest[start + end..];
    }
    found
}

/// The value of `name="…"` in a tag, with XML entities decoded.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let needle = format!(" {name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let len = tag[start..].find('"')?;
    Some(
        tag[start..start + len]
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&amp;", "&"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(s: &str) -> Chord {
        s.parse().unwrap()
    }

    fn find<'a>(bs: &'a [Binding], keys: &str) -> &'a Binding {
        bs.iter()
            .find(|b| b.keys() == keys)
            .unwrap_or_else(|| panic!("no {keys} binding"))
    }

    #[test]
    fn every_default_parses() {
        for (id, _, shortcuts, _) in DEFAULTS {
            for shortcut in *shortcuts {
                assert!(parse_sequence(shortcut).is_some(), "{id}: {shortcut}");
            }
        }
        let shortcut_count: usize = DEFAULTS.iter().map(|d| d.2.len()).sum();
        assert_eq!(bindings(&[]).len(), shortcut_count);
    }

    #[test]
    fn defaults_have_no_duplicates() {
        assert!(crate::conflict::find(&bindings(&[])).is_empty());
    }

    #[test]
    fn qt_key_names_match_other_tools() {
        let bs = bindings(&[]);
        assert_eq!(find(&bs, "Ctrl+Insert").action, "Copy");
        assert_eq!(find(&bs, "Ctrl+PageDown").action, "Next Tab (with 2+ tabs)");
        assert_eq!(find(&bs, "Ctrl++").action, "Enlarge Font");
        assert_eq!(find(&bs, "Ctrl+Alt+,").chord, chord("ctrl+alt+comma"));
    }

    #[test]
    fn conditional_actions_say_when_they_apply() {
        let bs = bindings(&[]);
        assert_eq!(find(&bs, "Shift+Right").action, "Next Tab (with 2+ tabs)");
        assert_eq!(
            find(&bs, "Ctrl+Shift+Up").action,
            "Focus Above Terminal (with split views)"
        );
        assert_eq!(find(&bs, "Ctrl+Shift+T").action, "New Tab");
    }

    #[test]
    fn overrides_replace_remove_and_add() {
        let xml = r#"<!DOCTYPE gui SYSTEM "kpartgui.dtd">
<gui name="konsole" version="1">
 <MenuBar>
  <Menu name="edit"><Action name="edit_copy"/></Menu>
 </MenuBar>
 <ActionProperties scheme="Default">
  <Action name="edit_copy" shortcut="Ctrl+Alt+C"/>
  <Action name="next-tab" shortcut=""/>
  <Action name="clone-tab" shortcut="Ctrl+Shift+Y; Ctrl+K, Ctrl+Y"/>
 </ActionProperties>
</gui>"#;
        let path = Path::new("sessionui.rc");
        let bs = bindings(&[(xml, path)]);

        let copy = find(&bs, "Ctrl+Alt+C");
        assert_eq!(copy.action, "Copy");
        assert_eq!(
            copy.source,
            Source::Config {
                path: path.into(),
                line: Some(7)
            }
        );
        assert!(!bs.iter().any(|b| b.keys() == "Ctrl+Shift+C"));
        assert!(!bs.iter().any(|b| b.action.starts_with("Next Tab")));
        assert_eq!(find(&bs, "Ctrl+Shift+Y").action, "clone-tab");
        assert_eq!(find(&bs, "Ctrl+K, Ctrl+Y").action, "clone-tab");
    }

    #[test]
    fn find_next_lives_in_search_mode() {
        let bs = bindings(&[]);
        let next = find(&bs, "F3");
        assert_eq!(next.describe(), "Find Next (search mode)");
        assert_eq!(next.grabs, Grabs::Never);
    }

    #[test]
    fn xml_entities_are_decoded() {
        let found = action_overrides(r#"<Action name="x" shortcut="Ctrl+&lt;"/>"#);
        assert_eq!(found, vec![("x".into(), "Ctrl+<".into(), 1)]);
    }
}
