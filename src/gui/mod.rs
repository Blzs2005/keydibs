//! The keydibs app: the same checks as the CLI, in a window.
//!
//! Scanning runs adapters that can take seconds (they start shells and
//! editors), so it happens on a background thread while the window shows what
//! it has read so far. The app only writes to the ignore list, and installs a
//! newer keydibs when you click Update available.

mod clashes;
mod lookup;
mod shortcuts;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use eframe::egui::{self, Color32, RichText};
use keydibs::ignore::{self, Rule};
use keydibs::layout::Layout;
use keydibs::report::{self, Finding, tilde};
use keydibs::scan::{self, Event, Found};
use keydibs::update::{self, Plan, Version};
use keydibs::{Chord, Source};

pub fn run() -> ExitCode {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("keydibs")
            // Matches the launcher entry `keydibs install-desktop` adds.
            .with_app_id("keydibs")
            .with_inner_size([1040.0, 720.0])
            .with_min_inner_size([640.0, 420.0])
            .with_icon(icon()),
        ..Default::default()
    };
    let started = eframe::run_native(
        "keydibs",
        options,
        Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))),
    );
    match started {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: can't open the keydibs window: {e}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Clashes,
    Lookup,
    Shortcuts,
}

/// What a scan found, and the keyboard layout and ignore list that go with it.
pub(crate) struct Scanned {
    found: Found,
    layout: Result<Layout, String>,
    /// Where the ignore list is, or would be.
    ignore_path: Option<PathBuf>,
    ignore: Result<Option<ignore::List>, String>,
}

impl Scanned {
    fn new(mut found: Found) -> Self {
        let layout = Layout::load();
        if let Ok(layout) = &layout {
            found.normalize(layout);
        }
        let mut scanned = Scanned {
            found,
            layout,
            ignore_path: ignore::path(),
            ignore: Ok(None),
        };
        scanned.reload_ignore();
        scanned
    }

    fn reload_ignore(&mut self) {
        let Some(path) = &self.ignore_path else {
            return;
        };
        self.ignore = ignore::load(path, self.layout.as_ref().ok()).map_err(|e| e.to_string());
    }

    fn rules(&self) -> &[Rule] {
        match &self.ignore {
            Ok(Some(list)) => &list.rules,
            _ => &[],
        }
    }

    /// Rewrites `chord` the way bindings are compared on this layout.
    fn normalize(&self, chord: &Chord) -> Chord {
        match &self.layout {
            Ok(layout) => layout.normalize(chord),
            Err(_) => chord.clone(),
        }
    }
}

enum Scan {
    Running {
        updates: Receiver<Update>,
        /// One line per tool read so far.
        progress: Vec<String>,
    },
    Done(Box<Scanned>),
}

/// What the scanning thread sends back.
enum Update {
    Progress(String),
    Done(Box<Found>),
}

fn start_scan(ctx: &egui::Context) -> Scan {
    let (send, updates) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let found = scan::load(|event| {
            let line = match event {
                Event::Scanned(tool) => format!("{}: {} shortcuts", tool.name, tool.bindings),
                Event::Skipped(skipped) => format!("{}: skipped", skipped.tool),
            };
            let _ = send.send(Update::Progress(line));
            ctx.request_repaint();
        });
        let _ = send.send(Update::Done(Box::new(found)));
        ctx.request_repaint();
    });
    Scan::Running {
        updates,
        progress: Vec::new(),
    }
}

/// Something the user asked for, done once the frame is drawn.
pub(crate) enum Action {
    Ignore(String),
    StopIgnoring(Rule),
    LookUp(Chord),
    Open(PathBuf),
    FilterTool(Option<&'static str>),
    Rescan,
    /// Install the newer keydibs.
    Update,
    /// Open the newly installed keydibs and close this one.
    Restart,
}

/// Whether there's a newer keydibs, and installing it.
pub(crate) enum Updating {
    /// Nothing to offer, or still asking crates.io (while there's a receiver).
    Idle(Option<Receiver<Option<Version>>>),
    Available(Version),
    Installing {
        version: Version,
        done: Receiver<Result<(), String>>,
    },
    /// Installed; the new version runs from the next start.
    Installed,
}

/// A message at the bottom of the window.
struct Status {
    text: String,
    error: bool,
}

struct App {
    scan: Scan,
    tab: Tab,
    /// Only show what this tool is part of.
    tool: Option<&'static str>,
    clashes: clashes::State,
    lookup: lookup::State,
    shortcuts: shortcuts::State,
    status: Option<Status>,
    text_files: HashMap<PathBuf, bool>,
    updating: Updating,
    /// How to update this keydibs.
    update_plan: Plan,
    /// This keydibs, to start again once it's updated. Read at startup: once
    /// the file is replaced, `/proc/self/exe` points at the deleted one.
    exe: Option<PathBuf>,
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        let mut app = Self::with_scan(ctx, start_scan(ctx));
        let ctx = ctx.clone();
        let check = update::check_in_background(std::time::Duration::from_secs(10), move || {
            ctx.request_repaint()
        });
        app.updating = Updating::Idle(Some(check));
        app
    }

