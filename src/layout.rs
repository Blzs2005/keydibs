//! Matching symbol keys the way your keyboard layout types them.
//!
//! Tools write a shortcut on a symbol either as the symbol (`Ctrl+(`) or as the
//! key that types it plus Shift (`Ctrl+Shift+9`). Which key that is depends on
//! the layout: `(` is Shift+9 on a US keyboard but Shift+8 on a Hungarian one.
//! keydibs asks libxkbcommon, which every Linux desktop has, what the configured
//! layout types on each key, and rewrites symbols typed with Shift as that key
//! plus Shift, so both spellings of a shortcut match.
//!
//! libxkbcommon is loaded at runtime, so keydibs still builds and runs without
//! it; symbols are then compared as written.

use std::collections::HashMap;
use std::ffi::{CString, c_char};
use std::fs;
use std::path::Path;

use xkbcommon_dl::{
    XkbCommon, xkb_context, xkb_context_flags, xkb_keymap, xkb_keymap_compile_flags,
    xkb_keysym_flags, xkb_rule_names,
};

use crate::binding::Binding;
use crate::chord::{Chord, Key, Mods};

/// What a keyboard layout types on its keys, without and with Shift.
pub struct Layout {
    /// The layout's name, like `hu` or `us(dvorak)`.
    pub name: String,
    /// Symbols that need Shift, and the key's unshifted symbol: `(` → `8`.
    shifted: HashMap<char, char>,
    xkb: &'static XkbCommon,
}

impl Layout {
    /// The layout configured on this system (see [`configured`]).
    pub fn load() -> Result<Layout, String> {
        let (layout, variant) = configured();
        Layout::named(&layout, &variant)
    }

    /// A specific layout and variant, like `("us", "dvorak")`. An empty layout
    /// means libxkbcommon's default.
    pub fn named(layout: &str, variant: &str) -> Result<Layout, String> {
        let xkb = xkbcommon_dl::xkbcommon_option()
            .ok_or("libxkbcommon isn't installed, so symbol keys are compared as written")?;
        let keymap = Keymap::new(xkb, layout, variant)
            .ok_or_else(|| format!("libxkbcommon doesn't know the keyboard layout `{layout}`"))?;

        // A symbol a typing key types without Shift stays as it is, even if another
        // key also types it with Shift.
        let mut unshifted = Vec::new();
        let mut shifted = HashMap::new();
        for key in TYPING_KEYS.clone().chain([ISO_EXTRA_KEY]) {
            let base = keymap.char_at(key, 0);
            if let Some(base) = base {
                unshifted.push(base);
            }
            if let (Some(base), Some(with_shift)) = (base, keymap.char_at(key, 1))
                && !with_shift.is_alphabetic()
                && with_shift != base
            {
                shifted.entry(with_shift).or_insert(base);
            }
        }
        shifted.retain(|symbol, _| !unshifted.contains(symbol));

        let name = match variant {
            "" => layout.to_string(),
            variant => format!("{layout}({variant})"),
        };
        Ok(Layout { name, shifted, xkb })
    }

    /// Rewrites `chord` in the form keydibs compares: key names that stand for a
    /// symbol become the symbol (`parenleft` → `(`), and symbols typed with Shift
    /// become the key plus Shift (`Ctrl+(` → `Ctrl+Shift+8` on a Hungarian layout).
    pub fn normalize(&self, chord: &Chord) -> Chord {
        let mut chord = chord.clone();
        if let Some(symbol) = self.symbol_for_name(chord.key.as_str()) {
            chord.key = symbol;
        }
        let mut chars = chord.key.as_str().chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && let Some(&base) = self.shifted.get(&c)
            && let Some(key) = Key::parse(&base.to_string())
        {
            chord.mods |= Mods::SHIFT;
            chord.key = key;
        }
        chord
    }

    /// Normalizes every key of `binding`.
    pub fn normalize_binding(&self, binding: &mut Binding) {
        binding.chord = self.normalize(&binding.chord);
        for chord in &mut binding.then {
            *chord = self.normalize(chord);
        }
    }

