use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use keydibs::conflict::{self, Conflict};
use keydibs::ignore::{self, Rule};
use keydibs::layout::Layout;
use keydibs::{Binding, Chord, Layer, Source, adapters, reach};

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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let mut found = load_all();
    let layout = match Layout::load() {
        Ok(layout) => {
            let name = if layout.name.is_empty() {
                "default"
            } else {
                &layout.name
            };
            eprintln!("keyboard layout: {name}");
            for binding in &mut found.bindings {
                layout.normalize_binding(binding);
            }
            Some(layout)
        }
        Err(e) => {
            eprintln!("note: {e}");
            None
        }
    };
    let command = cli.command.unwrap_or(Command::Check { no_ignore: false });
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
    }
}

/// What was found on this machine.
struct Found {
    bindings: Vec<Binding>,
    terminals: Vec<reach::Terminal>,
    /// Shells and apps that turn on the kitty keyboard protocol.
    kitty_programs: Vec<&'static str>,
}

fn load_all() -> Found {
    let mut found = Found {
        bindings: Vec::new(),
        terminals: Vec::new(),
        kitty_programs: Vec::new(),
    };
    for adapter in adapters::all() {
        match adapter.load() {
            Ok(Some(bindings)) => {
                eprintln!(
                    "scanned {} ({}): {} bindings",
                    adapter.tool(),
                    adapter.layer(),
                    bindings.len()
                );
                for note in adapter.notes() {
                    eprintln!("  note: {note}");
                }
                match adapter.layer() {
                    Layer::Terminal => found.terminals.push(reach::Terminal {
                        tool: adapter.tool(),
                        kitty_keyboard: adapter.kitty_keyboard(),
                    }),
                    Layer::Compositor => {}
                    _ if adapter.kitty_keyboard() => found.kitty_programs.push(adapter.tool()),
                    _ => {}
                }
                found.bindings.extend(bindings);
            }
            Ok(None) => {}
            Err(e) => eprintln!("warning: skipping {}: {e:#}", adapter.tool()),
        }
    }
    found
}

/// The rules in the ignore list, with its path, if there is one. Lines that
/// aren't rules are reported and skipped.
fn ignore_list(layout: Option<&Layout>) -> Option<(PathBuf, Vec<Rule>)> {
    let path = ignore::path()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return None,
        Err(e) => {
            eprintln!("warning: can't read {}: {e}", tilde(&path));
            return None;
        }
    };
    let tools: Vec<&str> = adapters::all().iter().map(|a| a.tool()).collect();
    let (mut rules, bad) = ignore::parse(&text, &tools);
    for line in bad {
        eprintln!(
            "warning: skipping {}:{}: {}",
            tilde(&path),
            line.line,
            line.message
        );
    }
    if let Some(layout) = layout {
        for rule in &mut rules {
            rule.normalize(layout);
        }
    }
    Some((path, rules))
}

fn check(found: &Found, ignore: Option<&(PathBuf, Vec<Rule>)>) -> ExitCode {
    let bindings = &found.bindings;
    let rules = ignore.map_or(&[][..], |(_, rules)| rules);
    let mut used = vec![false; rules.len()];
    let mut ignored = 0;
    let mut is_ignored = |covers: &dyn Fn(&Rule) -> bool| {
        let mut hit = false;
        for (rule, used) in rules.iter().zip(&mut used) {
            if covers(rule) {
                *used = true;
                hit = true;
            }
        }
        ignored += usize::from(hit);
        hit
    };
    let conflicts: Vec<_> = conflict::find(bindings)
        .into_iter()
        .filter(|c| !is_ignored(&|rule| rule.covers_conflict(c)))
        .collect();
    let unreachable: Vec<_> = reach::unreachable(bindings, &found.terminals, &found.kitty_programs)
        .into_iter()
        .filter(|u| !is_ignored(&|rule| rule.covers_unreachable(u)))
        .collect();

    for c in &conflicts {
        match c {
            Conflict::Shadowed { winner, loser } => println!(
                "shadowed   {}\n  {} \"{}\" ({})\n  is blocked by {} \"{}\" ({})\n",
                loser.keys(),
                loser.tool,
                loser.describe(),
                source(&loser.source),
                winner.tool,
                winner.describe(),
                source(&winner.source),
            ),
            Conflict::Duplicate { first, second } => println!(
                "duplicate  {}\n  {} binds it to \"{}\" ({})\n  and to \"{}\" ({})\n",
                first.keys(),
                first.tool,
                first.describe(),
                source(&first.source),
                second.describe(),
                source(&second.source),
            ),
        }
    }

    for u in &unreachable {
        println!(
            "can't arrive {}\n  {} \"{}\" ({})\n  {}\n",
            u.binding.keys(),
            u.binding.tool,
            u.binding.describe(),
            source(&u.binding.source),
            explain(u),
        );
    }

    let mut problems = Vec::new();
    if !conflicts.is_empty() {
        problems.push(plural(conflicts.len(), "conflict", "conflicts"));
    }
    if !unreachable.is_empty() {
        problems.push(plural(
            unreachable.len(),
            "shortcut that can't arrive",
            "shortcuts that can't arrive",
        ));
    }
    let hidden = if ignored > 0 {
        format!(" ({ignored} ignored)")
    } else {
        String::new()
    };
    let code = if problems.is_empty() {
        println!("no conflicts in {} bindings{hidden}", bindings.len());
        ExitCode::SUCCESS
    } else {
        println!(
            "{} in {} bindings{hidden}",
            problems.join(" and "),
            bindings.len()
        );
        ExitCode::FAILURE
    };

    // A rule that matches nothing is a typo, or a clash that's been fixed since.
    if let Some((path, rules)) = ignore {
        for (rule, used) in rules.iter().zip(&used) {
            if !used {
                eprintln!("note: {}:{} matches nothing", tilde(path), rule.line);
            }
        }
    }
    code
}

/// Why a shortcut can't arrive, in the terms of the terminals that were found.
fn explain(u: &reach::Unreachable) -> String {
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
        format!(
            "{sent}; it only arrives in {}, which has the kitty keyboard protocol",
            list(&u.works_in)
        )
    }
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
    for b in &chain {
        let shadowers = conflict::shadowed_by(&chain, b);
        let verdict = if shadowers.is_empty() {
            b.grabs.to_string()
        } else {
            let tools: Vec<_> = shadowers.iter().map(|s| s.tool).collect();
            format!("SHADOWED by {}", tools.join(", "))
        };
        let keys = if b.then.is_empty() {
            String::new()
        } else {
            format!("[{}] ", b.keys())
        };
        println!(
            "  {:<11} {:<10} {:<34} {keys}{}  ({})",
            b.layer,
            b.tool,
            verdict,
            b.describe(),
            source(&b.source)
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

fn source(source: &Source) -> String {
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
fn tilde(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME")
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}
