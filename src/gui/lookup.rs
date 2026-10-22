//! The "Look up a key" tab: `keydibs why`, with keys you type or press.

use eframe::egui::{self, RichText};
use keydibs::binding::Grabs;
use keydibs::chord::{Chord, Key, Mods};
use keydibs::conflict;
use keydibs::reach::{self, Legacy};
use keydibs::{Binding, Layer};

use super::View;

#[derive(Default)]
pub(crate) struct State {
    text: String,
    /// Waiting for a key press to look up.
    recording: bool,
    chord: Option<Chord>,
    error: Option<String>,
}

impl State {
    pub(crate) fn look_up(&mut self, chord: Chord) {
        self.text = chord.to_string();
        self.chord = Some(chord);
        self.error = None;
        self.recording = false;
    }
}

pub(crate) fn show(ui: &mut egui::Ui, state: &mut State, view: &mut View) {
    if state.recording {
        match ui.input(|input| input.events.iter().find_map(recorded)) {
            Some(Recorded::Cancel) => state.recording = false,
            Some(Recorded::Chord(chord)) => state.look_up(chord),
            None => {}
        }
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let edit = ui.add(
            egui::TextEdit::singleline(&mut state.text)
                .hint_text("Type keys, like ctrl+shift+c")
                .desired_width(240.0)
                .font(egui::TextStyle::Monospace),
        );
        if edit.changed() {
            state.recording = false;
            match state.text.trim() {
                "" => {
                    state.chord = None;
                    state.error = None;
                }
                text => match text.parse::<Chord>() {
                    Ok(chord) => {
                        state.chord = Some(chord);
                        state.error = None;
                    }
                    Err(e) => state.error = Some(e.to_string()),
                },
            }
        }
        ui.label("or");
        let label = if state.recording {
            "Press keys… (Esc to stop)"
        } else {
            "Press keys"
        };
        let button = egui::Button::new(label).selected(state.recording);
        let record = ui.add(button).on_hover_text(
            "Press a key combination to look it up. Keys your desktop takes \
             (most Super+… shortcuts) never reach this window, so type those.",
        );
        if record.clicked() {
            state.recording = !state.recording;
            // Otherwise Space or Enter would press the button again.
            record.surrender_focus();
        }
    });
    if let Some(error) = &state.error {
        ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
    }
    ui.add_space(4.0);
    ui.separator();

    let Some(chord) = &state.chord else {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label("Look up a key combination to see every tool that uses it,");
            ui.label("in the order a key press reaches them, and which one gets it.");
        });
        return;
    };
    let chord = view.scanned.normalize(chord);
    results(ui, view, &chord);
}

fn results(ui: &mut egui::Ui, view: &mut View, chord: &Chord) {
    ui.add_space(4.0);
    ui.label(
        RichText::new(chord.to_string())
            .monospace()
            .strong()
            .size(18.0),
    );

    let chain = conflict::chain(&view.scanned.found.bindings, chord);
    if chain.is_empty() {
        ui.label("Nothing you use is bound to this. It's free to use.");
    } else {
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .show(ui, |ui| table(ui, view, &chain));
    }

    if let Legacy::SameAs(other) = reach::legacy(chord) {
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!(
                "Terminals without the kitty keyboard protocol send this as {other}, \
                 so shells and editors in them may not tell the two apart."
            ))
            .weak(),
        );
    }
}

/// Every binding in `chain`, with who gets the key.
fn table(ui: &mut egui::Ui, view: &mut View, chain: &[&Binding]) {
    egui::Grid::new("chain")
        .striped(true)
        .num_columns(5)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            for heading in ["Layer", "Tool", "Gets it?", "Action", "Set in"] {
                ui.strong(heading);
            }
            ui.end_row();
            for (binding, count) in conflict::folded(chain) {
                ui.label(layer_name(binding.layer));
                ui.label(RichText::new(binding.tool).strong());
                let blockers = conflict::shadowed_by(chain, binding);
                if blockers.is_empty() {
                    let text = RichText::new(binding.grabs.to_string());
                    ui.label(match binding.grabs {
                        Grabs::Never => text.weak(),
                        _ => text.color(ui.visuals().strong_text_color()),
                    });
                } else {
                    let tools: Vec<_> = blockers.iter().map(|b| b.tool).collect();
                    ui.label(
                        RichText::new(format!("blocked by {}", tools.join(", ")))
                            .color(ui.visuals().warn_fg_color),
                    );
                }
                if count > 1 {
                    ui.label(format!(
                        "[{}, …] {count} shortcuts that start with these keys",
                        binding.chord
                    ));
                    ui.label(RichText::new("All shortcuts lists them").weak());
                    ui.end_row();
                    continue;
                }
                let action = if binding.then.is_empty() {
                    binding.describe()
                } else {
                    format!("[{}] {}", binding.keys(), binding.describe())
                };
                ui.label(action);
                view.source(ui, &binding.source);
                ui.end_row();
            }
        });
}

fn layer_name(layer: Layer) -> &'static str {
    match layer {
        Layer::Compositor => "Desktop",
        Layer::Terminal => "Terminal",
        Layer::Multiplexer => "Multiplexer",
        Layer::Shell => "Shell",
        Layer::App => "App",
    }
}

enum Recorded {
    Cancel,
    Chord(Chord),
}

/// The key combination an input event stands for, while recording.
fn recorded(event: &egui::Event) -> Option<Recorded> {
    let chord = |mods: Mods, key: &str| {
        Some(Recorded::Chord(Chord {
            mods,
            key: Key::parse(key)?,
        }))
    };
    match event {
        egui::Event::Key {
            key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } => {
            if *key == egui::Key::Escape && modifiers.is_none() {
                return Some(Recorded::Cancel);
            }
            let mut mods = Mods::empty();
            for (held, flag) in [
                (modifiers.ctrl, Mods::CTRL),
                (modifiers.alt, Mods::ALT),
                (modifiers.shift, Mods::SHIFT),
            ] {
                if held {
                    mods |= flag;
                }
            }
            chord(mods, key_name(*key))
        }
        // egui turns these into clipboard events before they arrive as keys.
        egui::Event::Copy => chord(Mods::CTRL, "c"),
        egui::Event::Cut => chord(Mods::CTRL, "x"),
        egui::Event::Paste(_) | egui::Event::PasteImage(_) => chord(Mods::CTRL, "v"),
        _ => None,
    }
}

/// egui's name for a key, in a form [`Key::parse`] understands.
fn key_name(key: egui::Key) -> &'static str {
    use egui::Key as K;
    match key {
        // `symbol_or_name` gives arrow glyphs and a Unicode minus for these.
        K::ArrowUp | K::ArrowDown | K::ArrowLeft | K::ArrowRight => key.name(),
        K::Minus => "-",
        K::Quote => "'",
        _ => key.symbol_or_name(),
    }
}