    fn with_scan(ctx: &egui::Context, scan: Scan) -> Self {
        ctx.all_styles_mut(|style| {
            use egui::FontFamily::{Monospace, Proportional};
            use egui::{FontId, TextStyle};
            style.text_styles = [
                (TextStyle::Heading, FontId::new(20.0, Proportional)),
                (TextStyle::Body, FontId::new(14.5, Proportional)),
                (TextStyle::Button, FontId::new(14.5, Proportional)),
                (TextStyle::Monospace, FontId::new(14.0, Monospace)),
                (TextStyle::Small, FontId::new(12.0, Proportional)),
            ]
            .into();
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(8.0, 3.0);
        });
        App {
            scan,
            tab: Tab::Clashes,
            tool: None,
            clashes: clashes::State::default(),
            lookup: lookup::State::default(),
            shortcuts: shortcuts::State::default(),
            status: None,
            text_files: HashMap::new(),
            updating: Updating::Idle(None),
            update_plan: Plan::detect(),
            exe: std::env::current_exe().ok(),
        }
    }

    /// Takes in the answer from crates.io, or from installing the update.
    fn poll_update(&mut self) {
        match &self.updating {
            Updating::Idle(Some(check)) => match check.try_recv() {
                Ok(Some(version)) => self.updating = Updating::Available(version),
                Ok(None) | Err(TryRecvError::Disconnected) => self.updating = Updating::Idle(None),
                Err(TryRecvError::Empty) => {}
            },
            Updating::Installing { version, done } => {
                let version = *version;
                let failed = match done.try_recv() {
                    Ok(Ok(())) => {
                        self.status = info(format!(
                            "Updated to keydibs {version}. Restart keydibs to use it."
                        ));
                        self.updating = Updating::Installed;
                        return;
                    }
                    Ok(Err(e)) => e,
                    Err(TryRecvError::Disconnected) => "it stopped without saying why".to_string(),
                    Err(TryRecvError::Empty) => return,
                };
                self.status = error(format!("Updating failed: {failed}"));
                self.updating = Updating::Available(version);
            }
            Updating::Idle(None) | Updating::Available(_) | Updating::Installed => {}
        }
    }

    fn start_update(&mut self, ctx: &egui::Context) {
        let Updating::Available(version) = self.updating else {
            return;
        };
        let run = match &self.update_plan {
            Plan::Run(run) => run.clone(),
            Plan::Explain(how) => {
                self.status = info(how.clone());
                return;
            }
        };
        self.status = info(format!("Running {run}…"));
        let (send, done) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = send.send(run.run_quietly());
            ctx.request_repaint();
        });
        self.updating = Updating::Installing { version, done };
    }

    fn restart(&mut self, ctx: &egui::Context) {
        let Some(exe) = &self.exe else {
            self.status = error("can't find keydibs to start it again; open it yourself");
            return;
        };
        match Command::new(exe).arg("gui").spawn() {
            Ok(_) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(e) => self.status = error(format!("can't start {}: {e}", tilde(exe))),
        }
    }

    /// Takes in what the scanning thread sent since the last frame.
    fn poll_scan(&mut self) {
        let Scan::Running { updates, progress } = &mut self.scan else {
            return;
        };
        loop {
            match updates.try_recv() {
                Ok(Update::Progress(line)) => progress.push(line),
                Ok(Update::Done(found)) => {
                    self.scan = Scan::Done(Box::new(Scanned::new(*found)));
                    return;
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    // The thread panicked. Say so, rather than spin forever.
                    self.scan = Scan::Done(Box::new(Scanned::new(Found::default())));
                    self.status = error("reading the shortcuts failed; try Scan again");
                    return;
                }
            }
        }
    }

    fn apply(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Rescan => {
                self.scan = start_scan(ctx);
                self.status = None;
                return;
            }
            Action::Update => return self.start_update(ctx),
            Action::Restart => return self.restart(ctx),
            _ => {}
        }
        let Scan::Done(scanned) = &mut self.scan else {
            return;
        };
        match action {
            Action::Ignore(rule) => {
                let Some(path) = scanned.ignore_path.clone() else {
                    self.status = error("can't find your config directory");
                    return;
                };
                self.status = match ignore::add(&path, &rule) {
                    Ok(()) => info(format!("Added “{rule}” to {}", tilde(&path))),
                    Err(e) => error(format!("can't write {}: {e}", tilde(&path))),
                };
                scanned.reload_ignore();
            }
            Action::StopIgnoring(rule) => {
                let Some(path) = scanned.ignore_path.clone() else {
                    return;
                };
                self.status = match ignore::remove(&path, &rule) {
                    Ok(()) => info(format!("Removed “{}” from {}", rule.text, tilde(&path))),
                    Err(e) => error(format!("can't change {}: {e}", tilde(&path))),
                };
                scanned.reload_ignore();
            }
            Action::LookUp(chord) => {
                self.lookup.look_up(chord);
                self.tab = Tab::Lookup;
            }
            Action::Open(path) => {
                if let Err(e) = open(&path) {
                    self.status = error(format!("can't open {}: {e}", tilde(&path)));
                }
            }
            Action::FilterTool(tool) => self.tool = tool,
            Action::Rescan | Action::Update | Action::Restart => unreachable!("handled above"),
        }
    }
}

