//! Packaging and installing a WM pair never switches or overwrites the user's
//! default window manager: $XDG_STATE_HOME/sophia/bin/hagia (the user-owned
//! policy client that the reload workflow replaces) stays byte-identical,
//! with the same mode and mtime, and nothing in the release or the install
//! links to it. The installed session may legitimately reference that path:
//! it prefers the user's client when one is present and falls back to the
//! packaged pair otherwise, reading it and never writing it.
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use xtask::package_desktop::assemble;

#[path = "support/release_fixture.rs"]
mod fixture;
use fixture::{Dir, repo, script, sha256};

/// Every entry below `dir`: (sha256 or link target, mode, mtime).
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, (String, u32, i64)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let meta = fs::symlink_metadata(&path).unwrap();
        let content = if meta.is_dir() {
            for entry in fs::read_dir(&path).unwrap() {
                stack.push(entry.unwrap().path());
            }
            String::new()
        } else if meta.is_symlink() {
            fs::read_link(&path).unwrap().display().to_string()
        } else {
            sha256(&fs::read(&path).unwrap())
        };
        out.insert(
            path,
            (
                content,
                meta.mode(),
                meta.mtime() * 1_000_000_000 + meta.mtime_nsec(),
            ),
        );
    }
    out
}

fn user_state(root: &Path) -> (PathBuf, PathBuf) {
    let state = root.join("home/.local/state");
    let bin = state.join("sophia/bin");
    fs::create_dir_all(&bin).unwrap();
    fs::set_permissions(state.join("sophia"), fs::Permissions::from_mode(0o700)).unwrap();
    let wm = bin.join("hagia");
    fs::write(
        &wm,
        "#!/bin/sh\n# the user's own reloadable policy client\n",
    )
    .unwrap();
    fs::set_permissions(&wm, fs::Permissions::from_mode(0o700)).unwrap();
    (state, wm)
}

/// Every symlink below `dir` whose target resolves inside `forbidden`.
fn links_into(dir: &Path, forbidden: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_symlink() {
            if fs::canonicalize(&path).is_ok_and(|t| t.starts_with(forbidden)) {
                found.push(path);
            }
        } else if meta.is_dir() {
            for entry in fs::read_dir(&path).unwrap() {
                stack.push(entry.unwrap().path());
            }
        }
    }
    found
}

#[test]
fn packaging_and_installing_leave_the_users_default_wm_untouched() {
    let dir = Dir::new("wm-default");
    let (state, user_wm) = user_state(&dir.0);
    let before = snapshot(&state);

    let assembly = fixture::assembly(&dir.0);
    assemble(&assembly).unwrap();
    let prefix = dir.0.join("prefix");
    let output = Command::new(repo().join("tools/install_live_session.sh"))
        .arg(&assembly.out)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir.0.join("home"))
        .env("XDG_STATE_HOME", &state)
        .env("SOPHIA_INSTALL_PREFIX", &prefix)
        .env("SOPHIA_SESSION_DIR", dir.0.join("sessions"))
        .env("SOPHIA_COMMAND_DIR", dir.0.join("commands"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    assert_eq!(
        snapshot(&state),
        before,
        "packaging or installing changed user WM state"
    );
    let state_real = fs::canonicalize(state).unwrap();
    for root in [
        &assembly.out,
        &prefix,
        &dir.0.join("commands"),
        &dir.0.join("sessions"),
    ] {
        assert!(
            links_into(root, &state_real).is_empty(),
            "{} links into the user's state",
            root.display()
        );
    }
    // The packaged pair is the release's own copy, not the user's client.
    let packaged = prefix.join("current/target/release/hagia");
    assert_eq!(
        sha256(&fs::read(&packaged).unwrap()),
        assembly.pair.hagia_sha256
    );
    assert_ne!(fs::read(packaged).unwrap(), fs::read(user_wm).unwrap());
}

/// A minimal release around the real installed launcher whose Sophia stub
/// records the policy client it was handed.
fn launcher_release(root: &Path) -> PathBuf {
    let release = root.join("release");
    for sub in ["bin", "tools/lib", "tools/session", "target/release"] {
        fs::create_dir_all(release.join(sub)).unwrap();
    }
    fs::copy(
        repo().join("tools/installed/sophia-session"),
        release.join("bin/sophia-session"),
    )
    .unwrap();
    fs::write(release.join("tools/lib/session_lifecycle.sh"), "").unwrap();
    fs::write(
        release.join("manifest"),
        format!("version=0.1.0\ncommit={}\n", "0".repeat(40)),
    )
    .unwrap();
    for name in ["hagia", "narthex", "sophia-wm-demo"] {
        script(&release.join("target/release").join(name), "# packaged");
    }
    script(
        &release.join("target/release/sophia"),
        "printf '%s\\n' \"$SOPHIA_HAGIA_BIN\" >\"$CAPTURE\"",
    );
    release
}

fn selected_client(release: &Path, state: &Path, capture: &Path) -> String {
    let _ = fs::remove_file(capture);
    let output = Command::new(release.join("bin/sophia-session"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", state.parent().unwrap())
        .env("XDG_STATE_HOME", state)
        .env("CAPTURE", capture)
        .env("SOPHIA_TTY_PROFILE", "hagia")
        .env("SOPHIA_INSTALLED_ATTEMPT_MODE", "hagia")
        .env("SOPHIA_DESKTOP_PROFILE_MODE", "user")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    fs::read_to_string(capture).unwrap().trim().to_owned()
}

#[test]
fn the_session_prefers_the_users_client_and_falls_back_without_writing_it() {
    let dir = Dir::new("wm-select");
    let (state, user_wm) = user_state(&dir.0);
    let release = launcher_release(&dir.0);
    let capture = dir.0.join("capture");
    let before = snapshot(&state);

    // The reference is legitimate: the ordinary session runs the user's own
    // reloadable client when it is present and safe.
    assert_eq!(
        selected_client(&release, &state, &capture),
        user_wm.display().to_string()
    );
    assert_eq!(snapshot(&state), before);

    // Without it the packaged pair runs; nothing is copied into user state.
    fs::remove_file(&user_wm).unwrap();
    let without = snapshot(&state);
    assert_eq!(
        selected_client(&release, &state, &capture),
        release.join("target/release/hagia").display().to_string()
    );
    assert_eq!(snapshot(&state), without);
    assert!(!user_wm.exists());
}
