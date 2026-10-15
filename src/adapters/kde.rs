//! KDE Plasma global shortcuts, read from `~/.config/kglobalshortcutsrc`.
//!
//! Regular entries look like `action=active,default,description`. One action can
//! have several chords, separated by a literal `\t` (backslash, t), and `none`
//! means unbound. `[services][app.desktop]` sections hold app-launch shortcuts
//! as bare chords, e.g. `_launch=Ctrl+Alt+T`.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use anyhow::Context;

use super::{Adapter, config_home};
use crate::binding::{Binding, Grabs, Layer, Source};

const TOOL: &str = "kde";

pub struct Kde;

impl Adapter for Kde {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn layer(&self) -> Layer {
        Layer::Compositor
    }

    fn load(&self) -> anyhow::Result<Option<Vec<Binding>>> {
        let Some(path) = config_home().map(|dir| dir.join("kglobalshortcutsrc")) else {
            return Ok(None);
        };
        match fs::read_to_string(&path) {
            Ok(text) => Ok(Some(parse(&text, &path))),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }
}

pub fn parse(text: &str, path: &Path) -> Vec<Binding> {
    // A section's display name can come after its entries, so collect them first.
    let mut friendly_names = HashMap::new();
    let mut section = "";
    for line in text.lines() {
        if let Some(header) = section_header(line) {
            section = header;
        } else if let Some(name) = line.trim().strip_prefix("_k_friendly_name=") {
            friendly_names.insert(section, name);
        }
    }

    let mut bindings = Vec::new();
    let mut section = "";
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if let Some(header) = section_header(line) {
            section = header;
            continue;
        }
        if line.is_empty() || line.starts_with('#') || line.starts_with("_k_") {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        let (chords, action) = if let Some(app) = section.strip_prefix("services][") {
            let app = app.strip_suffix(".desktop").unwrap_or(app);
            let action = if key == "_launch" {
                format!("Launch {app}")
            } else {
                format!("{key} [{app}]")
            };
            (value, action)
        } else {
            let (active, description) = split_fields(value);
            let component = friendly_names.get(section).copied().unwrap_or(section);
            let what = if description.is_empty() {
                key
            } else {
                description
            };
            (active, format!("{what} [{component}]"))
        };

        for chord in chords
            .split("\\t")
            .map(str::trim)
            .filter(|c| !c.is_empty() && *c != "none")
        {
            // A chord we can't parse (say, an unknown modifier) is skipped rather than guessed at.
            let Ok(chord) = chord.parse() else { continue };
            bindings.push(Binding {
                chord,
                action: action.clone(),
                tool: TOOL,
                layer: Layer::Compositor,
                grabs: Grabs::Always,
                mode: None,
                then: Vec::new(),
                source: Source::Config {
                    path: path.to_path_buf(),
                    line: Some(index + 1),
                },
            });
        }
    }
    bindings
}

/// The text between the outer brackets of `[a]` or `[a][b]`.
fn section_header(line: &str) -> Option<&str> {
    line.trim().strip_prefix('[')?.strip_suffix(']')
}

/// Splits `active,default,description` into `(active, description)`.
/// A comma right after `+` is the comma key itself (`Ctrl+,`), not a separator,
/// and the description may contain commas of its own.
fn split_fields(value: &str) -> (&str, &str) {
    let mut separators = value
        .char_indices()
        .filter(|&(i, c)| c == ',' && !value[..i].ends_with('+'))
        .map(|(i, _)| i);
    let Some(first) = separators.next() else {
        return (value, "");
    };
    let description = separators.next().map_or("", |second| &value[second + 1..]);
    (&value[..first], description)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[KDE Keyboard Layout Switcher]
Switch to Next Keyboard Layout=Meta+Alt+K,Meta+Alt+K,
_k_friendly_name=Keyboard Layout Switcher

[ksmserver]
_k_friendly_name=Session Management
Lock Session=Screensaver\tMeta+L,Screensaver\tMeta+L,Lock Session
Reboot=none,none,Reboot

[kwin]
Zoom=Ctrl+,,Ctrl+,,Zoom in, a little
Window Close=Alt+F4,Alt+F4,Close Window

[services][org.kde.konsole.desktop]
_launch=Ctrl+Alt+T
"#;

    fn parsed() -> Vec<Binding> {
        parse(SAMPLE, Path::new("kglobalshortcutsrc"))
    }

    fn find<'a>(bindings: &'a [Binding], chord: &str) -> &'a Binding {
        let chord = chord.parse().unwrap();
        bindings.iter().find(|b| b.chord == chord).unwrap()
    }

    #[test]
    fn friendly_name_after_entries_is_used() {
        let bs = parsed();
        assert_eq!(
            find(&bs, "Meta+Alt+K").action,
            "Switch to Next Keyboard Layout [Keyboard Layout Switcher]"
        );
    }

    #[test]
    fn several_chords_per_action_and_none_is_skipped() {
        let bs = parsed();
        assert_eq!(
            find(&bs, "Meta+L").action,
            "Lock Session [Session Management]"
        );
        assert_eq!(
            find(&bs, "Screensaver").action,
            "Lock Session [Session Management]"
        );
        assert!(!bs.iter().any(|b| b.action.starts_with("Reboot")));
    }

    #[test]
    fn comma_key_and_comma_in_description() {
        let bs = parsed();
        assert_eq!(find(&bs, "Ctrl+comma").action, "Zoom in, a little [kwin]");
    }

    #[test]
    fn service_launchers() {
        let bs = parsed();
        assert_eq!(find(&bs, "Ctrl+Alt+T").action, "Launch org.kde.konsole");
    }

    #[test]
    fn line_numbers_point_at_the_entry() {
        let bs = parsed();
        assert_eq!(
            find(&bs, "Alt+F4").source,
            Source::Config {
                path: "kglobalshortcutsrc".into(),
                line: Some(13)
            }
        );
        assert_eq!(bs.len(), 6);
    }
}
