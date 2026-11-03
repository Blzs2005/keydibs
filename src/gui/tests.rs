//! Clicking through the app with a fixed set of shortcuts and an ignore list
//! in a temporary directory.

use std::path::PathBuf;

use egui_kittest::Harness;
use egui_kittest::kittest::{Queryable, by};
use keydibs::reach::Terminal;
use keydibs::scan::Tool;
use keydibs::update::Plan;
use keydibs::{Binding, Layer};

use super::*;

fn bind(keys: &str, action: &str, tool: &'static str, layer: Layer) -> Binding {
    Binding {
        chord: keys.parse().unwrap(),
        action: action.into(),
        tool,
        layer,
        grabs: keydibs::binding::Grabs::Always,
        mode: None,
        then: Vec::new(),
        source: Source::Default,
    }
}

fn tool(name: &'static str, layer: Layer) -> Tool {
    Tool {
        name,
        layer,
        bindings: 1,
        where_to_change: "its settings",
        notes: Vec::new(),
    }
}

/// Konsole takes Shift+Right from fish and micro, and micro's Ctrl+Shift+S
/// can't arrive.
fn app(ignore_path: PathBuf) -> Harness<'static, App> {
    let found = Found {
        bindings: vec![
            bind("Shift+Right", "SelectRight", "micro", Layer::App),
            bind("Shift+Right", "forward-bigword", "fish", Layer::Shell),
            bind("Shift+Right", "Next Tab", "konsole", Layer::Terminal),
            bind("Ctrl+Shift+S", "SaveAll", "micro", Layer::App),
        ],
        tools: vec![
            tool("konsole", Layer::Terminal),
            tool("fish", Layer::Shell),
            tool("micro", Layer::App),
        ],
        terminals: vec![Terminal {
            tool: "konsole",
            kitty_keyboard: true,
            layer: Layer::Terminal,
        }],
        ..Found::default()
    };
    let scanned = Scanned {
        found,
        layout: Err("no layout in tests".into()),
        ignore_path: Some(ignore_path),
        ignore: Ok(None),
    };
    let mut harness = Harness::builder()
        .with_size([1000.0, 700.0])
        .allow_missing_glyphs()
        .build_eframe(|cc| App::with_scan(&cc.egui_ctx, Scan::Done(Box::new(scanned))));
    harness.run();
    harness
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("keydibs-gui-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn clashes_can_be_ignored_and_brought_back() {
    let dir = temp_dir("ignore");
    let path = dir.join("keydibs/ignore");
    let mut app = app(path.clone());
    app.get(by().label("Clashes (3)"));

    // Konsole blocks both on Shift+Right, so they share one card.
    assert_eq!(app.query_all(by().label("Ignore")).count(), 2);
    app.query_all(by().label("Ignore")).next().unwrap().click();
    app.run();
    app.get(by().label("Every clash on Shift+Right that konsole is part of"))
        .click();
    app.run();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "Shift+Right konsole\n"
    );
    app.get(by().label("Clashes (1)"));

    app.get(by().label("Show ignored (2)")).click();
    app.run();
    app.get(by().label("Stop ignoring")).click();
    app.run();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
    app.get(by().label("Clashes (3)"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn pressed_keys_are_looked_up() {
    let mut app = app(temp_dir("lookup").join("ignore"));
    app.get(by().label("Look up a key")).click();
    app.run();
    app.get(by().label("Press keys")).click();
    app.run();
    app.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowRight);
    app.run();
    assert_eq!(app.query_all(by().label("blocked by konsole")).count(), 2);
    app.get(by().label("takes it"));
}

#[test]
fn all_shortcuts_can_be_filtered_by_tool() {
    let mut app = app(temp_dir("shortcuts").join("ignore"));
    app.get(by().label("All shortcuts")).click();
    app.run();
    app.get(by().label("4 shortcuts"));

    // Fish is part of one clash, on Shift+Right.
    app.query_all(by().label_contains("fish"))
        .next()
        .unwrap()
        .click();
    app.run();
    app.get(by().label("1 shortcut"));
}

/// The app with keydibs 9.9.9 out, updating it with `plan`.
fn app_with_update(name: &str, plan: Plan) -> Harness<'static, App> {
    let mut app = app(temp_dir(name).join("ignore"));
    let state = app.state_mut();
    state.updating = Updating::Available("9.9.9".parse().unwrap());
    state.update_plan = plan;
    app.run();
    app
}

/// Steps the app until a label containing `text` shows up, while a background
/// thread finishes.
fn wait_for(app: &mut Harness<'static, App>, text: &str) {
    for _ in 0..400 {
        app.step();
        if app.query_all_by_label_contains(text).next().is_some() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("“{text}” never showed up");
}

#[test]
fn there_is_no_update_button_without_a_newer_version() {
    let app = app(temp_dir("no-update").join("ignore"));
    assert!(app.query_by_label_contains("Update available").is_none());
}

#[test]
fn the_update_button_installs_the_newer_version() {
    let plan = Plan::Run(keydibs::update::Run::new("sh", &["-c", "sleep 0.3"]));
    let mut app = app_with_update("update", plan);
    app.get(by().label("Update available (9.9.9)")).click();
    // The spinner keeps asking for frames, so `run` would never settle.
    app.run_steps(2);
    app.get(by().label("Updating…"));
    app.get(by().label("Running sh -c sleep 0.3…"));

    wait_for(&mut app, "Updated to keydibs 9.9.9");
    app.get(by().label("Restart keydibs"));
    app.get(by().label("Updated to keydibs 9.9.9. Restart keydibs to use it."));
    assert!(app.query_by_label_contains("Update available").is_none());
}

#[test]
fn a_failed_update_says_why_and_can_be_tried_again() {
    let fails = "echo 'error: no network' >&2; exit 1";
    let plan = Plan::Run(keydibs::update::Run::new("sh", &["-c", fails]));
    let mut app = app_with_update("update-fails", plan);
    app.get(by().label("Update available (9.9.9)")).click();
    wait_for(&mut app, "Updating failed: error: no network");
    app.get(by().label("Update available (9.9.9)"));
    assert!(app.query_by_label("Restart keydibs").is_none());
}

#[test]
fn an_update_keydibs_cant_install_explains_how() {
    let plan = Plan::Explain("update it with your package manager".into());
    let mut app = app_with_update("update-explain", plan);
    app.get(by().label("Update available (9.9.9)")).click();
    app.run();
    app.get(by().label("update it with your package manager"));
    app.get(by().label("Update available (9.9.9)"));
}
