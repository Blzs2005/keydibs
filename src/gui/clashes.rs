//! The Clashes tab: what `keydibs check` reports, one card each, with buttons
//! to look a key up or ignore the clash.

use eframe::egui::{self, Color32, RichText};
use keydibs::Binding;
use keydibs::conflict::Conflict;
use keydibs::ignore::{self, Rule};
use keydibs::report::{self, Finding, Problem, tilde};

use super::{Action, View};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Blocked,
    Duplicate,
    CantArrive,
}

impl Kind {
    const ALL: [Kind; 3] = [Kind::Blocked, Kind::Duplicate, Kind::CantArrive];

    fn of(problem: &Problem) -> Kind {
        match problem {
            Problem::Conflict(Conflict::Shadowed { .. }) => Kind::Blocked,
            Problem::Conflict(Conflict::Duplicate { .. }) => Kind::Duplicate,
            Problem::Unreachable(_) => Kind::CantArrive,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Kind::Blocked => "Blocked",
            Kind::Duplicate => "Duplicate",
            Kind::CantArrive => "Can't arrive",
        }
    }

    fn explanation(self) -> &'static str {
        match self {
            Kind::Blocked => "A tool that sees the key first takes it, so another never gets it",
            Kind::Duplicate => "One tool binds the same keys to two things; only one of them runs",
            Kind::CantArrive => {
                "The terminal sends these keys as other keys, so the program can't tell"
            }
        }
    }

    fn color(self, ui: &egui::Ui) -> Color32 {
        let visuals = ui.visuals();
        match self {
            Kind::Blocked => visuals.warn_fg_color,
            Kind::Duplicate if visuals.dark_mode => Color32::from_rgb(0xb1, 0x97, 0xfc),
            Kind::Duplicate => Color32::from_rgb(0x67, 0x41, 0xd9),
            Kind::CantArrive => visuals.error_fg_color,
        }
    }
}

pub(crate) struct State {
    search: String,
    /// Which kinds to show, in the order of [`Kind::ALL`].
    kinds: [bool; 3],
    show_ignored: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            search: String::new(),
            kinds: [true; 3],
            show_ignored: false,
        }
    }
}

pub(crate) fn show(ui: &mut egui::Ui, state: &mut State, view: &mut View, findings: &[Finding]) {
    let for_tool: Vec<&Finding> = findings
        .iter()
        .filter(|f| {
            view.tool
                .is_none_or(|tool| f.problem.tools().contains(&tool))
        })
        .collect();

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.search)
                .hint_text("Filter by key, tool or action")
                .desired_width(240.0),
        );
        for (kind, shown) in Kind::ALL.into_iter().zip(&mut state.kinds) {
            let count = for_tool
                .iter()
                .filter(|f| !f.is_ignored() && Kind::of(&f.problem) == kind)
                .count();
            ui.toggle_value(shown, format!("{} ({count})", kind.label()))
                .on_hover_text(kind.explanation());
        }
        let ignored = for_tool.iter().filter(|f| f.is_ignored()).count();
        ui.checkbox(&mut state.show_ignored, format!("Show ignored ({ignored})"));
    });
    ui.add_space(4.0);
    ui.separator();

    let search = state.search.to_lowercase();
    let shown: Vec<&Finding> = for_tool
        .into_iter()
        .filter(|f| state.show_ignored || !f.is_ignored())
        .filter(|f| {
            let kind = Kind::ALL.iter().position(|&k| k == Kind::of(&f.problem));
            kind.is_some_and(|i| state.kinds[i])
        })
        .filter(|f| search.is_empty() || matches_search(&f.problem, &search))
        .collect();

    if shown.is_empty() {
        empty(ui, view, findings);
        return;
    }
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            for group in group(shown) {
                card(ui, view, &group);
                ui.add_space(6.0);
            }
        });
}

/// Findings shown on one card: every shortcut one tool's binding blocks on the
/// same keys, or a single finding of another kind.
fn group<'a>(findings: Vec<&'a Finding<'a>>) -> Vec<Vec<&'a Finding<'a>>> {
    let mut groups: Vec<Vec<&Finding>> = Vec::new();
    for finding in findings {
        let joins = |group: &Vec<&Finding>| match (&group[0].problem, &finding.problem) {
            (
                Problem::Conflict(Conflict::Shadowed { winner, loser }),
                Problem::Conflict(Conflict::Shadowed {
                    winner: other_winner,
                    loser: other_loser,
                }),
            ) => {
                std::ptr::eq(*winner, *other_winner)
                    && loser.keys() == other_loser.keys()
                    && group[0].is_ignored() == finding.is_ignored()
            }
            _ => false,
        };
        match groups.iter_mut().find(|group| joins(group)) {
            Some(group) => group.push(finding),
            None => groups.push(vec![finding]),
        }
    }
    groups
}

