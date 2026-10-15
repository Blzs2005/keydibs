use std::collections::BTreeMap;

use crate::binding::{Binding, Grabs};
use crate::chord::Chord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict<'a> {
    /// `loser` is blocked: `winner`, in a layer above it, grabs the chord first.
    Shadowed {
        winner: &'a Binding,
        loser: &'a Binding,
    },
    /// One tool binds the same chord to two different actions; only one can win.
    Duplicate {
        first: &'a Binding,
        second: &'a Binding,
    },
}

/// Every binding for `chord`, in the order the key press reaches them.
pub fn chain<'a>(bindings: &'a [Binding], chord: &Chord) -> Vec<&'a Binding> {
    let mut hits: Vec<_> = bindings.iter().filter(|b| &b.chord == chord).collect();
    // Within a layer, bindings that actually take the key come first.
    hits.sort_by_key(|b| (b.layer, b.grabs == Grabs::Never));
    hits
}

/// The bindings in `chain` (as returned by [`chain`]) that take the key away
/// before it reaches `target`: those in the topmost layer that can grab it,
/// one per tool. The key travels down, so that layer gets it first.
///
/// Tools in the same layer are alternatives (you run Alacritty *or* Konsole), so
/// they never shadow each other.
pub fn shadowed_by<'a>(chain: &[&'a Binding], target: &Binding) -> Vec<&'a Binding> {
    let mut found: Vec<&Binding> = Vec::new();
    for b in chain {
        if b.layer >= target.layer {
            break;
        }
        if !b.layer.is_above(target.layer) || !b.grabs.can_shadow(target.layer) {
            continue;
        }
        if found.first().is_some_and(|f| f.layer != b.layer) {
            break;
        }
        if !found.iter().any(|f| f.tool == b.tool) {
            found.push(b);
        }
    }
    found
}

