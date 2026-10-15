//! One module per tool. Each reads the tool's config (plus its built-in
//! defaults, where it has them) and turns it into [`Binding`]s.

use std::path::PathBuf;

use crate::binding::{Binding, Layer};

pub mod alacritty;
pub mod fish;
pub mod hyprland;
pub mod kde;

pub trait Adapter {
    fn tool(&self) -> &'static str;
    fn layer(&self) -> Layer;
    /// The tool's bindings, or `Ok(None)` when the tool isn't set up on this machine.
    fn load(&self) -> anyhow::Result<Option<Vec<Binding>>>;
}

pub fn all() -> Vec<Box<dyn Adapter>> {
    vec![
        Box::new(kde::Kde),
        Box::new(hyprland::Hyprland),
        Box::new(alacritty::Alacritty),
        Box::new(fish::Fish),
    ]
}

/// Whether an executable called `name` is on `$PATH`.
pub(crate) fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
}

/// `$XDG_CONFIG_HOME`, falling back to `~/.config`.
pub(crate) fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

/// Long actions (inline shell commands, say) get cut to this many characters.
pub(crate) const MAX_ACTION_LEN: usize = 60;

pub(crate) fn shorten(action: &str) -> String {
    if action.chars().count() <= MAX_ACTION_LEN {
        action.to_string()
    } else {
        let cut: String = action.chars().take(MAX_ACTION_LEN - 1).collect();
        format!("{}…", cut.trim_end())
    }
}
