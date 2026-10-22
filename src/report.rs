//! What `keydibs check` reports, and the wording the CLI and the app share.

use std::path::Path;

use crate::binding::{Binding, Source};
use crate::conflict::{self, Conflict};
use crate::ignore::Rule;
use crate::reach::{self, Unreachable};
use crate::scan::Found;

/// Something worth telling the user about.
#[derive(Debug)]
pub enum Problem<'a> {
    Conflict(Conflict<'a>),
    Unreachable(Unreachable<'a>),
}

impl Problem<'_> {
    /// The binding that loses out: the one that's blocked or can't arrive, or
    /// the first of a duplicate.
    pub fn binding(&self) -> &Binding {
        match self {
            Problem::Conflict(Conflict::Shadowed { loser, .. }) => loser,
            Problem::Conflict(Conflict::Duplicate { first, .. }) => first,
            Problem::Unreachable(u) => u.binding,
        }
    }

    /// Every tool that's part of it.
    pub fn tools(&self) -> Vec<&'static str> {
        match self {
            Problem::Conflict(Conflict::Shadowed { winner, loser }) => {
                vec![loser.tool, winner.tool]
            }
            _ => vec![self.binding().tool],
        }
    }

    fn ignored_by(&self, rule: &Rule) -> bool {
        match self {
            Problem::Conflict(c) => rule.covers_conflict(c),
            Problem::Unreachable(u) => rule.covers_unreachable(u),
        }
    }
}

/// A problem, and the ignore rules that cover it.
#[derive(Debug)]
pub struct Finding<'a> {
    pub problem: Problem<'a>,
    pub ignored_by: Vec<&'a Rule>,
}

impl Finding<'_> {
    pub fn is_ignored(&self) -> bool {
        !self.ignored_by.is_empty()
    }
}

/// Every conflict, then every shortcut that can't arrive, each with the rules
/// in `rules` that cover it.
pub fn check<'a>(found: &'a Found, rules: &'a [Rule]) -> Vec<Finding<'a>> {
    let conflicts = conflict::find(&found.bindings)
        .into_iter()
        .map(Problem::Conflict);
    let unreachable = reach::unreachable(&found.bindings, &found.terminals, &found.kitty_programs)
        .into_iter()
        .map(Problem::Unreachable);
    conflicts
        .chain(unreachable)
        .map(|problem| Finding {
            ignored_by: rules.iter().filter(|r| problem.ignored_by(r)).collect(),
            problem,
        })
        .collect()
}

/// The rules that cover none of `findings`: typos, or clashes fixed since.
pub fn unused<'a>(rules: &'a [Rule], findings: &[Finding]) -> impl Iterator<Item = &'a Rule> {
    rules.iter().filter(|rule| {
        !findings
            .iter()
            .any(|f| f.ignored_by.iter().any(|r| r.line == rule.line))
    })
}

/// Why a shortcut can't arrive, in the terms of the terminals that were found.
pub fn explain(u: &Unreachable) -> String {
    let list = |tools: &[&str]| tools.join(" and ");
    let sends = |tools: &[&str]| match tools {
        [] => "terminals send".to_string(),
        [one] => format!("{one} sends"),
        many => format!("{} send", list(many)),
    };
    let sent = format!("{} it as {}", sends(&u.fails_in), u.sent_as);
    if !u.program_asks {
        format!(
            "{sent}, because {} doesn't use the kitty keyboard protocol",
            u.binding.tool
        )
    } else if u.works_in.is_empty() {
        format!("{sent}; it needs a terminal with the kitty keyboard protocol")
    } else {
        let has = if u.works_in.len() == 1 { "has" } else { "have" };
        format!(
            "{sent}; it only arrives in {}, which {has} the kitty keyboard protocol",
            list(&u.works_in)
        )
    }
}

/// Where a binding was made: `~/.config/micro/bindings.json:2`, `built-in default`…
pub fn source(source: &Source) -> String {
    match source {
        Source::Config {
            path,
            line: Some(line),
        } => format!("{}:{line}", tilde(path)),
        Source::Config { path, line: None } => tilde(path),
        Source::Default => "built-in default".to_string(),
        Source::Runtime => "set at runtime".to_string(),
    }
}

/// Shows paths under the home directory as `~/...`.
pub fn tilde(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME")
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binding::{Grabs, Layer};
    use crate::ignore;

    fn bind(keys: &str, action: &str, tool: &'static str, layer: Layer) -> Binding {
        Binding {
            chord: keys.parse().unwrap(),
            action: action.into(),
            tool,
            layer,
            grabs: Grabs::Always,
            mode: None,
            then: Vec::new(),
            source: Source::Default,
        }
    }

    #[test]
    fn findings_carry_the_rules_that_cover_them() {
        let found = Found {
            bindings: vec![
                bind("Shift+Right", "SelectRight", "micro", Layer::App),
                bind("Shift+Right", "Next Tab", "konsole", Layer::Terminal),
                bind("Ctrl+Shift+S", "SaveAll", "micro", Layer::App),
            ],
            ..Found::default()
        };
        let (rules, _) = ignore::parse("Shift+Right konsole\nShift+Right", &["konsole"]);
        let findings = check(&found, &rules);

        assert_eq!(findings.len(), 2);
        assert!(matches!(
            findings[0].problem,
            Problem::Conflict(Conflict::Shadowed { .. })
        ));
        assert_eq!(findings[0].problem.tools(), ["micro", "konsole"]);
        assert_eq!(findings[0].ignored_by.len(), 2);

        // No terminal was found, so it can't arrive anywhere.
        assert!(matches!(findings[1].problem, Problem::Unreachable(_)));
        assert_eq!(findings[1].problem.binding().action, "SaveAll");
        assert!(!findings[1].is_ignored());
    }
}