/// Finds every conflict among `bindings`.
pub fn find(bindings: &[Binding]) -> Vec<Conflict<'_>> {
    let mut by_chord: BTreeMap<&Chord, Vec<&Binding>> = BTreeMap::new();
    for b in bindings {
        by_chord.entry(&b.chord).or_default().push(b);
    }

    let mut conflicts = Vec::new();
    for group in by_chord.values_mut() {
        group.sort_by_key(|b| b.layer);

        for loser in group.iter() {
            for winner in shadowed_by(group, loser) {
                conflicts.push(Conflict::Shadowed { winner, loser });
            }
        }

        for (i, first) in group.iter().enumerate() {
            for second in &group[i + 1..] {
                if first.tool == second.tool
                    && first.mode == second.mode
                    && first.then == second.then
                    && first.action != second.action
                    && first.grabs.overlaps(second.grabs)
                {
                    conflicts.push(Conflict::Duplicate { first, second });
                }
            }
        }
    }
    conflicts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binding::{Layer, Source};

    fn bind(chord: &str, action: &str, tool: &'static str, layer: Layer) -> Binding {
        Binding {
            chord: chord.parse().unwrap(),
            action: action.into(),
            tool,
            layer,
            grabs: Grabs::Always,
            mode: None,
            then: Vec::new(),
            source: Source::Default,
        }
    }

    fn grabbing(grabs: Grabs, b: Binding) -> Binding {
        Binding { grabs, ..b }
    }

    #[test]
    fn upper_layer_shadows_lower() {
        let bs = [
            bind("Ctrl+Shift+C", "Copy line", "micro", Layer::App),
            bind("ctrl+shift+c", "Copy", "alacritty", Layer::Terminal),
        ];
        assert_eq!(
            find(&bs),
            vec![Conflict::Shadowed {
                winner: &bs[1],
                loser: &bs[0]
            }]
        );
    }

    #[test]
    fn same_layer_tools_are_alternatives() {
        let bs = [
            bind("Ctrl+Shift+T", "New tab", "alacritty", Layer::Terminal),
            bind("Ctrl+Shift+T", "New tab", "konsole", Layer::Terminal),
        ];
        assert!(find(&bs).is_empty());
    }

    #[test]
    fn each_tool_in_the_nearest_layer_shadows() {
        let bs = [
            bind("Ctrl+R", "History search", "fish", Layer::Shell),
            bind("Ctrl+R", "Reload", "alacritty", Layer::Terminal),
            bind("Ctrl+R", "Rename", "konsole", Layer::Terminal),
        ];
        let conflicts = find(&bs);
        assert_eq!(conflicts.len(), 2);
        assert!(
            conflicts
                .iter()
                .all(|c| matches!(c, Conflict::Shadowed { loser, .. } if loser.tool == "fish"))
        );
    }

    #[test]
    fn the_topmost_grabbing_layer_gets_the_key() {
        let bs = [
            bind("Meta+X", "Run", "fish", Layer::Shell),
            bind("Meta+X", "Thing", "alacritty", Layer::Terminal),
            bind("Meta+X", "Menu", "kde", Layer::Compositor),
        ];
        let fish = &bs[0];
        let chain = chain(&bs, &fish.chord);
        let tools: Vec<_> = shadowed_by(&chain, fish).iter().map(|b| b.tool).collect();
        assert_eq!(tools, ["kde"]);

        // A terminal binding that only grabs in full-screen apps can't take
        // the key from the shell.
        let bs = [
            bind("Meta+X", "Run", "fish", Layer::Shell),
            grabbing(
                Grabs::AltScreen,
                bind("Meta+X", "Thing", "alacritty", Layer::Terminal),
            ),
        ];
        let chain = super::chain(&bs, &bs[0].chord);
        assert!(shadowed_by(&chain, &bs[0]).is_empty());
    }

    #[test]
    fn screen_conditions_decide_who_is_shadowed() {
        let bs = [
            grabbing(
                Grabs::NormalScreen,
                bind("Shift+PageUp", "Scroll", "alacritty", Layer::Terminal),
            ),
            bind("Shift+PageUp", "Select page", "fish", Layer::Shell),
            bind("Shift+PageUp", "Select page", "micro", Layer::App),
        ];
        assert_eq!(
            find(&bs),
            vec![Conflict::Shadowed {
                winner: &bs[0],
                loser: &bs[1]
            }]
        );
    }

    #[test]
    fn shell_and_apps_take_turns() {
        let bs = [
            bind("Ctrl+R", "History search", "fish", Layer::Shell),
            bind("Ctrl+R", "Replace", "micro", Layer::App),
        ];
        assert!(find(&bs).is_empty());
    }

    #[test]
    fn bindings_that_pass_the_key_on_shadow_nothing() {
        let bs = [
            grabbing(
                Grabs::Never,
                bind("Ctrl+L", "Clear notice", "alacritty", Layer::Terminal),
            ),
            bind("Ctrl+L", "Clear screen", "fish", Layer::Shell),
        ];
        assert!(find(&bs).is_empty());
    }

    #[test]
    fn duplicates_within_one_tool() {
        let bs = [
            bind("Meta+L", "Lock Session", "kde", Layer::Compositor),
            bind("Meta+L", "Toggle Layout", "kde", Layer::Compositor),
        ];
        assert_eq!(
            find(&bs),
            vec![Conflict::Duplicate {
                first: &bs[0],
                second: &bs[1]
            }]
        );
    }

    #[test]
    fn no_duplicate_when_conditions_never_overlap() {
        let bs = [
            grabbing(
                Grabs::NormalScreen,
                bind("Shift+Home", "Scroll to top", "alacritty", Layer::Terminal),
            ),
            grabbing(
                Grabs::AltScreen,
                bind("Shift+Home", "Something", "alacritty", Layer::Terminal),
            ),
        ];
        assert!(find(&bs).is_empty());
    }

    #[test]
    fn chain_is_in_layer_order() {
        let bs = [
            bind("Ctrl+D", "Delete", "micro", Layer::App),
            bind("Ctrl+D", "Exit", "fish", Layer::Shell),
            bind("Ctrl+E", "Other", "kde", Layer::Compositor),
        ];
        let tools: Vec<_> = chain(&bs, &"ctrl+d".parse().unwrap())
            .iter()
            .map(|b| b.tool)
            .collect();
        assert_eq!(tools, ["fish", "micro"]);
    }
}
