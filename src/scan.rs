//! Running every adapter and gathering what it found, for the CLI and the app.

use crate::adapters;
use crate::binding::{Binding, Layer};
use crate::layout::Layout;
use crate::reach;

/// A tool that was found and read.
#[derive(Debug, Clone)]
pub struct Tool {
    pub name: &'static str,
    pub layer: Layer,
    pub bindings: usize,
    /// See [`adapters::Adapter::where_to_change`].
    pub where_to_change: &'static str,
    pub notes: Vec<String>,
}

/// A tool that's set up but couldn't be read.
#[derive(Debug, Clone)]
pub struct Skipped {
    pub tool: &'static str,
    pub error: String,
}

/// Everything keydibs found on this machine.
#[derive(Debug, Default)]
pub struct Found {
    pub bindings: Vec<Binding>,
    pub tools: Vec<Tool>,
    pub skipped: Vec<Skipped>,
    pub terminals: Vec<reach::Terminal>,
    /// Shells and apps that turn on the kitty keyboard protocol.
    pub kitty_programs: Vec<&'static str>,
}

/// Reported as each adapter finishes, since some take a while.
pub enum Event<'a> {
    Scanned(&'a Tool),
    Skipped(&'a Skipped),
}

/// Runs every adapter, telling `on_event` about each tool that's set up.
pub fn load(mut on_event: impl FnMut(Event)) -> Found {
    let mut found = Found::default();
    for adapter in adapters::all() {
        match adapter.load() {
            Ok(Some(bindings)) => {
                let tool = Tool {
                    name: adapter.tool(),
                    layer: adapter.layer(),
                    bindings: bindings.len(),
                    where_to_change: adapter.where_to_change(),
                    notes: adapter.notes(),
                };
                on_event(Event::Scanned(&tool));
                match tool.layer {
                    Layer::Terminal => found.terminals.push(reach::Terminal {
                        tool: tool.name,
                        kitty_keyboard: adapter.kitty_keyboard(),
                    }),
                    Layer::Compositor => {}
                    _ if adapter.kitty_keyboard() => found.kitty_programs.push(tool.name),
                    _ => {}
                }
                found.tools.push(tool);
                found.bindings.extend(bindings);
            }
            Ok(None) => {}
            Err(e) => {
                let skipped = Skipped {
                    tool: adapter.tool(),
                    error: format!("{e:#}"),
                };
                on_event(Event::Skipped(&skipped));
                found.skipped.push(skipped);
            }
        }
    }
    found
}

impl Found {
    /// Rewrites every binding's keys the way they're compared on `layout`.
    pub fn normalize(&mut self, layout: &Layout) {
        for binding in &mut self.bindings {
            layout.normalize_binding(binding);
        }
    }

    pub fn tool(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|tool| tool.name == name)
    }
}