fn matches_search(problem: &Problem, search: &str) -> bool {
    let bindings: Vec<&Binding> = match problem {
        Problem::Conflict(Conflict::Shadowed { winner, loser }) => vec![loser, winner],
        Problem::Conflict(Conflict::Duplicate { first, second }) => vec![first, second],
        Problem::Unreachable(u) => vec![u.binding],
    };
    bindings.iter().any(|b| {
        b.keys().to_lowercase().contains(search)
            || b.tool.contains(search)
            || b.describe().to_lowercase().contains(search)
    })
}

fn empty(ui: &mut egui::Ui, view: &View, findings: &[Finding]) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        if findings.iter().all(|f| f.is_ignored()) {
            ui.heading("No clashes");
            let shortcuts = view.scanned.found.bindings.len();
            ui.label(format!(
                "None of your {shortcuts} shortcuts get in each other's way."
            ));
        } else {
            ui.heading("Nothing matches");
            ui.label("Try another filter, or pick All tools on the left.");
        }
    });
}

fn card(ui: &mut egui::Ui, view: &mut View, group: &[&Finding]) {
    let first = &group[0].problem;
    let kind = Kind::of(first);
    let binding = first.binding();
    let ignored = group[0].is_ignored();
    let mut tools: Vec<&'static str> = Vec::new();
    for finding in group {
        for tool in finding.problem.tools() {
            if !tools.contains(&tool) {
                tools.push(tool);
            }
        }
    }

    let frame = egui::Frame::group(ui.style()).inner_margin(egui::Margin::same(10));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(kind.label()).strong().color(kind.color(ui)))
                .on_hover_text(kind.explanation());
            ui.label(
                RichText::new(binding.keys())
                    .monospace()
                    .strong()
                    .size(17.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ignored {
                    ui.label(RichText::new("ignored").weak().italics());
                } else {
                    ignore_menu(ui, view, binding, &tools);
                }
                if ui
                    .button("Look up")
                    .on_hover_text("See every tool that uses these keys")
                    .clicked()
                {
                    view.act(Action::LookUp(binding.chord.clone()));
                }
            });
        });

        match first {
            Problem::Conflict(Conflict::Shadowed { winner, .. }) => {
                for finding in group {
                    let loser = finding.problem.binding();
                    line(ui, view, loser.tool, "wants it for", loser);
                }
                line(ui, view, winner.tool, "takes it first, for", winner);
            }
            Problem::Conflict(Conflict::Duplicate { first, second }) => {
                line(ui, view, first.tool, "binds it to", first);
                line(ui, view, "", "and also to", second);
            }
            Problem::Unreachable(u) => {
                line(ui, view, u.binding.tool, "wants it for", u.binding);
                ui.label(report::explain(u));
            }
        }

        ui.add_space(2.0);
        for tool in &tools {
            if let Some(place) = view.where_to_change(tool) {
                ui.label(
                    RichText::new(format!("To change {tool}'s shortcut: {place}"))
                        .small()
                        .weak(),
                );
            }
        }

        let mut rules: Vec<&Rule> = Vec::new();
        for rule in group.iter().flat_map(|f| &f.ignored_by) {
            if !rules.iter().any(|r| r.line == rule.line) {
                rules.push(rule);
            }
        }
        for rule in rules {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Ignored by line {}: “{}”", rule.line, rule.text))
                        .small(),
                );
                if ui
                    .small_button("Stop ignoring")
                    .on_hover_text("Remove this line from your ignore list")
                    .clicked()
                {
                    view.act(Action::StopIgnoring(rule.clone()));
                }
            });
        }
    });
}

/// One binding of a clash: `micro wants it for “SelectRight” (built-in default)`.
fn line(ui: &mut egui::Ui, view: &mut View, tool: &str, verb: &str, binding: &Binding) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if !tool.is_empty() {
            ui.label(RichText::new(tool).strong());
        }
        ui.label(verb);
        ui.label(RichText::new(format!("“{}”", binding.describe())).italics());
        view.source(ui, &binding.source);
    });
}

/// The ways to ignore a clash on `binding`'s keys that `tools` are part of,
/// from the narrowest.
fn ignore_menu(ui: &mut egui::Ui, view: &mut View, binding: &Binding, tools: &[&str]) {
    let keys = binding.keys();
    let mut choices: Vec<(String, String)> = tools
        .iter()
        .map(|tool| {
            (
                format!("Every clash on {keys} that {tool} is part of"),
                ignore::rule_for(binding, Some(tool)),
            )
        })
        .collect();
    choices.push((
        format!("Every clash on {keys}"),
        ignore::rule_for(binding, None),
    ));

    let file = view
        .scanned
        .ignore_path
        .as_deref()
        .map_or_else(|| "your ignore list".into(), tilde);
    ui.menu_button("Ignore", |ui| {
        ui.label(RichText::new("Stop reporting:").weak());
        for (label, rule) in choices {
            let clicked = ui
                .button(label)
                .on_hover_text(format!("Adds “{rule}” to {file}"))
                .clicked();
            if clicked {
                view.act(Action::Ignore(rule));
                ui.close();
            }
        }
    });
}
