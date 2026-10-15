use std::path::Path;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use keydibs::conflict::{self, Conflict};
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
    /// Report shadowed and duplicate bindings (the default). Exits 1 if any are found.
    Check,
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
    eprintln!();

    match cli.command.unwrap_or(Command::Check) {
        Command::Check => check(&found),
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

fn check(found: &Found) -> ExitCode {
    let bindings = &found.bindings;
    let conflicts = conflict::find(bindings);
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

    let unreachable = reach::unreachable(bindings, &found.terminals, &found.kitty_programs);
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
    if problems.is_empty() {
        println!("no conflicts in {} bindings", bindings.len());
        ExitCode::SUCCESS
    } else {
        println!("{} in {} bindings", problems.join(" and "), bindings.len());
        ExitCode::FAILURE
    }
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