    /// The symbol a keysym name stands for (`parenleft`, `eacute`), if it's a
    /// printable one. Single characters and names like `f1` are left alone.
    fn symbol_for_name(&self, name: &str) -> Option<Key> {
        if name.chars().count() < 2 || name.contains(' ') {
            return None;
        }
        let name = CString::new(name).ok()?;
        // SAFETY: `name` is a valid C string for the duration of the call.
        let keysym = unsafe {
            (self.xkb.xkb_keysym_from_name)(
                name.as_ptr(),
                xkb_keysym_flags::XKB_KEYSYM_CASE_INSENSITIVE,
            )
        };
        let symbol = char::from_u32(unsafe { (self.xkb.xkb_keysym_to_utf32)(keysym) })?;
        if keysym == 0 || symbol.is_control() || symbol.is_whitespace() {
            return None;
        }
        Key::parse(&symbol.to_string())
    }
}

/// The configured layout and variant, from the first place that sets one:
/// `XKB_DEFAULT_LAYOUT`, KDE's keyboard settings, or the system keyboard
/// settings (`localectl`). With several layouts, the first one. Empty if none
/// is set, which libxkbcommon takes as its default (US).
pub fn configured() -> (String, String) {
    let first = |list: &str| list.split(',').next().unwrap_or("").trim().to_string();

    if let Ok(layout) = std::env::var("XKB_DEFAULT_LAYOUT")
        && !layout.is_empty()
    {
        let variant = std::env::var("XKB_DEFAULT_VARIANT").unwrap_or_default();
        return (first(&layout), first(&variant));
    }

    let kde = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))
        .map(|dir| dir.join("kxkbrc"));
    if let Some(text) = kde.and_then(|path| fs::read_to_string(path).ok()) {
        let value = |key: &str| {
            text.lines()
                .find_map(|line| line.trim().strip_prefix(key)?.strip_prefix('='))
                .map(str::trim)
        };
        if value("Use") == Some("true")
            && let Some(layout) = value("LayoutList").filter(|l| !l.is_empty())
        {
            return (first(layout), first(value("VariantList").unwrap_or("")));
        }
    }

    if let Ok(text) = fs::read_to_string("/etc/X11/xorg.conf.d/00-keyboard.conf") {
        let option = |name: &str| {
            text.lines().find_map(|line| {
                let rest = line.trim().strip_prefix("Option")?.trim();
                let rest = rest.strip_prefix(&format!("\"{name}\""))?.trim();
                Some(rest.trim_matches('"').to_string())
            })
        };
        if let Some(layout) = option("XkbLayout").filter(|l| !l.is_empty()) {
            return (
                first(&layout),
                first(&option("XkbVariant").unwrap_or_default()),
            );
        }
    }

    (String::new(), String::new())
}

/// The main typing keys, as xkb keycodes (evdev codes plus 8): the number row
/// through the bottom letter row, `KEY_1` (2) to `KEY_SLASH` (53). Extra keys
/// like a keypad `(` are left out, so they don't hide the usual way to type a
/// symbol.
const TYPING_KEYS: std::ops::RangeInclusive<u32> = 10..=61;

/// The extra key next to left Shift on ISO keyboards (`KEY_102ND`, 86), which
/// many layouts use for a symbol or letter.
const ISO_EXTRA_KEY: u32 = 94;

/// A compiled keymap, freed when dropped.
struct Keymap {
    xkb: &'static XkbCommon,
    context: *mut xkb_context,
    keymap: *mut xkb_keymap,
}

impl Keymap {
    fn new(xkb: &'static XkbCommon, layout: &str, variant: &str) -> Option<Keymap> {
        let c_string = |s: &str| CString::new(s).ok();
        let layout = c_string(layout)?;
        let variant = c_string(variant)?;
        let pointer = |s: &CString| -> *const c_char {
            if s.as_bytes().is_empty() {
                std::ptr::null()
            } else {
                s.as_ptr()
            }
        };
        let names = xkb_rule_names {
            rules: std::ptr::null(),
            model: std::ptr::null(),
            layout: pointer(&layout),
            variant: pointer(&variant),
            options: std::ptr::null(),
        };

        // SAFETY: plain libxkbcommon calls; every pointer passed is valid for the
        // call, and null results are checked before use.
        unsafe {
            let context = (xkb.xkb_context_new)(xkb_context_flags::XKB_CONTEXT_NO_FLAGS);
            if context.is_null() {
                return None;
            }
            let keymap = (xkb.xkb_keymap_new_from_names)(
                context,
                &names,
                xkb_keymap_compile_flags::XKB_KEYMAP_COMPILE_NO_FLAGS,
            );
            if keymap.is_null() {
                (xkb.xkb_context_unref)(context);
                return None;
            }
            Some(Keymap {
                xkb,
                context,
                keymap,
            })
        }
    }

