//! The installed session never writes the user's default window manager,
//! $XDG_STATE_HOME/sophia/bin/hagia (the user-owned policy client that the
//! reload workflow replaces). It may legitimately reference that path: it
//! prefers the user's client when one is present and falls back to the
//! packaged Hagia otherwise, reading it and never writing it.
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

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
