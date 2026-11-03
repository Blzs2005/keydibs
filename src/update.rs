//! Telling the user a newer keydibs is out, and installing it (`keydibs update`).
//!
//! The newest version comes from keydibs' file in the crates.io index, at most
//! once a day: the answer is kept in `~/.cache/keydibs/update-check`. Releases
//! go to crates.io, GitHub and the AUR together, so that's the newest version
//! however keydibs was installed.
//!
//! Set `KEYDIBS_NO_UPDATE_CHECK=1` (or `DO_NOT_TRACK=1`) to turn the check off.
//! It's also off when `CI` is set.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The version running now.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// keydibs' file in the crates.io sparse index: one JSON line per published version.
const INDEX: &str = "https://index.crates.io/ke/yd/keydibs";

const RELEASES: &str = "https://github.com/Blzs2005/keydibs/releases/latest";

/// How long an answer from crates.io is good for.
const CHECK_EVERY: u64 = 24 * 60 * 60;

/// A released version, like `0.8.2`. Pre-releases don't parse, so they're
/// never offered as an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    /// The version running now, unless it's a pre-release.
    pub fn current() -> Option<Version> {
        CURRENT.parse().ok()
    }
}

impl FromStr for Version {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        let number = |part: Option<&str>| match part {
            Some(part) if !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) => {
                part.parse().map_err(|_| ())
            }
            _ => Err(()),
        };
        let mut parts = s.split('.');
        let version = Version {
            major: number(parts.next())?,
            minor: number(parts.next())?,
            patch: number(parts.next())?,
        };
        match parts.next() {
            None => Ok(version),
            Some(_) => Err(()),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The newest version in keydibs' crates.io index file that isn't yanked.
pub fn newest(index: &str) -> Option<Version> {
    #[derive(Deserialize)]
    struct Entry {
        vers: String,
        #[serde(default)]
        yanked: bool,
    }
    index
        .lines()
        .filter_map(|line| serde_json::from_str::<Entry>(line).ok())
        .filter(|entry| !entry.yanked)
        .filter_map(|entry| entry.vers.parse().ok())
        .max()
}

/// What the terminal says when there's a newer version.
pub fn notice(newer: Version) -> String {
    format!(
        "note: an update is available (keydibs {newer}, you have {CURRENT}); run keydibs update to install it"
    )
}

/// Whether to look for a newer version, given the environment.
pub fn enabled() -> bool {
    enabled_with(|name| std::env::var_os(name))
}

fn enabled_with(var: impl Fn(&str) -> Option<OsString>) -> bool {
    let set = |name| var(name).is_some_and(|v| !v.is_empty() && v != "0" && v != "false");
    !(set("KEYDIBS_NO_UPDATE_CHECK") || set("DO_NOT_TRACK") || set("CI"))
}

/// The newest version, if it's newer than this one. Asks crates.io when the
/// last answer is more than a day old, waiting at most `timeout`. `None` when
/// the check is off, offline, or there's nothing newer.
pub fn check(timeout: Duration) -> Option<Version> {
    if !enabled() {
        return None;
    }
    let current = Version::current()?;
    let newest = newest_cached(&cache_path()?, now(), || fetch(INDEX, timeout))?;
    (newest > current).then_some(newest)
}

/// [`check`] on a thread of its own, so it doesn't hold anything up.
pub fn check_in_background(
    timeout: Duration,
    done: impl FnOnce() + Send + 'static,
) -> Receiver<Option<Version>> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(check(timeout));
        done();
    });
    receive
}

/// What we last heard from crates.io, and when.
#[derive(Serialize, Deserialize)]
struct Checked {
    /// Seconds since the Unix epoch.
    at: u64,
    newest: Option<String>,
}

