//! `keydibs install-desktop`: adds the app to the desktop's app launcher, with
//! a `.desktop` entry and an icon in the user's data directory.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use keydibs::adapters::data_home;
use keydibs::report::tilde;

const ICON: &str = include_str!("../assets/keydibs.svg");

pub fn install(remove: bool) -> ExitCode {
    let Some(data) = data_home() else {
        eprintln!("error: can't find your home directory");
        return ExitCode::FAILURE;
    };
    let entry = data.join("applications/keydibs.desktop");
    let icon = data.join("icons/hicolor/scalable/apps/keydibs.svg");
    let result = if remove {
        uninstall(&[&entry, &icon])
    } else {
        write(&entry, &icon)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn write(entry: &Path, icon: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("can't find keydibs itself: {e}"))?;
    for (path, text) in [(icon, ICON.to_string()), (entry, desktop_entry(&exe))] {
        create(path, &text).map_err(|e| format!("can't write {}: {e}", tilde(path)))?;
    }
    println!("Added keydibs to your app launcher ({}).", tilde(entry));
    println!("It opens {} gui.", tilde(&exe));
    if exe.components().any(|part| part.as_os_str() == "target") {
        println!("note: that's a development build; run this again from the keydibs you install");
    }
    Ok(())
}

fn create(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text)
}

fn uninstall(paths: &[&PathBuf]) -> Result<(), String> {
    for path in paths {
        match std::fs::remove_file(path) {
            Ok(()) => println!("Removed {}", tilde(path)),
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(format!("can't remove {}: {e}", tilde(path))),
        }
    }
    Ok(())
}

fn desktop_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]
Type=Application
Name=keydibs
GenericName=Shortcut Clash Finder
Comment=Find keyboard shortcuts that clash between your desktop, terminal, shell and editor
Exec={} gui
Icon=keydibs
Terminal=false
Categories=Utility;
Keywords=keyboard;shortcuts;keybindings;hotkeys;clash;conflict;
StartupWMClass=keydibs
",
        exec_arg(&exe.to_string_lossy())
    )
}

/// Quotes `arg` for an `Exec` line. The Desktop Entry spec escapes `"`, `` ` ``,
/// `$` and `\` inside quotes, then escapes every `\` once more because the
/// value is a string, and writes `%` as `%%`.
fn exec_arg(arg: &str) -> String {
    let mut quoted = String::from("\"");
    for c in arg.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                quoted.push('\\');
                quoted.push(c);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted.replace('\\', "\\\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_paths_are_quoted() {
        assert_eq!(
            exec_arg("/home/me/.cargo/bin/keydibs"),
            r#""/home/me/.cargo/bin/keydibs""#
        );
        assert_eq!(
            exec_arg("/opt/my apps/keydibs"),
            r#""/opt/my apps/keydibs""#
        );
        assert_eq!(exec_arg(r#"/a"b$c%d"#), r#""/a\\"b\\$c%%d""#);
    }

    #[test]
    fn entry_opens_the_app() {
        let entry = desktop_entry(Path::new("/usr/bin/keydibs"));
        assert!(entry.contains("Exec=\"/usr/bin/keydibs\" gui\n"));
        assert!(entry.contains("StartupWMClass=keydibs\n"));
    }
}
