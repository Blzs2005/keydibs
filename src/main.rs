use std::process::ExitCode;

use clap::{Parser, Subcommand};
use keydibs::conflict::{self, Conflict};
use keydibs::ignore;
use keydibs::layout::Layout;
use keydibs::report::{self, Finding, Problem, source, tilde};
use keydibs::scan::{self, Event, Found};
use keydibs::{Binding, Chord};

#[cfg(feature = "gui")]
mod desktop;
#[cfg(feature = "gui")]
mod gui;

#[derive(Parser)]
#[command(
    version,
    about = "Find keyboard shortcuts that clash between your desktop, terminal, shell and editor"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Report clashes and shortcuts that can't arrive (the default). Exits 1 if any are found.
    Check {
        /// Also report the clashes listed in ~/.config/keydibs/ignore
        #[arg(long)]
        no_ignore: bool,
    },
    /// Show every layer that binds a chord, in the order a key press reaches them
    Why {
        /// e.g. `ctrl+shift+c` or `Meta+E`
        chord: Chord,
    },
    /// List every binding keydibs found
    List,
    /// Open the keydibs app
    #[cfg(feature = "gui")]
    Gui,
    /// Add keydibs to your app launcher, so the app opens like any other
    #[cfg(feature = "gui")]
    InstallDesktop {
        /// Take it out of the app launcher again
        #[arg(long)]
        remove: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Command::Check { no_ignore: false });
    #[cfg(feature = "gui")]
    match command {
        Command::Gui => return gui::run(),
        Command::InstallDesktop { remove } => return desktop::install(remove),
        _ => {}
    }

    let mut found = scan::load(|event| match event {
        Event::Scanned(tool) => {
            eprintln!(
                "scanned {} ({}): {} bindings",
                tool.name, tool.layer, tool.bindings
            );
            for note in &tool.notes {
                eprintln!("  note: {note}");
            }
        }
        Event::Skipped(skipped) => {
            eprintln!("warning: skipping {}: {}", skipped.tool, skipped.error)
        }
    });
    let layout = match Layout::load() {
        Ok(layout) => {
            let name = if layout.name.is_empty() {
                "default"
            } else {
                &layout.name
            };
            eprintln!("keyboard layout: {name}");
            found.normalize(&layout);
            Some(layout)
        }
        Err(e) => {
            eprintln!("note: {e}");
            None
        }
    };
    let ignore = match command {
        Command::Check { no_ignore: false } => ignore_list(layout.as_ref()),
        _ => None,
    };
    eprintln!();

    match command {
        Command::Check { .. } => check(&found, ignore.as_ref()),
        Command::Why { chord } => {
            let chord = match &layout {
                Some(layout) => layout.normalize(&chord),
                None => chord,
            };
            why(&found.bindings, &chord);
            ExitCode::SUCCESS
        }
        Command::List => {
            list(&found.bindings);
            ExitCode::SUCCESS
        }
        #[cfg(feature = "gui")]
        Command::Gui | Command::InstallDesktop { .. } => unreachable!("handled above"),
    }
}

/// The ignore list, if there is one. Lines that aren't rules are reported and skipped.
fn ignore_list(layout: Option<&Layout>) -> Option<ignore::List> {
    let path = ignore::path()?;
    let list = match ignore::load(&path, layout) {
        Ok(list) => list?,
        Err(e) => {
            eprintln!("warning: can't read {}: {e}", tilde(&path));
            return None;
        }
    };
    for line in &list.bad {
        eprintln!(
            "warning: skipping {}:{}: {}",
            tilde(&list.path),
            line.line,
            line.message
        );
    }
    Some(list)
}

fn check(found: &Found, ignore: Option<&ignore::List>) -> ExitCode {
    let rules = ignore.map_or(&[][..], |list| &list.rules[..]);
    let findings = report::check(found, rules);
    let (ignored, shown): (Vec<&Finding>, Vec<&Finding>) =
        findings.iter().partition(|f| f.is_ignored());

    let mut conflicts = 0;
    let mut unreachable = 0;
    for finding in &shown {
        match &finding.problem {
            Problem::Conflict(Conflict::Shadowed { winner, loser }) => println!(
                "shadowed   {}\n  {} \"{}\" ({})\n  is blocked by {} \"{}\" ({})\n",
                loser.keys(),
                loser.tool,
                loser.describe(),
                source(&loser.source),
                winner.tool,
                winner.describe(),
                source(&winner.source),
            ),
            Problem::Conflict(Conflict::Duplicate { first, second }) => println!(
                "duplicate  {}\n  {} binds it to \"{}\" ({})\n  and to \"{}\" ({})\n",
                first.keys(),
                first.tool,
                first.describe(),
                source(&first.source),
                second.describe(),
                source(&second.source),
            ),
            Problem::Unreachable(u) => println!(
                "can't arrive {}\n  {} \"{}\" ({})\n  {}\n",
                u.binding.keys(),
                u.binding.tool,
                u.binding.describe(),
                source(&u.binding.source),
                report::explain(u),
            ),
        }
        match finding.problem {
            Problem::Conflict(_) => conflicts += 1,
            Problem::Unreachable(_) => unreachable += 1,
        }
    }

    let mut problems = Vec::new();
    if conflicts > 0 {
        problems.push(plural(conflicts, "conflict", "conflicts"));
    }
    if unreachable > 0 {
        problems.push(plural(
            unreachable,
            "shortcut that can't arrive",
            "shortcuts that can't arrive",
        ));
    }
    let hidden = if ignored.is_empty() {
        String::new()
    } else {
        format!(" ({} ignored)", ignored.len())
    };
    let bindings = found.bindings.len();
    let code = if problems.is_empty() {
        println!("no conflicts in {bindings} bindings{hidden}");
        ExitCode::SUCCESS
    } else {
        println!("{} in {bindings} bindings{hidden}", problems.join(" and "));
        ExitCode::FAILURE
    };

    // A rule that matches nothing is a typo, or a clash that's been fixed since.
    if let Some(list) = ignore {
        for rule in report::unused(&list.rules, &findings) {
            eprintln!("note: {}:{} matches nothing", tilde(&list.path), rule.line);
        }
    }
    code
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn why(bindings: &[Binding], chord: &Chord) {
    println!("{chord}");
    let chain = conflict::chain(bindings, chord);
    if chain.is_empty() {
        println!("  nothing binds this");
        return;
    }
    for (b, count) in conflict::folded(&chain) {
        let shadowers = conflict::shadowed_by(&chain, b);
        let verdict = if shadowers.is_empty() {
            b.grabs.to_string()
        } else {
            let tools: Vec<_> = shadowers.iter().map(|s| s.tool).collect();
            format!("SHADOWED by {}", tools.join(", "))
        };
        let (keys, action, set_in) = match count {
            1 if b.then.is_empty() => (String::new(), b.describe(), source(&b.source)),
            1 => (format!("[{}] ", b.keys()), b.describe(), source(&b.source)),
            _ => (
                format!("[{}, …] ", b.chord),
                format!("{count} shortcuts that start with these keys"),
                "keydibs list shows them".to_string(),
            ),
        };
        println!(
            "  {:<11} {:<10} {:<34} {keys}{action}  ({set_in})",
            b.layer, b.tool, verdict,
        );
    }
}

fn list(bindings: &[Binding]) {
    let mut sorted: Vec<_> = bindings.iter().collect();
    sorted.sort_by(|a, b| (a.layer, &a.chord, &a.then).cmp(&(b.layer, &b.chord, &b.then)));
    for b in sorted {
        println!("{:<24} {:<10} {}", b.keys(), b.tool, b.describe());
    }
}