/// The newest version, from the cache at `path` if it's fresh, otherwise from
/// `fetch`. A failed fetch keeps the last answer and isn't retried for a day,
/// so being offline doesn't slow every run down.
fn newest_cached(
    path: &Path,
    now: u64,
    fetch: impl FnOnce() -> Result<String, String>,
) -> Option<Version> {
    let cached: Option<Checked> = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok());
    let parse = |checked: &Checked| checked.newest.as_deref().and_then(|v| v.parse().ok());
    if let Some(checked) = &cached
        && now >= checked.at
        && now - checked.at < CHECK_EVERY
    {
        return parse(checked);
    }
    let newest = match fetch() {
        Ok(index) => newest(&index),
        Err(_) => cached.as_ref().and_then(parse),
    };
    let checked = Checked {
        at: now,
        newest: newest.map(|v| v.to_string()),
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, serde_json::to_string(&checked).unwrap_or_default());
    newest
}

/// The body at `url`, waiting at most `timeout` for all of it.
fn fetch(url: &str, timeout: Duration) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent(concat!(
            "keydibs/",
            env!("CARGO_PKG_VERSION"),
            " (https://github.com/Blzs2005/keydibs)"
        ))
        .build()
        .into();
    agent
        .get(url)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .with_config()
        .limit(1 << 20)
        .read_to_string()
        .map_err(|e| e.to_string())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `$XDG_CACHE_HOME/keydibs/update-check`, falling back to `~/.cache`.
fn cache_path() -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(cache.join("keydibs/update-check"))
}

/// How the running keydibs was installed, which decides how to update it.
#[derive(Debug, PartialEq, Eq)]
pub enum Install {
    /// With `cargo install` or `cargo binstall`, into `root/bin`.
    Cargo {
        root: PathBuf,
        /// Built with `--no-default-features`: no app.
        cli_only: bool,
    },
    /// A development build, in a `target` directory.
    Dev(PathBuf),
    /// By the system's package manager, like the AUR's keydibs-bin.
    Package(PathBuf),
    /// Anywhere else, like a binary unpacked from the releases page.
    Other(PathBuf),
}

impl Install {
    /// How the keydibs that's running was installed.
    pub fn detect() -> Install {
        let exe = std::env::current_exe().unwrap_or_default();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let roots = [
            std::env::var_os("CARGO_INSTALL_ROOT").map(PathBuf::from),
            std::env::var_os("CARGO_HOME").map(PathBuf::from),
            home.map(|home| home.join(".cargo")),
        ];
        let roots: Vec<PathBuf> = roots
            .into_iter()
            .flatten()
            .filter(|root| !root.as_os_str().is_empty())
            .map(|root| std::fs::canonicalize(&root).unwrap_or(root))
            .collect();
        Install::of(&exe, &roots)
    }

    /// How the keydibs at `exe` was installed, when cargo installs into `roots`.
    fn of(exe: &Path, roots: &[PathBuf]) -> Install {
        let dir = exe.parent().unwrap_or(Path::new(""));
        if let Some(root) = roots.iter().find(|root| dir == root.join("bin")) {
            return Install::Cargo {
                root: root.clone(),
                cli_only: cli_only(&root.join(".crates2.json")),
            };
        }
        if exe.components().any(|part| part.as_os_str() == "target") {
            Install::Dev(exe.to_path_buf())
        } else if exe.starts_with("/usr") {
            Install::Package(exe.to_path_buf())
        } else {
            Install::Other(exe.to_path_buf())
        }
    }
}

/// Whether cargo's record of what it installed (`.crates2.json`) says keydibs
/// was built without the app.
fn cli_only(crates2: &Path) -> bool {
    #[derive(Deserialize)]
    struct Installed {
        installs: std::collections::HashMap<String, Options>,
    }
    #[derive(Deserialize)]
    struct Options {
        #[serde(default)]
        features: Vec<String>,
        #[serde(default)]
        all_features: bool,
        #[serde(default)]
        no_default_features: bool,
    }
    let Some(installed) = std::fs::read_to_string(crates2)
        .ok()
        .and_then(|text| serde_json::from_str::<Installed>(&text).ok())
    else {
        return false;
    };
    installed
        .installs
        .iter()
        .find(|(package, _)| package.starts_with("keydibs "))
        .is_some_and(|(_, options)| {
            options.no_default_features
                && !options.all_features
                && !options.features.iter().any(|f| f == "gui")
        })
}

/// What `keydibs update` does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Run a command that installs the newest version over this one.
    Run(Run),
    /// keydibs can't update this copy itself; says how to.
    Explain(String),
}

