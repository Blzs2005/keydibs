//! Hyprland, read from the running compositor with `hyprctl -j binds`.
//!
//! Since 0.55 Hyprland is configured in Lua, where binds can come from loops,
//! variables and functions, so reading the config file without running it would
//! miss or misread them. The live keymap is exact for any config format, so
//! keydibs asks for that. The catch is that Hyprland has to be running.
//!
//! Skipped: mouse binds, binds on raw keycodes (`code:28`), and `catchall`.

use std::process::{Command, Stdio};

use anyhow::{Context, bail};
use serde::Deserialize;

use super::{Adapter, on_path, shorten};
use crate::binding::{Binding, Grabs, Layer, Source};
use crate::chord::{Chord, Key, Mods};

const TOOL: &str = "hyprland";

/// Modifier bits in `modmask`, as wlroots/xkb define them.
const MOD_SHIFT: u32 = 1;
const MOD_CTRL: u32 = 1 << 2;
const MOD_ALT: u32 = 1 << 3;
const MOD_SUPER: u32 = 1 << 6;

pub struct Hyprland;

impl Adapter for Hyprland {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn layer(&self) -> Layer {
        Layer::Compositor
    }

    fn load(&self) -> anyhow::Result<Option<Vec<Binding>>> {
        if !on_path("hyprctl") {
            return Ok(None);
        }
        // Inside a Hyprland session hyprctl finds it through the environment.
        // Outside one (another desktop, a TTY, SSH) try the first instance, and
        // treat "none running" as Hyprland not being in use.
        let in_session = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some();
        let mut command = Command::new("hyprctl");
        command.arg("-j");
        if !in_session {
            command.args(["--instance", "0"]);
        }
        let output = command
            .arg("binds")
            .stdin(Stdio::null())
            .output()
            .context("running hyprctl")?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        match parse(&stdout) {
            Ok(bindings) if output.status.success() => Ok(Some(bindings)),
            _ if !in_session => Ok(None),
            _ => {
                // hyprctl reports errors as plain text, so show that rather than a JSON error.
                let stderr = String::from_utf8_lossy(&output.stderr);
                let message = stdout
                    .lines()
                    .chain(stderr.lines())
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .unwrap_or("no output");
                bail!("`hyprctl -j binds` failed ({}): {message}", output.status)
            }
        }
    }
}

/// One entry of `hyprctl -j binds`. Older Hyprland versions leave some fields
/// out, hence the defaults.
#[derive(Deserialize)]
#[serde(default)]
struct RawBind {
    mouse: bool,
    non_consuming: bool,
    modmask: u32,
    submap: String,
    /// Printed as a quoted string (`"true"`) by current Hyprland.
    submap_universal: serde_json::Value,
    key: String,
    keycode: u32,
    catch_all: bool,
    has_description: bool,
    description: String,
    dispatcher: String,
    arg: String,
}

impl Default for RawBind {
    fn default() -> Self {
        RawBind {
            mouse: false,
            non_consuming: false,
            modmask: 0,
            submap: String::new(),
            submap_universal: serde_json::Value::Bool(false),
            key: String::new(),
            keycode: 0,
            catch_all: false,
            has_description: false,
            description: String::new(),
            dispatcher: String::new(),
            arg: String::new(),
        }
    }
}

/// Turns the output of `hyprctl -j binds` into bindings.
pub fn parse(json: &str) -> anyhow::Result<Vec<Binding>> {
    let raw: Vec<RawBind> = serde_json::from_str(json)?;
    Ok(raw.into_iter().filter_map(convert).collect())
}

fn convert(raw: RawBind) -> Option<Binding> {
    if raw.mouse || raw.catch_all || raw.keycode != 0 || raw.key.starts_with("mouse") {
        return None;
    }
    let chord = Chord {
        mods: mods(raw.modmask)?,
        key: Key::parse(&raw.key)?,
    };

    let universal = match &raw.submap_universal {
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::String(s) => s == "true",
        _ => false,
    };
    // A submap is Hyprland's own mode: its binds only exist while you're in it.
    let mode = (!raw.submap.is_empty() && !universal).then_some(raw.submap);
    let grabs = if raw.non_consuming || mode.is_some() {
        Grabs::Never
    } else {
        Grabs::Always
    };

    let action = if raw.has_description && !raw.description.is_empty() {
        raw.description
    } else {
        let command = format!("{} {}", raw.dispatcher, raw.arg);
        match command.trim() {
            "" => "Lua function".to_string(),
            command => command.to_string(),
        }
    };

    Some(Binding {
        chord,
        action: shorten(&action),
        tool: TOOL,
        layer: Layer::Compositor,
        grabs,
        mode,
        then: Vec::new(),
        source: Source::Runtime,
    })
}

