//! keydibs: find keyboard shortcuts that clash between your desktop, terminal, shell and editor.
//!
//! A key press travels down a chain of layers (compositor, terminal, multiplexer,
//! shell, app). The first layer with a binding for the chord takes it, and every
//! binding further down for the same chord is dead. Adapters read each tool's
//! config into [`Binding`]s, and [`conflict`] works out who wins.

pub mod adapters;
pub mod binding;
pub mod chord;
pub mod conflict;

pub use binding::{Binding, Layer, Source};
pub use chord::{Chord, Key, Mods};
