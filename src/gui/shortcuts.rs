//! The "All shortcuts" tab: `keydibs list`, searchable and sortable, with the
//! shortcuts that are part of a clash marked.

use std::collections::HashSet;

use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};
use keydibs::Binding;
use keydibs::conflict::Conflict;
use keydibs::report::{self, Finding, Problem};

use super::{Action, View};

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortBy {
    Keys,
    Tool,
    Layer,
    Action,
    Source,
}

pub(crate) struct State {
    search: String,
    only_clashes: bool,
    sort_by: SortBy,
    descending: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            search: String::new(),
            only_clashes: false,
            sort_by: SortBy::Layer,
            descending: false,
        }
    }
}

pub(crate) fn show(ui: &mut egui::Ui, state: &mut State, view: &mut View, findings: &[Finding]) {
    let clashing = clashing(findings);
    let search = state.search.to_lowercase();
    let mut rows: Vec<&Binding> = view
        .scanned
        .found
        .bindings
        .iter()
        .filter(|b| view.tool.is_none_or(|tool| b.tool == tool))
        .filter(|b| !state.only_clashes || clashing.contains(&(*b as *const Binding)))
        .filter(|b| {
            search.is_empty()
                || b.keys().to_lowercase().contains(&search)
                || b.tool.contains(&search)
                || b.describe().to_lowercase().contains(&search)
        })
        .collect();
    sort(&mut rows, state.sort_by);
    if state.descending {
        rows.reverse();
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.search)
                .hint_text("Filter by key, tool or action")
                .desired_width(240.0),
        );
        ui.checkbox(&mut state.only_clashes, "Only ones that clash");
        let count = match rows.len() {
            1 => "1 shortcut".to_string(),
            count => format!("{count} shortcuts"),
        };
        ui.label(RichText::new(count).weak());
    });
    ui.add_space(4.0);

    let warn = ui.visuals().warn_fg_color;
    let row_height = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .auto_shrink(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(170.0).at_least(90.0))
        .column(Column::initial(90.0).at_least(60.0))
        .column(Column::initial(90.0).at_least(60.0))
        .column(Column::initial(300.0).at_least(100.0).clip(true))
        .column(Column::remainder().at_least(100.0).clip(true))
        .header(row_height + 4.0, |mut header| {
            for (column, title) in [
                (SortBy::Keys, "Keys"),
                (SortBy::Tool, "Tool"),
                (SortBy::Layer, "Layer"),
                (SortBy::Action, "Action"),
                (SortBy::Source, "Set in"),
            ] {
                header.col(|ui| {
                    let sorted = state.sort_by == column;
                    let arrow = match (sorted, state.descending) {
                        (false, _) => "",
                        (true, false) => " ⏶",
                        (true, true) => " ⏷",
                    };
                    let label = RichText::new(format!("{title}{arrow}")).strong();
                    if ui.selectable_label(sorted, label).clicked() {
                        state.descending = sorted && !state.descending;
                        state.sort_by = column;
                    }
                });
            }
        })
        .body(|body| {
            body.rows(row_height, rows.len(), |mut row| {
                let binding = rows[row.index()];
                let clashes = clashing.contains(&(binding as *const Binding));
                row.col(|ui| {
                    let keys = RichText::new(binding.keys()).monospace();
                    let keys = if clashes { keys.color(warn) } else { keys };
                    let hint = if clashes {
                        "Part of a clash. Click to see every tool that uses these keys"
                    } else {
                        "Click to see every tool that uses these keys"
                    };
                    let label = ui
                        .add(egui::Label::new(keys).sense(egui::Sense::click()))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(hint);
                    if label.clicked() {
                        view.act(Action::LookUp(binding.chord.clone()));
                    }
                });
                row.col(|ui| {
                    ui.label(binding.tool);
                });
                row.col(|ui| {
                    ui.label(binding.layer.to_string());
                });
                row.col(|ui| {
                    ui.label(binding.describe());
                });
                row.col(|ui| view.source(ui, &binding.source));
            });
        });
}

/// The bindings that are part of a clash that isn't ignored.
fn clashing(findings: &[Finding]) -> HashSet<*const Binding> {
    let mut set = HashSet::new();
    for finding in findings.iter().filter(|f| !f.is_ignored()) {
        match &finding.problem {
            Problem::Conflict(Conflict::Shadowed { winner, loser }) => {
                set.extend([*winner as *const Binding, *loser]);
            }
            Problem::Conflict(Conflict::Duplicate { first, second }) => {
                set.extend([*first as *const Binding, *second]);
            }
            Problem::Unreachable(u) => {
                set.insert(u.binding as *const Binding);
            }
        }
    }
    set
}

fn sort(rows: &mut [&Binding], by: SortBy) {
    let keys = |b: &Binding| (b.chord.clone(), b.then.clone());
    match by {
        SortBy::Keys => rows.sort_by_cached_key(|b| keys(b)),
        SortBy::Tool => rows.sort_by_cached_key(|b| (b.tool, keys(b))),
        SortBy::Layer => rows.sort_by_cached_key(|b| (b.layer, keys(b))),
        SortBy::Action => rows.sort_by_cached_key(|b| (b.describe().to_lowercase(), keys(b))),
        SortBy::Source => rows.sort_by_cached_key(|b| (report::source(&b.source), keys(b))),
    }
}