fn info(text: impl Into<String>) -> Option<Status> {
    Some(Status {
        text: text.into(),
        error: false,
    })
}

fn error(text: impl Into<String>) -> Option<Status> {
    Some(Status {
        text: text.into(),
        error: true,
    })
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_scan();
        self.poll_update();

        let mut actions = Vec::new();
        let App {
            scan,
            tab,
            tool,
            clashes,
            lookup,
            shortcuts,
            status,
            text_files,
            updating,
            update_plan,
            exe: _,
        } = self;
        let scan = &*scan;
        let scanned = match scan {
            Scan::Done(scanned) => Some(&**scanned),
            Scan::Running { .. } => None,
        };
        let findings = scanned.map(|s| report::check(&s.found, s.rules()));
        let count = findings
            .as_ref()
            .map(|f| f.iter().filter(|f| !f.is_ignored()).count());

        egui::Panel::top("tabs").show(ui, |ui| {
            ui.add_space(4.0);
            top_bar(ui, tab, count, updating, update_plan, &mut actions);
            ui.add_space(2.0);
        });
        status_bar(ui, status);

        match (scan, &findings) {
            (Scan::Done(scanned), Some(findings)) => {
                let mut view = View {
                    scanned,
                    tool: *tool,
                    actions: &mut actions,
                    text_files,
                };
                egui::Panel::left("tools")
                    .default_size(230.0)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .show(ui, |ui| side_panel(ui, &mut view, findings));
                    });
                egui::CentralPanel::default().show(ui, |ui| match tab {
                    Tab::Clashes => clashes::show(ui, clashes, &mut view, findings),
                    Tab::Lookup => lookup::show(ui, lookup, &mut view),
                    Tab::Shortcuts => shortcuts::show(ui, shortcuts, &mut view, findings),
                });
            }
            (Scan::Running { progress, .. }, _) => {
                egui::CentralPanel::default().show(ui, |ui| scanning(ui, progress));
            }
            (Scan::Done(_), None) => unreachable!("a finished scan has findings"),
        }

        drop(findings);
        for action in actions {
            self.apply(ui.ctx(), action);
        }
    }
}

/// The app's name, its tabs, the button to scan again, and the one to update
/// keydibs when there's a newer version. `clashes` is `None` while scanning.
fn top_bar(
    ui: &mut egui::Ui,
    tab: &mut Tab,
    clashes: Option<usize>,
    updating: &Updating,
    plan: &Plan,
    actions: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        ui.heading(RichText::new("keydibs").strong());
        ui.add_space(12.0);
        let label = match clashes {
            Some(count) => format!("Clashes ({count})"),
            None => "Clashes".to_string(),
        };
        ui.selectable_value(tab, Tab::Clashes, label);
        ui.selectable_value(tab, Tab::Lookup, "Look up a key");
        ui.selectable_value(tab, Tab::Shortcuts, "All shortcuts");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            update_button(ui, updating, plan, actions);
            let rescan = ui
                .add_enabled(clashes.is_some(), egui::Button::new("Scan again"))
                .on_hover_text("Read every tool's shortcuts again, after you change one");
            if rescan.clicked() {
                actions.push(Action::Rescan);
            }
        });
    });
}

