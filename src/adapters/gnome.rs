//! GNOME, read with `gsettings`.
//!
//! GNOME keeps its shortcuts in GSettings: the window manager's and mutter's,
//! GNOME Shell's, and the settings daemon's media keys, which also list the
//! custom shortcuts added in Settings. `gsettings` prints the value of each,
//! and the same call with the in-memory backend prints the defaults, so
//! changed ones point to the dconf database they're saved in.
//!
//! Only read in a GNOME session: other desktops install the same schemas
//! without using them. Skipped: shortcuts on Hyper or other modifiers keydibs
//! doesn't model. `Above_Tab` (the key above Tab, whatever it types) is kept
//! as its own key, so it doesn't match anything else yet.

use std::path::Path;
use std::process::Command;

use super::{Adapter, COMMAND_TIMEOUT, config_home, on_path, run, shorten};
use crate::binding::{Binding, Grabs, Layer, Source};
use crate::chord::{Chord, Key, Mods};

const TOOL: &str = "gnome";

/// The schemas with GNOME's shortcuts, each a list of `name ['<Super>a', …]`.
const SCHEMAS: &[&str] = &[
    "org.gnome.desktop.wm.keybindings",
    "org.gnome.mutter.keybindings",
    "org.gnome.mutter.wayland.keybindings",
    "org.gnome.shell.keybindings",
    "org.gnome.settings-daemon.plugins.media-keys",
];

/// Custom shortcuts: one of these per path in media-keys' `custom-keybindings`.
const CUSTOM: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";

pub struct Gnome;

impl Adapter for Gnome {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn layer(&self) -> Layer {
        Layer::Compositor
    }

    fn load(&self) -> anyhow::Result<Option<Vec<Binding>>> {
        if !in_gnome() || !on_path("gsettings") {
            return Ok(None);
        }
        let mut current = String::new();
        let mut defaults = String::new();
        for schema in SCHEMAS {
            current.push_str(&gsettings(schema, false)?);
            defaults.push_str(&gsettings(schema, true)?);
        }
        let mut custom = Vec::new();
        for path in custom_paths(&current) {
            custom.push(gsettings(&format!("{CUSTOM}:{path}"), false)?);
        }
        let database = config_home().unwrap_or_default().join("dconf/user");
        Ok(Some(parse(&current, &defaults, &custom, &database)))
    }
}

/// Whether this is a GNOME session: `$XDG_CURRENT_DESKTOP` says so (`GNOME`,
/// `ubuntu:GNOME`), or, outside a desktop session, GNOME Shell is installed.
fn in_gnome() -> bool {
    match std::env::var("XDG_CURRENT_DESKTOP") {
        Ok(desktops) if !desktops.is_empty() => desktops
            .split(':')
            .any(|desktop| desktop.eq_ignore_ascii_case("gnome")),
        _ => on_path("gnome-shell"),
    }
}

/// `gsettings list-recursively <schema>`, with the in-memory backend when
/// `defaults`, so that nothing the user changed shows. Empty when this GNOME
/// doesn't have the schema.
fn gsettings(schema: &str, defaults: bool) -> anyhow::Result<String> {
    let mut command = Command::new("gsettings");
    command.args(["list-recursively", schema]);
    if defaults {
        command.env("GSETTINGS_BACKEND", "memory");
    }
    let output = run(&mut command, COMMAND_TIMEOUT)?;
    Ok(if output.status.success() {
        String::from_utf8_lossy(&output.stdout).into_owned()
    } else {
        String::new()
    })
}

/// The paths of the custom shortcuts, from media-keys' `custom-keybindings`.
fn custom_paths(listing: &str) -> Vec<String> {
    listing
        .lines()
        .filter_map(|line| line.strip_prefix(SCHEMAS[4]))
        .filter_map(|rest| rest.trim_start().strip_prefix("custom-keybindings "))
        .flat_map(strings)
        .collect()
}

