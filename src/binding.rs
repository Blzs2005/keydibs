use std::fmt;
use std::path::PathBuf;

use crate::chord::Chord;

/// Where a binding sits in the chain a key press travels down.
/// Earlier variants see the key first, so the derived `Ord` is the chain order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layer {
    Compositor,
    Terminal,
    Multiplexer,
    Shell,
    App,
}

impl fmt::Display for Layer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            Layer::Compositor => "compositor",
            Layer::Terminal => "terminal",
            Layer::Multiplexer => "multiplexer",
            Layer::Shell => "shell",
            Layer::App => "app",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Read from a config file; `line` is 1-based, when known.
    Config { path: PathBuf, line: Option<usize> },
    /// A built-in default of the tool that no config file mentions.
    Default,
    /// Set at runtime (say, by a shell function) with no file we can point to.
    Runtime,
}

impl Layer {
    /// Whether keys reach this layer before `other`, so it can keep them from it.
    ///
    /// The shell and full-screen apps take turns at the terminal instead of
    /// stacking: while an app runs, the shell isn't reading keys.
    pub fn is_above(self, other: Layer) -> bool {
        self < other && !(self == Layer::Shell && other == Layer::App)
    }
}

/// When a binding takes the key away from the layers below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Grabs {
    /// Nothing below ever sees the chord.
    Always,
    /// Only on the terminal's normal screen: the shell loses the key, but
    /// full-screen programs (editors, pagers, tmux) still get it.
    NormalScreen,
    /// Only while a full-screen program is running (the alternate screen).
    AltScreen,
    /// Never: it only fires in one of the tool's own modes (like Alacritty's vi
    /// mode), or it acts and then passes the key on.
    Never,
}

impl Grabs {
    /// Whether this binding can keep the key from reaching `layer`.
    pub fn can_shadow(self, layer: Layer) -> bool {
        match self {
            Grabs::Always => true,
            Grabs::NormalScreen => layer == Layer::Shell,
            Grabs::AltScreen => matches!(layer, Layer::Multiplexer | Layer::App),
            Grabs::Never => false,
        }
    }

    /// Whether two bindings can both want the same key press.
    pub fn overlaps(self, other: Grabs) -> bool {
        !matches!(
            (self, other),
            (Grabs::Never, _)
                | (_, Grabs::Never)
                | (Grabs::NormalScreen, Grabs::AltScreen)
                | (Grabs::AltScreen, Grabs::NormalScreen)
        )
    }
}

impl fmt::Display for Grabs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            Grabs::Always => "takes it",
            Grabs::NormalScreen => "takes it outside full-screen apps",
            Grabs::AltScreen => "takes it in full-screen apps",
            Grabs::Never => "lets it through",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub chord: Chord,
    pub action: String,
    /// Short tool name, e.g. `kde` or `alacritty`.
    pub tool: &'static str,
    pub layer: Layer,
    pub grabs: Grabs,
    /// The tool's own mode this binding lives in (`vi`, `insert`…), if it has modes.
    /// Bindings in different modes never compete with each other.
    pub mode: Option<String>,
    /// Keys pressed after `chord` to complete a sequence like `Ctrl+X, Ctrl+E`.
    /// Only `chord` matters for shadowing: if it never arrives, nor does the rest.
    pub then: Vec<Chord>,
    pub source: Source,
}

impl Binding {
    /// The keys that trigger it: `Ctrl+X` or `Ctrl+X, Ctrl+E`.
    pub fn keys(&self) -> String {
        let mut keys = self.chord.to_string();
        for chord in &self.then {
            keys.push_str(&format!(", {chord}"));
        }
        keys
    }

    /// The action, plus the mode it lives in, if any.
    pub fn describe(&self) -> String {
        match &self.mode {
            Some(mode) => format!("{} ({mode} mode)", self.action),
            None => self.action.clone(),
        }
    }
}