impl Plan {
    /// How to update the keydibs that's running.
    pub fn detect() -> Plan {
        let install = Install::detect();
        let binstall = match &install {
            Install::Cargo { root, .. } => find("cargo-binstall", &root.join("bin")),
            _ => None,
        };
        Plan::new(&install, binstall)
    }

    /// How to update `install`, with cargo-binstall at `binstall`, if it's installed.
    pub fn new(install: &Install, binstall: Option<PathBuf>) -> Plan {
        match (install, binstall) {
            (
                Install::Cargo {
                    cli_only: false, ..
                },
                Some(binstall),
            ) => Plan::Run(Run {
                program: binstall.into(),
                args: vec!["--no-confirm".into(), "keydibs".into()],
                shown: "cargo binstall --no-confirm keydibs".into(),
            }),
            (Install::Cargo { cli_only, .. }, _) => {
                let mut args = vec!["install", "--locked", "keydibs"];
                if *cli_only {
                    args.push("--no-default-features");
                }
                Plan::Run(Run::new("cargo", &args))
            }
            (Install::Dev(exe), _) => Plan::Explain(format!(
                "{} is a development build; update it with git pull and cargo build",
                crate::report::tilde(exe)
            )),
            (Install::Package(exe), _) => Plan::Explain(format!(
                "{} came from your package manager, so update it there \
                 (for the AUR's keydibs-bin, with your AUR helper, like yay -Syu)",
                crate::report::tilde(exe)
            )),
            (Install::Other(exe), _) => Plan::Explain(format!(
                "{} wasn't installed with cargo, so keydibs can't update it; \
                 download the new version from {RELEASES}, or install it with cargo binstall keydibs",
                crate::report::tilde(exe)
            )),
        }
    }
}

/// A command that updates keydibs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    program: OsString,
    args: Vec<OsString>,
    /// How it's shown to the user.
    shown: String,
}

impl Run {
    pub fn new(program: &str, args: &[&str]) -> Run {
        let mut shown = program.to_string();
        for arg in args {
            shown.push(' ');
            shown.push_str(arg);
        }
        Run {
            program: program.into(),
            args: args.iter().map(OsString::from).collect(),
            shown,
        }
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        command
    }

    /// Runs it with its output captured, for the app. On failure, the error is
    /// its last line of output.
    pub fn run_quietly(&self) -> Result<(), String> {
        let output = self
            .command()
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("can't run {self}: {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        let last_line = |bytes: &[u8]| {
            String::from_utf8_lossy(bytes)
                .lines()
                .map(str::trim)
                .rfind(|line| !line.is_empty())
                .map(str::to_string)
        };
        Err(last_line(&output.stderr)
            .or_else(|| last_line(&output.stdout))
            .unwrap_or_else(|| format!("{self} failed ({})", output.status)))
    }
}

impl fmt::Display for Run {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.shown)
    }
}

