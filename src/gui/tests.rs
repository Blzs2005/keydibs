//! Clicking through the app with a fixed set of shortcuts and an ignore list
//! in a temporary directory.

use std::path::PathBuf;

use egui_kittest::Harness;
use egui_kittest::kittest::{Queryable, by};
use keydibs::reach::Terminal;
use keydibs::scan::Tool;
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
