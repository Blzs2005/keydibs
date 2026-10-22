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

/// Whether `b` is a sequence that starts on a key its own tool also takes by
/// itself, like tmux's `Ctrl+B, C` after its prefix key `Ctrl+B`. Whatever
/// happens to that key happens to the whole sequence, so it's reported once,
/// for the key.
pub fn led_by_own_key<'a>(b: &Binding, mut others: impl Iterator<Item = &'a Binding>) -> bool {
    !b.then.is_empty()
        && others.any(|lead| {
            lead.then.is_empty()
                && lead.chord == b.chord
                && lead.tool == b.tool
                && lead.mode == b.mode
                && lead.grabs != Grabs::Never
        })
}

/// More sequences than this, of one tool in one mode, are folded into one row.
const MAX_SEQUENCES: usize = 3;

/// `chain` (as returned by [`chain`]) as rows to show, each with how many
/// bindings it stands for. When one tool has more than [`MAX_SEQUENCES`]
/// sequences in one mode that start with the key, like the dozens behind tmux's
/// prefix key, they're folded into the first of them.
pub fn folded<'a>(chain: &[&'a Binding]) -> Vec<(&'a Binding, usize)> {
    let together = |a: &Binding, b: &Binding| {
        !a.then.is_empty()
            && !b.then.is_empty()
            && a.tool == b.tool
            && a.mode == b.mode
            && a.grabs == b.grabs
    };
    let mut rows: Vec<(&Binding, usize)> = Vec::new();
    for &b in chain {
        let count = chain.iter().filter(|other| together(other, b)).count();
        if count <= MAX_SEQUENCES {
            rows.push((b, 1));
        } else if !rows.iter().any(|(first, _)| together(first, b)) {
            rows.push((b, count));
        }
    }
    rows
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
            if led_by_own_key(loser, group.iter().copied()) {
                continue;
            }
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

    // The later keys of a sequence travel down the layers too, so a layer above
    // can take one of them: Konsole's Alt+1 blocks tmux's `Ctrl+B, Alt+1`.
    for loser in bindings.iter().filter(|b| !b.then.is_empty()) {
        if !shadowed_by(&by_chord[&loser.chord], loser).is_empty() {
            continue;
        }
        let blocked = loser.then.iter().find_map(|key| {
            let winners = shadowed_by(by_chord.get(key)?, loser);
            (!winners.is_empty()).then_some(winners)
        });
        for winner in blocked.unwrap_or_default() {
            conflicts.push(Conflict::Shadowed { winner, loser });
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
    fn sequences_after_a_blocked_key_are_reported_once() {
        let sequence = |then: &str| Binding {
            then: vec![then.parse().unwrap()],
            ..bind("Ctrl+B", then, "tmux", Layer::Multiplexer)
        };
        let bs = [
            bind("Ctrl+B", "Something", "konsole", Layer::Terminal),
            bind("Ctrl+B", "prefix key", "tmux", Layer::Multiplexer),
            sequence("c"),
            sequence("n"),
            // A sequence with no binding on its first key is still reported.
            Binding {
                then: vec!["ctrl+e".parse().unwrap()],
                ..bind("Ctrl+B", "edit", "fish", Layer::Shell)
            },
        ];
        let losers: Vec<_> = find(&bs)
            .iter()
            .map(|c| match c {
                Conflict::Shadowed { loser, .. } => loser.keys(),
                Conflict::Duplicate { .. } => panic!("no duplicates here"),
            })
            .collect();
        assert_eq!(losers, ["Ctrl+B", "Ctrl+B, Ctrl+E"]);
    }

    #[test]
    fn later_keys_of_a_sequence_can_be_taken() {
        let sequence = |first: &str, then: &str| Binding {
            then: vec![then.parse().unwrap()],
            ..bind(first, "select-layout", "tmux", Layer::Multiplexer)
        };
        let bs = [
            bind("Alt+1", "Switch to Tab 1", "konsole", Layer::Terminal),
            bind("Ctrl+B", "prefix key", "tmux", Layer::Multiplexer),
            sequence("Ctrl+B", "Alt+1"),
            sequence("Ctrl+B", "Alt+2"),
            // Its first key is blocked already, which says it all.
            bind("Alt+2", "Switch to Tab 2", "konsole", Layer::Terminal),
            sequence("Alt+2", "Alt+1"),
        ];
        assert_eq!(
            find(&bs),
            vec![
                Conflict::Shadowed {
                    winner: &bs[4],
                    loser: &bs[5]
                },
                Conflict::Shadowed {
                    winner: &bs[0],
                    loser: &bs[2]
                },
                Conflict::Shadowed {
                    winner: &bs[4],
                    loser: &bs[3]
                },
            ]
        );
    }

    #[test]
    fn many_sequences_of_one_tool_fold_together() {
        let sequence = |then: &str, tool| Binding {
            then: vec![then.parse().unwrap()],
            ..bind("Ctrl+B", then, tool, Layer::Multiplexer)
        };
        let mut bs = vec![
            bind("Ctrl+B", "prefix key", "tmux", Layer::Multiplexer),
            bind("Ctrl+B", "backward-char", "fish", Layer::Shell),
        ];
        bs.extend(["c", "n", "p", "d"].map(|key| sequence(key, "tmux")));
        bs.push(sequence("x", "other"));
        let chain = chain(&bs, &"ctrl+b".parse().unwrap());
        let rows: Vec<_> = folded(&chain)
            .iter()
            .map(|(b, count)| (b.keys(), *count))
            .collect();
        assert_eq!(
            rows,
            [
                ("Ctrl+B".to_string(), 1),
                ("Ctrl+B, C".to_string(), 4),
                ("Ctrl+B, X".to_string(), 1),
                ("Ctrl+B".to_string(), 1),
            ]
        );
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