/// In the top right corner, once there's a newer keydibs.
fn update_button(ui: &mut egui::Ui, updating: &Updating, plan: &Plan, actions: &mut Vec<Action>) {
    match updating {
        Updating::Idle(_) => {}
        Updating::Available(version) => {
            let how = match plan {
                Plan::Run(run) => format!("Click to install it with {run}"),
                Plan::Explain(_) => "Click to see how to install it".to_string(),
            };
            // The blue of the icon, which reads the same in light and dark mode.
            let text = RichText::new(format!("Update available ({version})")).color(Color32::WHITE);
            let button = egui::Button::new(text).fill(Color32::from_rgb(0x3b, 0x5b, 0xdb));
            let hover = format!(
                "keydibs {version} is out; you have {}.\n{how}",
                update::CURRENT
            );
            if ui.add(button).on_hover_text(hover).clicked() {
                actions.push(Action::Update);
            }
        }
        Updating::Installing { .. } => {
            ui.add_enabled(false, egui::Button::new("Updating…"));
            ui.spinner();
        }
        Updating::Installed => {
            if ui
                .button("Restart keydibs")
                .on_hover_text("Close this window and open the new version")
                .clicked()
            {
                actions.push(Action::Restart);
            }
        }
    }
}

fn status_bar(ui: &mut egui::Ui, status: &mut Option<Status>) {
    let Some(shown) = status else {
        return;
    };
    let mut dismissed = false;
    egui::Panel::bottom("status").show(ui, |ui| {
        ui.horizontal(|ui| {
            let color = if shown.error {
                ui.visuals().error_fg_color
            } else {
                ui.visuals().text_color()
            };
            ui.label(RichText::new(&shown.text).color(color));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                dismissed = ui.small_button("✕").on_hover_text("Dismiss").clicked();
            });
        });
    });
    if dismissed {
        *status = None;
    }
}

fn scanning(ui: &mut egui::Ui, progress: &[String]) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        ui.spinner();
        ui.add_space(8.0);
        ui.heading("Reading your shortcuts…");
        ui.label("keydibs only reads settings; it never changes them.");
        ui.add_space(12.0);
        for line in progress {
            ui.weak(line);
        }
    });
}

/// The tools that were found, to filter by, and anything worth knowing about
/// how they were read.
fn side_panel(ui: &mut egui::Ui, view: &mut View, findings: &[Finding]) {
    ui.add_space(6.0);
    ui.strong("Tools found");
    // A justified layout makes every row as wide as the panel, so they all
    // highlight the same on hover however long their text is.
    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
        if ui
            .add(egui::Button::selectable(view.tool.is_none(), "All tools"))
            .clicked()
        {
            view.act(Action::FilterTool(None));
        }
        for tool in &view.scanned.found.tools {
            let clashes = findings
                .iter()
                .filter(|f| !f.is_ignored() && f.problem.tools().contains(&tool.name))
                .count();
            let count = match clashes {
                0 => String::new(),
                1 => "1 clash".to_string(),
                count => format!("{count} clashes"),
            };
            let selected = view.tool == Some(tool.name);
            let response = ui
                .add(egui::Button::selectable(selected, tool.name).right_text(count))
                .on_hover_text(format!(
                    "{} ({}), {} shortcuts\nWhere to change them: {}",
                    tool.name, tool.layer, tool.bindings, tool.where_to_change
                ));
            if response.clicked() {
                view.act(Action::FilterTool((!selected).then_some(tool.name)));
            }
            for note in &tool.notes {
                ui.label(RichText::new(note).small().weak());
            }
        }
    });
    let found = &view.scanned.found;
    for skipped in &found.skipped {
        ui.label(
            RichText::new(format!("{} skipped: {}", skipped.tool, skipped.error))
                .small()
                .color(ui.visuals().warn_fg_color),
        );
    }

    ui.add_space(12.0);
    ui.strong("Keyboard layout");
    match &view.scanned.layout {
        Ok(layout) if layout.name.is_empty() => ui.label("default"),
        Ok(layout) => ui.label(&layout.name),
        Err(e) => ui.label(RichText::new(e).small().weak()),
    };

    ui.add_space(12.0);
    ui.strong("Ignore list");
    match &view.scanned.ignore {
        Ok(None) => {
            ui.label(RichText::new("None yet. Ignore a clash to start one.").weak());
        }
        Ok(Some(list)) => {
            let rules = match list.rules.len() {
                1 => "1 rule".to_string(),
                count => format!("{count} rules"),
            };
            ui.label(format!("{rules} in {}", tilde(&list.path)));
            if ui.link("Open in your editor").clicked() {
                view.act(Action::Open(list.path.clone()));
            }
            let warn = ui.visuals().warn_fg_color;
            for line in &list.bad {
                let text = format!("line {}: {}", line.line, line.message);
                ui.label(RichText::new(text).small().color(warn));
            }
            for rule in report::unused(&list.rules, findings) {
                let text = format!("line {} matches nothing: {}", rule.line, rule.text);
                ui.label(RichText::new(text).small().weak());
            }
        }
        Err(e) => {
            ui.label(RichText::new(e).small().color(ui.visuals().error_fg_color));
        }
    }
}