/// `program` in `$PATH`, or else in `fallback`.
fn find(program: &str, fallback: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain([fallback.to_path_buf()])
        .map(|dir| dir.join(program))
        .find(|file| {
            std::fs::metadata(file)
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Instant;

    use super::*;

    fn v(s: &str) -> Version {
        s.parse().unwrap()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("keydibs-update-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn versions_parse_and_compare_by_number() {
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert_eq!(v("0.8.2").to_string(), "0.8.2");
        for bad in [
            "0.9.0-rc.1",
            "1.2",
            "1.2.3.4",
            "1.2.x",
            "",
            "1..2",
            "+1.2.3",
            "1.2.3+build",
        ] {
            assert!(bad.parse::<Version>().is_err(), "{bad} parsed");
        }
        assert!(
            Version::current().is_some(),
            "Cargo.toml's version doesn't parse"
        );
    }

    #[test]
    fn newest_skips_yanked_versions_pre_releases_and_junk() {
        let index = r#"{"name":"keydibs","vers":"0.9.0","yanked":false}
{"name":"keydibs","vers":"0.10.0","yanked":false}
{"name":"keydibs","vers":"0.11.0","yanked":true}
{"name":"keydibs","vers":"0.12.0-rc.1","yanked":false}
not json
{"name":"keydibs","vers":"0.8.2"}
"#;
        assert_eq!(newest(index), Some(v("0.10.0")));
        assert_eq!(newest(""), None);
    }

    #[test]
    fn the_notice_says_how_to_update() {
        let text = notice(v("9.0.0"));
        assert!(text.contains("keydibs 9.0.0"), "{text}");
        assert!(text.contains(CURRENT), "{text}");
        assert!(text.contains("run keydibs update"), "{text}");
    }

    #[test]
    fn the_check_can_be_turned_off() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| OsString::from(v))
            }
        };
        assert!(enabled_with(env(&[])));
        assert!(!enabled_with(env(&[("KEYDIBS_NO_UPDATE_CHECK", "1")])));
        assert!(!enabled_with(env(&[("DO_NOT_TRACK", "1")])));
        assert!(!enabled_with(env(&[("CI", "true")])));
        assert!(enabled_with(env(&[
            ("KEYDIBS_NO_UPDATE_CHECK", "0"),
            ("CI", "")
        ])));
        assert!(enabled_with(env(&[("DO_NOT_TRACK", "false")])));
    }

    #[test]
    fn crates_io_is_asked_at_most_once_a_day() {
        let path = temp_dir("cache").join("keydibs/update-check");
        let index = || Ok(r#"{"vers":"0.9.0"}"#.to_string());
        let never = || -> Result<String, String> { panic!("fetched with a fresh cache") };
        let day = CHECK_EVERY;

        // No cache yet: asks, and remembers.
        assert_eq!(newest_cached(&path, 1000, index), Some(v("0.9.0")));
        assert_eq!(
            newest_cached(&path, 1000 + day - 1, never),
            Some(v("0.9.0"))
        );

        // A day later it asks again.
        let newer = || Ok(r#"{"vers":"0.9.1"}"#.to_string());
        assert_eq!(newest_cached(&path, 1000 + day, newer), Some(v("0.9.1")));

        // Offline: keeps the last answer, and doesn't try again for a day.
        let offline = || Err("no network".to_string());
        assert_eq!(
            newest_cached(&path, 1000 + 3 * day, offline),
            Some(v("0.9.1"))
        );
        assert_eq!(
            newest_cached(&path, 1000 + 4 * day - 1, never),
            Some(v("0.9.1"))
        );

        // A clock that went backwards, or a broken cache file, means asking again.
        assert_eq!(newest_cached(&path, 10, index), Some(v("0.9.0")));
        std::fs::write(&path, "{ broken").unwrap();
        assert_eq!(newest_cached(&path, 20, newer), Some(v("0.9.1")));
        std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
    }

    /// Serves one HTTP request on localhost with `respond`, and returns its URL.
    fn serve(respond: impl FnOnce(std::net::TcpStream) + Send + 'static) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/ke/yd/keydibs", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request);
            respond(stream);
        });
        url
    }

    #[test]
    fn fetch_reads_the_index_over_http() {
        let url = serve(|mut stream| {
            let body = "{\"vers\":\"0.8.2\"}\n{\"vers\":\"0.9.0\"}\n";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        });
        let index = fetch(&url, Duration::from_secs(5)).unwrap();
        assert_eq!(newest(&index), Some(v("0.9.0")));
    }

    #[test]
    fn fetch_fails_on_an_error_status() {
        let url = serve(|mut stream| {
            let _ = stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        });
        assert!(fetch(&url, Duration::from_secs(5)).is_err());
    }

    #[test]
    fn fetch_gives_up_on_a_server_that_never_answers() {
        let url = serve(|stream| {
            std::thread::sleep(Duration::from_secs(5));
            drop(stream);
        });
        let started = Instant::now();
        assert!(fetch(&url, Duration::from_millis(300)).is_err());
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn the_install_is_told_apart_by_where_keydibs_is() {
        let root = temp_dir("install");
        let roots = [root.clone()];
        assert_eq!(
            Install::of(&root.join("bin/keydibs"), &roots),
            Install::Cargo {
                root: root.clone(),
                cli_only: false
            }
        );
        let dev = Path::new("/home/me/keydibs/target/debug/keydibs");
        assert_eq!(Install::of(dev, &roots), Install::Dev(dev.into()));
        let aur = Path::new("/usr/bin/keydibs");
        assert_eq!(Install::of(aur, &roots), Install::Package(aur.into()));
        let other = Path::new("/home/me/.local/bin/keydibs");
        assert_eq!(Install::of(other, &roots), Install::Other(other.into()));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_cli_only_install_is_read_from_cargos_record() {
        let root = temp_dir("crates2");
        let record = |no_default_features: bool, features: &str| {
            let json = format!(
                r#"{{"installs":{{"other 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)":{{"no_default_features":true}},
                "keydibs 0.8.2 (registry+https://github.com/rust-lang/crates.io-index)":{{"features":[{features}],"all_features":false,"no_default_features":{no_default_features}}}}}}}"#
            );
            std::fs::write(root.join(".crates2.json"), json).unwrap();
            Install::of(&root.join("bin/keydibs"), std::slice::from_ref(&root))
        };
        let cli_only = |cli_only| Install::Cargo {
            root: root.clone(),
            cli_only,
        };
        assert_eq!(record(false, ""), cli_only(false));
        assert_eq!(record(true, ""), cli_only(true));
        assert_eq!(record(true, r#""gui""#), cli_only(false));
        std::fs::write(root.join(".crates2.json"), "not json").unwrap();
        assert_eq!(
            Install::of(&root.join("bin/keydibs"), std::slice::from_ref(&root)),
            cli_only(false)
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cargo_installs_update_with_binstall_when_they_can() {
        let cargo = |cli_only| Install::Cargo {
            root: "/home/me/.cargo".into(),
            cli_only,
        };
        let binstall = Some(PathBuf::from("/home/me/.cargo/bin/cargo-binstall"));
        let Plan::Run(run) = Plan::new(&cargo(false), binstall.clone()) else {
            panic!("no command")
        };
        assert_eq!(run.to_string(), "cargo binstall --no-confirm keydibs");
        assert_eq!(run.program, "/home/me/.cargo/bin/cargo-binstall");
        assert_eq!(run.args, ["--no-confirm", "keydibs"]);

        // Without binstall, or for a build without the app (which binstall's
        // ready-made binaries always have), cargo builds it.
        let shown = |plan| match plan {
            Plan::Run(run) => run.to_string(),
            Plan::Explain(text) => panic!("{text}"),
        };
        assert_eq!(
            shown(Plan::new(&cargo(false), None)),
            "cargo install --locked keydibs"
        );
        assert_eq!(
            shown(Plan::new(&cargo(true), binstall)),
            "cargo install --locked keydibs --no-default-features"
        );
    }

    #[test]
    fn other_installs_are_told_how_to_update() {
        let explain = |install| match Plan::new(&install, Some("/usr/bin/cargo-binstall".into())) {
            Plan::Explain(text) => text,
            Plan::Run(run) => panic!("would run {run}"),
        };
        assert!(
            explain(Install::Dev("/src/keydibs/target/debug/keydibs".into()))
                .contains("development build")
        );
        assert!(explain(Install::Package("/usr/bin/keydibs".into())).contains("package manager"));
        assert!(explain(Install::Other("/opt/keydibs".into())).contains(RELEASES));
    }

    #[test]
    fn a_failed_update_reports_its_last_line() {
        let fails = Run::new(
            "sh",
            &[
                "-c",
                "echo working; echo 'error: no network' >&2; echo >&2; exit 3",
            ],
        );
        assert_eq!(fails.run_quietly(), Err("error: no network".to_string()));
        assert_eq!(Run::new("true", &[]).run_quietly(), Ok(()));
        let silent = Run::new("false", &[]).run_quietly().unwrap_err();
        assert!(silent.starts_with("false failed"), "{silent}");
        let missing = Run::new("keydibs-no-such-program", &[])
            .run_quietly()
            .unwrap_err();
        assert!(
            missing.starts_with("can't run keydibs-no-such-program"),
            "{missing}"
        );
    }
}