/// Turns `gsettings` output into bindings: `current` and `defaults` for the
/// built-in schemas, and one listing per custom shortcut. Changed and custom
/// shortcuts point to `database`.
pub fn parse(current: &str, defaults: &str, custom: &[String], database: &Path) -> Vec<Binding> {
    let changed = Source::Config {
        path: database.to_path_buf(),
        line: None,
    };
    let mut bindings = Vec::new();
    for (schema, name, value) in current.lines().filter_map(split_line) {
        if name == "custom-keybindings" {
            continue;
        }
        let default = defaults
            .lines()
            .filter_map(split_line)
            .find(|&(s, n, _)| s == schema && n == name)
            .map(|(_, _, value)| value);
        let source = if default == Some(value) {
            Source::Default
        } else {
            changed.clone()
        };
        for chord in strings(value).iter().filter_map(|a| parse_accelerator(a)) {
            bindings.push(binding(chord, name, source.clone()));
        }
    }

    for listing in custom {
        let field = |field: &str| {
            listing
                .lines()
                .filter_map(split_line)
                .find(|&(_, name, _)| name == field)
                .and_then(|(_, _, value)| strings(value).into_iter().next())
                .unwrap_or_default()
        };
        let action = match (field("name"), field("command")) {
            (name, _) if !name.is_empty() => name,
            (_, command) => command,
        };
        if let Some(chord) = parse_accelerator(&field("binding")) {
            bindings.push(binding(chord, &action, changed.clone()));
        }
    }
    bindings
}

fn binding(chord: Chord, action: &str, source: Source) -> Binding {
    Binding {
        chord,
        action: shorten(action),
        tool: TOOL,
        layer: Layer::Compositor,
        grabs: Grabs::Always,
        mode: None,
        then: Vec::new(),
        source,
    }
}

/// `schema key value`, as `gsettings list-recursively` prints each line.
fn split_line(line: &str) -> Option<(&str, &str, &str)> {
    let mut parts = line.splitn(3, ' ');
    Some((parts.next()?, parts.next()?, parts.next()?))
}

/// The strings in a GVariant value as `gsettings` prints it: `['<Super>a', 'F1']`,
/// `'<Super>t'`, `@as []`. Numbers and booleans have none.
fn strings(value: &str) -> Vec<String> {
    let mut strings = Vec::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\'' && c != '"' {
            continue;
        }
        let mut string = String::new();
        loop {
            match chars.next() {
                Some('\\') => string.extend(chars.next()),
                Some(end) if end == c => break,
                Some(other) => string.push(other),
                None => return strings,
            }
        }
        strings.push(string);
    }
    strings
}

