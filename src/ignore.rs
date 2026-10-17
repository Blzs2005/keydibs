//! Clashes you've decided to live with, listed in `~/.config/keydibs/ignore`.
//!
//! One rule per line: the keys as keydibs prints them, optionally followed by
//! the tools the rule is about. `#` starts a comment.
//!
//! ```text
//! Shift+Right             # every clash on Shift+Right
//! Ctrl+Shift+Up konsole   # only the ones Konsole is part of
//! Ctrl+X, Ctrl+E fish
//! ```
//!
//! A rule on a key also covers the sequences that start with it.

use std::path::PathBuf;

use crate::adapters::config_home;
use crate::binding::Binding;
use crate::chord::Chord;
use crate::conflict::Conflict;
use crate::layout::Layout;
use crate::reach::Unreachable;

/// Where the ignore list lives: `$XDG_CONFIG_HOME/keydibs/ignore`.
pub fn path() -> Option<PathBuf> {
    config_home().map(|dir| dir.join("keydibs").join("ignore"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub keys: Vec<Chord>,
    /// Tool names; empty for every tool.
    pub tools: Vec<String>,
    /// 1-based line in the file.
    pub line: usize,
}

/// A line that isn't a rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadLine {
    pub line: usize,
    pub message: String,
}

/// Reads the rules in `text`. `tools` are the tool names a rule can end with.
pub fn parse(text: &str, tools: &[&str]) -> (Vec<Rule>, Vec<BadLine>) {
    let mut rules = Vec::new();
    let mut bad = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let mut words: Vec<&str> = without_comment(line).split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        let mut rule_tools = Vec::new();
        while let Some(last) = words.last()
            && let Some(tool) = tools.iter().find(|t| t.eq_ignore_ascii_case(last))
        {
            rule_tools.insert(0, tool.to_string());
            words.pop();
        }
        let keys: Result<Vec<Chord>, _> = words
            .join(" ")
            .split(", ")
            .map(|chord| chord.parse::<Chord>())
            .collect();
        match keys {
            Ok(keys) => rules.push(Rule {
                keys,
                tools: rule_tools,
                line: i + 1,
            }),
            Err(e) => bad.push(BadLine {
                line: i + 1,
                message: e.to_string(),
            }),
        }
    }
    (rules, bad)
}

/// `line` up to a `#` that starts it or follows a space. `Ctrl+#` is a key.
fn without_comment(line: &str) -> &str {
    let mut previous = ' ';
    for (i, c) in line.char_indices() {
        if c == '#' && previous.is_whitespace() {
            return &line[..i];
        }
        previous = c;
    }
    line
}

impl Rule {
    /// Rewrites the keys the way bindings are compared on `layout`.
    pub fn normalize(&mut self, layout: &Layout) {
        for chord in &mut self.keys {
            *chord = layout.normalize(chord);
        }
    }

    pub fn covers_conflict(&self, conflict: &Conflict) -> bool {
        match conflict {
            Conflict::Shadowed { winner, loser } => self.covers(loser, &[winner.tool, loser.tool]),
            Conflict::Duplicate { first, .. } => self.covers(first, &[first.tool]),
        }
    }

    pub fn covers_unreachable(&self, unreachable: &Unreachable) -> bool {
        self.covers(unreachable.binding, &[unreachable.binding.tool])
    }

    /// Whether the rule is about `binding`'s keys and one of `tools`.
    fn covers(&self, binding: &Binding, tools: &[&str]) -> bool {
        let keys: Vec<&Chord> = std::iter::once(&binding.chord)
            .chain(&binding.then)
            .collect();
        self.keys.len() <= keys.len()
            && self.keys.iter().zip(keys).all(|(a, b)| a == b)
            && (self.tools.is_empty() || tools.iter().any(|t| self.tools.iter().any(|r| r == t)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binding::{Grabs, Layer, Source};

    const TOOLS: &[&str] = &["konsole", "fish", "micro"];

    fn bind(keys: &str, tool: &'static str, layer: Layer) -> Binding {
        let mut chords = keys.split(", ").map(|c| c.parse::<Chord>().unwrap());
        Binding {
            chord: chords.next().unwrap(),
            action: "something".into(),
            tool,
            layer,
            grabs: Grabs::Always,
            mode: None,
            then: chords.collect(),
            source: Source::Default,
        }
    }

    fn rule(line: &str) -> Rule {
        let (rules, bad) = parse(line, TOOLS);
        assert!(bad.is_empty(), "{bad:?}");
        rules.into_iter().next().unwrap()
    }

    #[test]
    fn rules_are_keys_then_tools() {
        let text = "\
# I don't split Konsole's view
Ctrl+Shift+Up  Konsole   # not micro's
shift+right

Ctrl+X, Ctrl+E fish micro
Ctrl+# ";
        let (rules, bad) = parse(text, TOOLS);
        assert!(bad.is_empty());
        assert_eq!(rules.len(), 4);
        assert_eq!(rules[0].keys, ["ctrl+shift+up".parse().unwrap()]);
        assert_eq!(rules[0].tools, ["konsole"]);
        assert_eq!(rules[0].line, 2);
        assert!(rules[1].tools.is_empty());
        assert_eq!(rules[2].keys.len(), 2);
        assert_eq!(rules[2].tools, ["fish", "micro"]);
        assert_eq!(rules[3].keys, ["ctrl+#".parse().unwrap()]);
    }

    #[test]
    fn bad_lines_are_reported() {
        let (rules, bad) = parse("Hyper+X\nCtrl+\nAlt+F", TOOLS);
        assert_eq!(rules.len(), 1);
        assert_eq!(
            bad,
            [
                BadLine {
                    line: 1,
                    message: "unknown modifier `Hyper`".into()
                },
                BadLine {
                    line: 2,
                    message: "chord has no key".into()
                },
            ]
        );
    }

    #[test]
    fn a_rule_covers_its_keys_and_tools() {
        let konsole = bind("Shift+Right", "konsole", Layer::Terminal);
        let fish = bind("Shift+Right", "fish", Layer::Shell);
        let micro = bind("Shift+Right", "micro", Layer::App);
        let shadowed = |loser| Conflict::Shadowed {
            winner: &konsole,
            loser,
        };

        assert!(rule("Shift+Right").covers_conflict(&shadowed(&fish)));
        assert!(rule("Shift+Right konsole").covers_conflict(&shadowed(&fish)));
        assert!(rule("Shift+Right micro").covers_conflict(&shadowed(&micro)));
        assert!(!rule("Shift+Right micro").covers_conflict(&shadowed(&fish)));
        assert!(!rule("Shift+Left").covers_conflict(&shadowed(&fish)));
    }

    #[test]
    fn a_rule_on_a_key_covers_sequences_starting_with_it() {
        let sequence = bind("Ctrl+X, Ctrl+E", "fish", Layer::Shell);
        assert!(rule("Ctrl+X").covers(&sequence, &["fish"]));
        assert!(rule("Ctrl+X, Ctrl+E").covers(&sequence, &["fish"]));
        assert!(!rule("Ctrl+X, Ctrl+F").covers(&sequence, &["fish"]));
        let single = bind("Ctrl+X", "fish", Layer::Shell);
        assert!(!rule("Ctrl+X, Ctrl+E").covers(&single, &["fish"]));
    }
}