/// What the tabs share: the scan, the tool filter, and a way to ask for actions.
pub(crate) struct View<'a> {
    scanned: &'a Scanned,
    tool: Option<&'static str>,
    actions: &'a mut Vec<Action>,
    /// Whether each config file seen so far is text, and so worth opening.
    text_files: &'a mut HashMap<PathBuf, bool>,
}

impl View<'_> {
    fn act(&mut self, action: Action) {
        self.actions.push(action);
    }

    /// Where a binding was made, as a link that opens the file when it's one
    /// you can edit.
    fn source(&mut self, ui: &mut egui::Ui, source: &Source) {
        let text = report::source(source);
        match source {
            Source::Config { path, .. } if self.is_text(path) => {
                if ui
                    .link(RichText::new(text).small())
                    .on_hover_text("Open in your editor")
                    .clicked()
                {
                    self.act(Action::Open(path.clone()));
                }
            }
            _ => {
                ui.label(RichText::new(text).small().weak());
            }
        }
    }

    /// Whether `path` is a text file, as opposed to, say, GNOME's settings database.
    fn is_text(&mut self, path: &Path) -> bool {
        *self
            .text_files
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                let mut start = Vec::new();
                std::fs::File::open(path)
                    .and_then(|file| file.take(4096).read_to_end(&mut start))
                    .is_ok_and(|_| !start.contains(&0))
            })
    }

    fn where_to_change(&self, tool: &str) -> Option<&'static str> {
        self.scanned.found.tool(tool).map(|t| t.where_to_change)
    }
}

/// Opens `path` in the app the desktop uses for it.
fn open(path: &Path) -> std::io::Result<()> {
    let mut child = Command::new("xdg-open").arg(path).spawn()?;
    // Reap it once it's done, so it doesn't linger as a zombie.
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// The window icon: two overlapping keycaps, as in `assets/keydibs.svg`.
fn icon() -> egui::IconData {
    const SIZE: usize = 64;
    // (x, y, width, height, corner radius, color), drawn in order.
    const SHAPES: [(f32, f32, f32, f32, f32, [u8; 3]); 4] = [
        (22.0, 6.0, 36.0, 36.0, 8.0, [0x3b, 0x5b, 0xdb]),
        (26.0, 9.0, 28.0, 26.0, 5.0, [0x74, 0x8f, 0xfc]),
        (6.0, 22.0, 36.0, 36.0, 8.0, [0xe8, 0x59, 0x0c]),
        (10.0, 25.0, 28.0, 26.0, 5.0, [0xff, 0x92, 0x2b]),
    ];
    let inside = |(x, y, w, h, r, _): (f32, f32, f32, f32, f32, [u8; 3]), px: f32, py: f32| {
        let dx = px - px.clamp(x + r, x + w - r);
        let dy = py - py.clamp(y + r, y + h - r);
        px >= x && px <= x + w && py >= y && py <= y + h && dx * dx + dy * dy <= r * r
    };
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    for (i, pixel) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (px, py) = ((i % SIZE) as f32, (i / SIZE) as f32);
        for shape in SHAPES {
            // 4×4 samples per pixel, for smooth edges.
            let covered = (0..16)
                .filter(|s| {
                    let sx = px + (s % 4) as f32 / 4.0 + 0.125;
                    let sy = py + (s / 4) as f32 / 4.0 + 0.125;
                    inside(shape, sx, sy)
                })
                .count() as f32
                / 16.0;
            if covered > 0.0 {
                let color = Color32::from_rgb(shape.5[0], shape.5[1], shape.5[2]);
                let under = Color32::from_rgba_unmultiplied(pixel[0], pixel[1], pixel[2], pixel[3]);
                let blended = under.blend(color.gamma_multiply(covered));
                pixel.copy_from_slice(&blended.to_srgba_unmultiplied());
            }
        }
    }
    egui::IconData {
        rgba,
        width: SIZE as u32,
        height: SIZE as u32,
    }
}
