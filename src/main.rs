use std::path::Path;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use keydibs::conflict::{self, Conflict};
use keydibs::{Binding, Chord, Source, adapters};

#[derive(Parser)]
#[command(
    version,
    about = "Find keybindings that never fire because another layer grabs them first"
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
    let bindings = load_all();

    match cli.command.unwrap_or(Command::Check) {
        Command::Check => check(&bindings),
        Command::Why { chord } => {
            why(&bindings, &chord);
            ExitCode::SUCCESS
        }
        Command::List => {
            list(&bindings);
            ExitCode::SUCCESS
        }
    }
}

fn load_all() -> Vec<Binding> {
    let mut all = Vec::new();
    for adapter in adapters::all() {
        match adapter.load() {
            Ok(Some(bindings)) => {
                eprintln!(
                    "scanned {} ({}): {} bindings",
                    adapter.tool(),
                    adapter.layer(),
                    bindings.len()
                );
                all.extend(bindings);
            }
            Ok(None) => {}
            Err(e) => eprintln!("warning: skipping {}: {e:#}", adapter.tool()),
        }
    }
    eprintln!();
    all
}

fn check(bindings: &[Binding]) -> ExitCode {
    let conflicts = conflict::find(bindings);
    for c in &conflicts {
        match c {
            Conflict::Shadowed { winner, loser } => println!(
                "shadowed   {}\n  {} \"{}\" ({}) never fires:\n  {} \"{}\" ({}) grabs {} first\n",
                loser.keys(),
                loser.tool,
                loser.describe(),
                source(&loser.source),
                winner.tool,
                winner.describe(),
                source(&winner.source),
                winner.chord,
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

    if conflicts.is_empty() {
        println!("no conflicts in {} bindings", bindings.len());
        ExitCode::SUCCESS
    } else {
        println!(
            "{} conflicts in {} bindings",
            conflicts.len(),
            bindings.len()
        );
        ExitCode::FAILURE
    }
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