    /// The character `key` types at `level` (0 plain, 1 with Shift) in the
    /// first layout group, if it types a printable one.
    fn char_at(&self, key: u32, level: u32) -> Option<char> {
        let mut syms: *const u32 = std::ptr::null();
        // SAFETY: `self.keymap` is valid; libxkbcommon points `syms` at its own
        // array of `count` keysyms, which lives as long as the keymap.
        let keysym = unsafe {
            let count =
                (self.xkb.xkb_keymap_key_get_syms_by_level)(self.keymap, key, 0, level, &mut syms);
            if count < 1 || syms.is_null() {
                return None;
            }
            *syms
        };
        let c = char::from_u32(unsafe { (self.xkb.xkb_keysym_to_utf32)(keysym) })?;
        (c != '\0' && !c.is_control() && !c.is_whitespace()).then_some(c)
    }
}

impl Drop for Keymap {
    fn drop(&mut self) {
        // SAFETY: both were created in `Keymap::new` and are released once.
        unsafe {
            (self.xkb.xkb_keymap_unref)(self.keymap);
            (self.xkb.xkb_context_unref)(self.context);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// libxkbcommon and its layout data are on every Linux desktop and in CI,
    /// but say so plainly if they're missing rather than failing obscurely.
    fn layout(name: &str) -> Layout {
        Layout::named(name, "").unwrap_or_else(|e| panic!("can't load layout {name}: {e}"))
    }

    fn same(layout: &Layout, a: &str, b: &str) -> bool {
        layout.normalize(&a.parse().unwrap()) == layout.normalize(&b.parse().unwrap())
    }

    #[test]
    fn us_symbols_match_key_plus_shift() {
        let us = layout("us");
        assert!(same(&us, "Ctrl+(", "Ctrl+Shift+9"));
        assert!(same(&us, "Meta+~", "Meta+Shift+`"));
        assert!(same(&us, "Ctrl++", "Ctrl+Shift+="));
        assert!(!same(&us, "Ctrl+=", "Ctrl+Shift+="));
        assert_eq!(
            us.normalize(&"Ctrl+(".parse().unwrap()).to_string(),
            "Ctrl+Shift+9"
        );
    }

    #[test]
    fn hungarian_symbols_are_on_other_keys() {
        let hu = layout("hu");
        assert!(same(&hu, "Ctrl+(", "Ctrl+Shift+8"));
        assert!(!same(&hu, "Ctrl+(", "Ctrl+Shift+9"));
        // `~` needs AltGr on a Hungarian keyboard, which keydibs doesn't model,
        // so it's left as written.
        assert_eq!(
            hu.normalize(&"Meta+~".parse().unwrap()).to_string(),
            "Super+~"
        );
    }

    #[test]
    fn letters_and_named_keys_are_left_alone() {
        let us = layout("us");
        assert_eq!(
            us.normalize(&"Meta+E".parse().unwrap()).to_string(),
            "Super+E"
        );
        assert_eq!(
            us.normalize(&"Ctrl+F1".parse().unwrap()).to_string(),
            "Ctrl+F1"
        );
        assert_eq!(
            us.normalize(&"Shift+Tab".parse().unwrap()).to_string(),
            "Shift+Tab"
        );
    }

    #[test]
    fn keysym_names_become_symbols() {
        let us = layout("us");
        assert!(same(&us, "Super+parenleft", "Super+("));
        assert!(same(&us, "Ctrl+asciitilde", "Ctrl+Shift+`"));
    }
}
