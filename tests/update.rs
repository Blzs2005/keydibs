//! `keydibs update`, run as the real binary: a copy of it "installed" into a
//! cargo root in a temporary directory, with a stand-in `cargo-binstall` that
//! records how it was called instead of downloading anything.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A cargo root with keydibs in its `bin`, and a stand-in cargo-binstall
/// (on the `PATH` it returns) that runs `script`.
fn cargo_root(name: &str, script: &str) -> (PathBuf, String) {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("update-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let keydibs = bin.join("keydibs");
    // A hard link is instant; the debug binary is big to copy.
    if std::fs::hard_link(env!("CARGO_BIN_EXE_keydibs"), &keydibs).is_err() {
        std::fs::copy(env!("CARGO_BIN_EXE_keydibs"), &keydibs).unwrap();
    }

    let tools = root.join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    let binstall = tools.join("cargo-binstall");
    std::fs::write(&binstall, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&binstall, std::fs::Permissions::from_mode(0o755)).unwrap();

    let path = format!(
        "{}:{}",
        tools.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    (root, path)
}

fn update(keydibs: &Path, root: &Path, path: &str) -> Output {
    Command::new(keydibs)
        .arg("update")
        .env("CARGO_INSTALL_ROOT", root)
        .env("PATH", path)
        .env("HOME", root)
        .output()
        .unwrap()
}

#[test]
fn a_cargo_install_updates_with_cargo_binstall() {
    let (root, path) = cargo_root("binstall", r#"echo "$@" > "$(dirname "$0")/called""#);
    let output = update(&root.join("bin/keydibs"), &root, &path);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "{stderr}");
    assert!(
        stderr.contains("running cargo binstall --no-confirm keydibs"),
        "{stderr}"
    );
    let called = std::fs::read_to_string(root.join("tools/called")).unwrap();
    assert_eq!(called.trim(), "--no-confirm keydibs");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_failed_update_fails_keydibs_update() {
    let (root, path) = cargo_root("binstall-fails", "echo 'no network' >&2; exit 3");
    let output = update(&root.join("bin/keydibs"), &root, &path);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success(), "{stderr}");
    assert!(stderr.contains("no network"), "{stderr}");
    assert!(
        stderr.contains("error: cargo binstall --no-confirm keydibs failed"),
        "{stderr}"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_development_build_isnt_updated() {
    let (root, path) = cargo_root("dev", "touch \"$(dirname \"$0\")/called\"");
    // The build in target/, not the copy in the cargo root.
    let output = update(Path::new(env!("CARGO_BIN_EXE_keydibs")), &root, &path);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success(), "{stderr}");
    assert!(stderr.contains("is a development build"), "{stderr}");
    assert!(!root.join("tools/called").exists(), "cargo-binstall ran");
    std::fs::remove_dir_all(&root).unwrap();
}
