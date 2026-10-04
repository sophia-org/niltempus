//! The niltempus desktop's login launcher
//! (tools/installed/sophia-niltempus-desktop-session), run from a fake
//! release through a link as the login entry reaches it: it resolves the
//! release, checks Hagia's WM environment contract and hands the session the
//! release's own profile and Hagia, with stale inherited settings removed.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[path = "support/release_fixture.rs"]
mod fixture;
use fixture::{Dir, repo, script};

const LAUNCHER: &str = "tools/installed/sophia-niltempus-desktop-session";

/// The contract line the launcher requires, read from the launcher itself.
fn expected_contract() -> String {
    let launcher = fs::read_to_string(repo().join(LAUNCHER)).unwrap();
    launcher
        .lines()
        .find_map(|line| line.strip_prefix("expected='")?.strip_suffix('\''))
        .expect("the launcher names its expected contract")
        .to_owned()
}

/// A release with the real launcher, a Hagia stub that reports `contract`
/// and a session stub that records its arguments and environment; returns
/// the link the login entry would run.
fn release(root: &Path, contract: &str) -> PathBuf {
    let release = root.join("releases/niltempus-test");
    for sub in ["bin", "target/release", "share/sophia-niltempus-desktop"] {
        fs::create_dir_all(release.join(sub)).unwrap();
    }
    fs::copy(
        repo().join(LAUNCHER),
        release.join("bin/sophia-niltempus-desktop-session"),
    )
    .unwrap();
    script(
        &release.join("target/release/hagia"),
        &format!(
            "[[ \"$*\" == \"config check-environment-contract\" ]]\nprintf '%s\\n' '{contract}'"
        ),
    );
    script(
        &release.join("bin/sophia-hagia-session"),
        "printf '%s\\n' \"$@\" >\"$CAPTURE.args\"\nenv >\"$CAPTURE.env\"",
    );
    fs::write(
        release.join("share/sophia-niltempus-desktop/desktop.kdl"),
        "schema 1\n",
    )
    .unwrap();
    std::os::unix::fs::symlink("releases/niltempus-test", root.join("current")).unwrap();
    root.join("current/bin/sophia-niltempus-desktop-session")
}

fn launch(entry: &Path, capture: &Path) -> Output {
    Command::new(entry)
        .arg("--extra")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/nonexistent-home")
        .env("CAPTURE", capture)
        // Stale settings a previous session or the user's shell may leave.
        .env("SOPHIA_HAGIA_BIN", "/stale/hagia")
        .env("SOPHIA_DESKTOP_PROFILE", "/stale/profile")
        .env("SOPHIA_DESKTOP_PROFILE_MODE", "packaged-promotion")
        .env("SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE", "1")
        .output()
        .unwrap()
}

#[test]
fn the_launcher_hands_the_session_the_releases_own_profile_and_hagia() {
    let dir = Dir::new("desktop-launcher");
    let entry = release(&dir.0, &expected_contract());
    let capture = dir.0.join("capture");
    let output = launch(&entry, &capture);
    assert!(output.status.success(), "{output:?}");

    let release = fs::canonicalize(dir.0.join("releases/niltempus-test")).unwrap();
    let hagia = release.join("target/release/hagia");
    let args = fs::read_to_string(capture.with_extension("args")).unwrap();
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        [
            format!("--wm-process={}", hagia.display()).as_str(),
            "--wm-transport=9p2000.L",
            "--extra",
        ]
    );
    let env = fs::read_to_string(capture.with_extension("env")).unwrap();
    let value = |name: &str| {
        env.lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            .map(str::to_owned)
    };
    let profile = release.join("share/sophia-niltempus-desktop/desktop.kdl");
    assert_eq!(
        value("SOPHIA_DESKTOP_PROFILE"),
        Some(profile.display().to_string())
    );
    assert_eq!(value("SOPHIA_HAGIA_BIN"), Some(hagia.display().to_string()));
    assert_eq!(value("SOPHIA_WM_BIN"), Some(hagia.display().to_string()));
    assert_eq!(
        value("SOPHIA_INSTALLED_ATTEMPT_MODE").as_deref(),
        Some("hagia")
    );
    assert_eq!(
        value("SOPHIA_INSTALL_PREFIX").as_deref(),
        Some("/opt/sophia-niltempus-desktop")
    );
    assert_eq!(
        value("PATH"),
        Some(format!(
            "{}:{}:/usr/bin:/bin",
            release.join("bin").display(),
            release.join("target/release").display()
        ))
    );
    for stale in [
        "SOPHIA_DESKTOP_PROFILE_MODE",
        "SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE",
    ] {
        assert_eq!(value(stale), None, "{stale} was passed on");
    }
}

#[test]
fn the_launcher_refuses_a_hagia_with_another_contract() {
    let dir = Dir::new("desktop-launcher-contract");
    let entry = release(&dir.0, "hagia_environment_contract schema=0");
    let capture = dir.0.join("capture");
    let output = launch(&entry, &capture);
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("incompatible WM environment contract"),
        "{output:?}"
    );
    assert!(!capture.with_extension("args").exists());
}