/// `None` for modifiers keydibs can't represent yet (Caps Lock, Num Lock, Mod3, Mod5).
fn mods(mask: u32) -> Option<Mods> {
    let mut mods = Mods::empty();
    for (bit, flag) in [
        (MOD_SHIFT, Mods::SHIFT),
        (MOD_CTRL, Mods::CTRL),
        (MOD_ALT, Mods::ALT),
        (MOD_SUPER, Mods::SUPER),
    ] {
        if mask & bit != 0 {
            mods |= flag;
        }
    }
    let known = MOD_SHIFT | MOD_CTRL | MOD_ALT | MOD_SUPER;
    (mask & !known == 0).then_some(mods)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped like Hyprland 0.56's output, including its quirks: hand-formatted
    /// JSON and `submap_universal` as a string.
    const SAMPLE: &str = r#"[
{
    "locked": false, "mouse": false, "release": false, "repeat": false, "longPress": false,
    "non_consuming": false, "auto_consuming": false, "has_description": false,
    "modmask": 64, "submap": "", "submap_universal": "false",
    "key": "Return", "keycode": 0, "catch_all": false, "description": "",
    "allow_input_capture": false, "dispatcher": "exec", "arg": "kitty"
},
{
    "mouse": false, "non_consuming": false, "has_description": true,
    "modmask": 65, "submap": "", "submap_universal": "false",
    "key": "Q", "keycode": 0, "catch_all": false, "description": "Close the active window",
    "dispatcher": "killactive", "arg": ""
},
{
    "mouse": false, "non_consuming": true, "has_description": false,
    "modmask": 12, "submap": "", "submap_universal": "false",
    "key": "C", "keycode": 0, "catch_all": false, "description": "",
    "dispatcher": "exec", "arg": "notify-send copied"
},
{
    "mouse": false, "non_consuming": false, "has_description": false,
    "modmask": 0, "submap": "resize", "submap_universal": "false",
    "key": "right", "keycode": 0, "catch_all": false, "description": "",
    "dispatcher": "resizeactive", "arg": "10 0"
},
{
    "mouse": false, "non_consuming": false, "has_description": false,
    "modmask": 64, "submap": "resize", "submap_universal": "true",
    "key": "K", "keycode": 0, "catch_all": false, "description": "",
    "dispatcher": "exec", "arg": "kitty"
},
{
    "mouse": false, "non_consuming": false, "has_description": false,
    "modmask": 64, "submap": "", "submap_universal": "false",
    "key": "bracketleft", "keycode": 0, "catch_all": false, "description": "",
    "dispatcher": "", "arg": ""
},
{ "mouse": true, "modmask": 64, "key": "mouse:272", "dispatcher": "movewindow" },
{ "mouse": false, "modmask": 64, "key": "", "keycode": 28, "dispatcher": "exec", "arg": "amongus" },
{ "mouse": false, "modmask": 0, "submap": "clean", "key": "", "catch_all": true, "dispatcher": "submap", "arg": "reset" },
{ "mouse": false, "modmask": 80, "key": "X", "dispatcher": "exec", "arg": "numlock-only" }
]"#;

    fn parsed() -> Vec<Binding> {
        parse(SAMPLE).unwrap()
    }

    fn find<'a>(bindings: &'a [Binding], chord: &str) -> &'a Binding {
        let chord: Chord = chord.parse().unwrap();
        bindings
            .iter()
            .find(|b| b.chord == chord)
            .unwrap_or_else(|| panic!("no binding for {chord}"))
    }

    #[test]
    fn modmask_and_keysyms() {
        let bs = parsed();
        let term = find(&bs, "Super+Enter");
        assert_eq!(term.action, "exec kitty");
        assert_eq!(term.grabs, Grabs::Always);
        assert_eq!(term.mode, None);
        assert_eq!(find(&bs, "Super+[").action, "Lua function");
    }

    #[test]
    fn descriptions_win_over_dispatchers() {
        assert_eq!(
            find(&parsed(), "Super+Shift+Q").action,
            "Close the active window"
        );
    }

    #[test]
    fn non_consuming_binds_let_the_key_through() {
        assert_eq!(find(&parsed(), "Ctrl+Alt+C").grabs, Grabs::Never);
    }

    #[test]
    fn submaps_are_modes_unless_universal() {
        let bs = parsed();
        let resize = find(&bs, "Right");
        assert_eq!(resize.describe(), "resizeactive 10 0 (resize mode)");
        assert_eq!(resize.grabs, Grabs::Never);

        let universal = find(&bs, "Super+K");
        assert_eq!(universal.mode, None);
        assert_eq!(universal.grabs, Grabs::Always);
    }

    #[test]
    fn mouse_keycode_catchall_and_unknown_mods_are_skipped() {
        let bs = parsed();
        assert_eq!(bs.len(), 6);
        assert!(bs.iter().all(|b| !b.action.contains("movewindow")
            && !b.action.contains("amongus")
            && !b.action.contains("reset")
            && !b.action.contains("numlock-only")));
    }

    #[test]
    fn shadows_a_terminal_binding() {
        use crate::conflict::{self, Conflict};

        let mut bs =
            parse(r#"[{ "modmask": 5, "key": "C", "dispatcher": "exec", "arg": "grim" }]"#)
                .unwrap();
        bs.push(Binding {
            chord: "Ctrl+Shift+C".parse().unwrap(),
            action: "Copy".into(),
            tool: "alacritty",
            layer: Layer::Terminal,
            grabs: Grabs::Always,
            mode: None,
            then: Vec::new(),
            source: Source::Default,
        });
        let conflicts = conflict::find(&bs);
        assert!(matches!(
            conflicts.as_slice(),
            [Conflict::Shadowed { winner, loser }] if winner.tool == "hyprland" && loser.tool == "alacritty"
        ));
    }

    #[test]
    fn rejects_non_json() {
        assert!(parse("HYPRLAND_INSTANCE_SIGNATURE not set! (is hyprland running?)").is_err());
    }
}
