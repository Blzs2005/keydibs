//! One module per tool. Each reads the tool's config (plus its built-in
//! defaults, where it has them) and turns it into [`Binding`]s.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::bail;

use crate::binding::{Binding, Layer};

pub mod alacritty;
mod escape;
pub mod fish;
pub mod hyprland;
pub mod kde;
pub mod konsole;
pub mod micro;
pub mod nvim;

pub trait Adapter {
    fn tool(&self) -> &'static str;
    fn layer(&self) -> Layer;
    /// The tool's bindings, or `Ok(None)` when the tool isn't set up on this machine.
    fn load(&self) -> anyhow::Result<Option<Vec<Binding>>>;
    /// Things worth telling the user about the results, like an installed version
    /// that differs from the one the built-in shortcuts come from.
    fn notes(&self) -> Vec<String> {
        Vec::new()
    }
    /// For a terminal: whether it supports the kitty keyboard protocol, which can
    /// send every key combination. For a shell or app: whether it turns it on.
    fn kitty_keyboard(&self) -> bool {
        false
    }
}

pub fn all() -> Vec<Box<dyn Adapter>> {
    vec![
        Box::new(kde::Kde),
        Box::new(hyprland::Hyprland),
        Box::new(alacritty::Alacritty),
        Box::new(konsole::Konsole),
        Box::new(fish::Fish),
        Box::new(micro::Micro),
        Box::new(nvim::Nvim::default()),
    ]
}

/// Whether an executable called `name` is on `$PATH`.
pub(crate) fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
}

/// `$XDG_DATA_HOME`, falling back to `~/.local/share`.
pub(crate) fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
}

/// `$XDG_CONFIG_HOME`, falling back to `~/.config`.
pub(crate) fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

/// How long a tool gets to answer before keydibs gives up on it. A shell config
/// that waits for input or the network shouldn't hang keydibs.
pub(crate) const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs `command` with stdin closed, giving up after `timeout`.
pub(crate) fn run(command: &mut Command, timeout: Duration) -> anyhow::Result<Output> {
    let deadline = Instant::now() + timeout;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = read_in_background(child.stdout.take());
    let stderr = read_in_background(child.stderr.take());

    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("gave up after {} seconds", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    // Something the command started in the background can keep its output open
    // after it exits, so don't wait for the pipes past the deadline either.
    let collect = |pipe: mpsc::Receiver<Vec<u8>>| {
        let left = deadline.saturating_duration_since(Instant::now());
        pipe.recv_timeout(left.max(Duration::from_millis(100)))
    };
    match (collect(stdout), collect(stderr)) {
        (Ok(stdout), Ok(stderr)) => Ok(Output {
            status,
            stdout,
            stderr,
        }),
        _ => bail!("gave up after {} seconds", timeout.as_secs()),
    }
}

fn read_in_background(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        let _ = send.send(bytes);
    });
    receive
}

/// A note when the installed version of a tool (as printed by `program args`
/// and picked out by `find`) isn't the one its built-in shortcuts come from.
/// Versions match on as many parts as `expected` has: `0.17` matches `0.17.2`.
pub(crate) fn version_note(
    name: &str,
    program: &str,
    args: &[&str],
    expected: &str,
    find: fn(&str) -> Option<&str>,
) -> Option<String> {
    if !on_path(program) {
        return None;
    }
    let output = run(Command::new(program).args(args), COMMAND_TIMEOUT).ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let installed = find(&text)?.trim();
    (!same_version(installed, expected)).then(|| {
        format!(
            "{name}'s built-in shortcuts are from version {expected}, but {installed} is installed, so a few may differ"
        )
    })
}

/// Picks the version out of output like `alacritty 0.17.0 (94e7c887)`.
pub(crate) fn second_word(output: &str) -> Option<&str> {
    output.split_whitespace().nth(1)
}

fn same_version(installed: &str, expected: &str) -> bool {
    let want: Vec<&str> = expected.split('.').collect();
    let have: Vec<&str> = installed.split('.').collect();
    have.len() >= want.len() && have[..want.len()] == want[..]
}

/// Long actions (inline shell commands, say) get cut to this many characters.
pub(crate) const MAX_ACTION_LEN: usize = 60;

pub(crate) fn shorten(action: &str) -> String {
    if action.chars().count() <= MAX_ACTION_LEN {
        action.to_string()
    } else {
        let cut: String = action.chars().take(MAX_ACTION_LEN - 1).collect();
        format!("{}…", cut.trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_collects_output() {
        let output = run(
            Command::new("sh").args(["-c", "echo out; echo err >&2"]),
            COMMAND_TIMEOUT,
        )
        .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"out\n");
        assert_eq!(output.stderr, b"err\n");
    }

    #[test]
    fn run_gives_up_on_a_hanging_command() {
        let started = Instant::now();
        let error = run(Command::new("sleep").arg("10"), Duration::from_millis(200)).unwrap_err();
        assert!(error.to_string().contains("gave up"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn run_does_not_wait_for_background_children() {
        // The shell exits at once, but its background `sleep` keeps stdout open.
        let started = Instant::now();
        let result = run(
            Command::new("sh").args(["-c", "sleep 10 & echo done"]),
            Duration::from_millis(300),
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(result.is_err());
    }

    #[test]
    fn versions_match_on_the_expected_parts() {
        assert!(same_version("0.17.0", "0.17"));
        assert!(same_version("26.08.2", "26.08"));
        assert!(same_version("2.0.15", "2.0.15"));
        assert!(!same_version("0.18.1", "0.17"));
        assert!(!same_version("2.0.14", "2.0.15"));
        assert!(!same_version("0", "0.17"));
    }
}