/// A GTK accelerator like `<Super><Shift>Page_Up` or `XF86AudioMute`. `None`
/// for an empty one, `disabled`, or one with a modifier keydibs doesn't model.
fn parse_accelerator(accelerator: &str) -> Option<Chord> {
    let mut mods = Mods::empty();
    let mut rest = accelerator;
    while let Some(after) = rest.strip_prefix('<') {
        let (name, key) = after.split_once('>')?;
        mods |= match name.to_ascii_lowercase().as_str() {
            "primary" | "control" | "ctrl" | "ctl" => Mods::CTRL,
            "shift" | "shft" => Mods::SHIFT,
            "alt" | "mod1" => Mods::ALT,
            "super" | "mod4" | "meta" => Mods::SUPER,
            _ => return None,
        };
        rest = key;
    }
    // Key names are keysym names: letters, digits and underscores.
    if rest.is_empty()
        || rest == "disabled"
        || !rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return None;
    }
    let key = match rest.to_ascii_lowercase().as_str() {
        "above_tab" => "above tab".to_string(),
        name => name.to_string(),
    };
    Some(Chord {
        mods,
        key: Key::parse(&key)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(s: &str) -> Chord {
        s.parse().unwrap()
    }

    #[test]
    fn accelerators() {
        let parsed = |a: &str| parse_accelerator(a).map(|c| c.to_string());
        assert_eq!(parsed("<Super>a").as_deref(), Some("Super+A"));
        assert_eq!(parsed("<Shift><Alt>F6").as_deref(), Some("Alt+Shift+F6"));
        assert_eq!(
            parsed("<Ctrl><Shift><Alt>R").as_deref(),
            Some("Ctrl+Alt+Shift+R")
        );
        assert_eq!(parsed("<Primary>q").as_deref(), Some("Ctrl+Q"));
        assert_eq!(
            parsed("<Super>Page_Up"),
            Some(chord("super+pageup").to_string())
        );
        assert_eq!(
            parsed("<Control><Alt>Escape").as_deref(),
            Some("Ctrl+Alt+Esc")
        );
        assert_eq!(parsed("<Alt>space").as_deref(), Some("Alt+Space"));
        assert_eq!(parsed("Print").as_deref(), Some("Print"));
        assert_eq!(parsed("<Alt>Above_Tab").as_deref(), Some("Alt+Above Tab"));
        assert_eq!(parsed(""), None);
        assert_eq!(parsed("disabled"), None);
        assert_eq!(parsed("<Hyper>x"), None);
        assert_eq!(
            parsed("/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/"),
            None
        );
    }

    #[test]
    fn gvariant_strings() {
        assert_eq!(
            strings("['<Super>v', '<Super>m']"),
            ["<Super>v", "<Super>m"]
        );
        assert_eq!(strings("'<Super>t'"), ["<Super>t"]);
        assert_eq!(strings("\"it's\""), ["it's"]);
        assert_eq!(strings("'a \\'quoted\\' b'"), ["a 'quoted' b"]);
        assert!(strings("@as []").is_empty());
        assert!(strings("6").is_empty());
    }

    /// Trimmed from `gsettings list-recursively` with GNOME 49's schemas.
    const DEFAULTS: &str = "\
org.gnome.desktop.wm.keybindings activate-window-menu ['<Alt>space']
org.gnome.desktop.wm.keybindings always-on-top @as []
org.gnome.desktop.wm.keybindings switch-to-workspace-left ['<Super>Page_Up', '<Super><Alt>Left', '<Control><Alt>Left']
org.gnome.shell.keybindings toggle-message-tray ['<Super>v', '<Super>m']
org.gnome.shell.keybindings show-screenshot-ui ['Print']
org.gnome.settings-daemon.plugins.media-keys calculator ['']
org.gnome.settings-daemon.plugins.media-keys custom-keybindings @as []
org.gnome.settings-daemon.plugins.media-keys volume-step 6
org.gnome.settings-daemon.plugins.media-keys home ['<Super>e']
";

    fn current() -> String {
        DEFAULTS
            .replace(
                "always-on-top @as []",
                "always-on-top ['<Super>t']",
            )
            .replace(
                "custom-keybindings @as []",
                "custom-keybindings ['/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/']",
            )
    }

    const CUSTOM_0: &str = "\
org.gnome.settings-daemon.plugins.media-keys.custom-keybinding binding '<Super>Return'
org.gnome.settings-daemon.plugins.media-keys.custom-keybinding command 'kgx'
org.gnome.settings-daemon.plugins.media-keys.custom-keybinding enable-in-lockscreen false
org.gnome.settings-daemon.plugins.media-keys.custom-keybinding name 'Terminal'
";

    fn parsed() -> Vec<Binding> {
        parse(
            &current(),
            DEFAULTS,
            &[CUSTOM_0.to_string()],
            Path::new("/home/me/.config/dconf/user"),
        )
    }

    fn find<'a>(bs: &'a [Binding], keys: &str) -> &'a Binding {
        bs.iter()
            .find(|b| b.keys() == keys)
            .unwrap_or_else(|| panic!("no {keys} binding"))
    }

    #[test]
    fn shortcuts_from_every_schema() {
        let bs = parsed();
        assert_eq!(find(&bs, "Alt+Space").action, "activate-window-menu");
        assert_eq!(find(&bs, "Alt+Space").source, Source::Default);
        assert_eq!(find(&bs, "Super+PageUp").action, "switch-to-workspace-left");
        assert_eq!(
            find(&bs, "Ctrl+Alt+Left").action,
            "switch-to-workspace-left"
        );
        assert_eq!(find(&bs, "Super+M").action, "toggle-message-tray");
        assert_eq!(find(&bs, "Print").action, "show-screenshot-ui");
        assert_eq!(find(&bs, "Super+E").layer, Layer::Compositor);
        assert_eq!(bs.len(), 10);
    }

    #[test]
    fn changed_and_custom_shortcuts_point_to_dconf() {
        let bs = parsed();
        let dconf = Source::Config {
            path: "/home/me/.config/dconf/user".into(),
            line: None,
        };
        assert_eq!(find(&bs, "Super+T").action, "always-on-top");
        assert_eq!(find(&bs, "Super+T").source, dconf);
        assert_eq!(find(&bs, "Super+Enter").action, "Terminal");
        assert_eq!(find(&bs, "Super+Enter").source, dconf);
    }

    #[test]
    fn custom_shortcut_paths() {
        assert_eq!(
            custom_paths(&current()),
            ["/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/"]
        );
    }
}
