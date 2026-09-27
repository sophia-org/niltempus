//! SOPHIA_DESKTOP_PROFILE_MODE, with no legacy alias.
//!
//! FORWARDING EVIDENCE ONLY: the installed launchers hand the resolved mode
//! and the profile digest to a stubbed Sophia unchanged. That the real
//! session reads them and emits `sophia_live_desktop_profile` is Sophia's
//! reader behaviour, recorded separately by a real (installed) run; a stub
//! cannot prove it and this test does not claim to.
//!
//! The retired names appear nowhere in this repository (tree-wide).
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "support/release_fixture.rs"]
mod fixture;
use fixture::{Dir, repo, script, sha256};

/// A release with the real installed launchers; the Sophia stub records what
/// it was handed (the ordinary supervisor) and the adapter stub does the same
/// (the proof path).
fn release(root: &Path) -> PathBuf {
    let release = root.join("release");
    for sub in [
        "bin",
        "tools/lib",
        "tools/session",
        "target/release",
        "share/sophia-policy/hagia",
    ] {
        fs::create_dir_all(release.join(sub)).unwrap();
    }
    for name in [
        "sophia-session",
        "sophia-hagia-session",
        "sophia-hagia-promotion-session",
    ] {
        fs::copy(
            repo().join("tools/installed").join(name),
            release.join("bin").join(name),
        )
        .unwrap();
    }
    // The system profile is an explicit fixture path, never the host's /etc:
    // the copy under test reads it from `root` (production keeps its default).
    let launcher = release.join("bin/sophia-hagia-session");
    let text = fs::read_to_string(&launcher).unwrap();
    assert_eq!(text.matches("/etc/sophia/desktop.kdl").count(), 2);
    let system = root.join("etc/sophia/desktop.kdl");
    fs::write(
        &launcher,
        text.replace("/etc/sophia/desktop.kdl", system.to_str().unwrap()),
    )
    .unwrap();
    fs::write(
        release.join("tools/lib/session_lifecycle.sh"),
        "sophia_session_rotate_log() { :; }\n",
    )
    .unwrap();
    fs::write(
        release.join("manifest"),
        format!("version=0.1.0\ncommit={}\n", "0".repeat(40)),
    )
    .unwrap();
    fs::write(
        release.join("share/sophia-policy/hagia/default.kdl"),
        "schema 1\n",
    )
    .unwrap();
    for name in ["hagia", "narthex", "sophia-wm-demo"] {
        script(&release.join("target/release").join(name), "# packaged");
    }
    let record = "printf 'mode=%s sha=%s via=%s\\n' \"${SOPHIA_DESKTOP_PROFILE_MODE-<unset>}\" \
                  \"${SOPHIA_DESKTOP_PROFILE_SHA256-<unset>}\" \"$VIA\" >\"$CAPTURE\"";
    script(
        &release.join("target/release/sophia"),
        &format!("VIA=supervisor\n{record}"),
    );
    script(
        &release.join("tools/session/run_desktop_session.sh"),
        &format!("VIA=adapter\n{record}"),
    );
    // Proof launches reserve and finish a ledger slot around the adapter.
    script(
        &release.join("bin/sophia-record-hagia-run"),
        "if [[ \"$1\" == begin ]]; then mktemp -d \"$CAPTURE.attempt.XXXXXX\"; fi",
    );
    script(&release.join("bin/capture-runtime-identity"), "exit 0");
    release
}

fn launch(root: &Path, command: &str, env: &[(&str, &str)]) -> (bool, String, String) {
    let capture = root.join("capture");
    let _ = fs::remove_file(&capture);
    let config = root.join("config");
    let output = Command::new(root.join("release/bin").join(command))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_STATE_HOME", root.join("state"))
        .env("CAPTURE", &capture)
        .envs(env.iter().copied())
        .output()
        .unwrap();
    (
        output.status.success(),
        fs::read_to_string(capture).unwrap_or_default(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn the_resolved_mode_and_digest_reach_the_session_unchanged() {
    let dir = Dir::new("profile-mode");
    let root = &dir.0;
    let release = release(root);
    let packaged = release.join("share/sophia-policy/hagia/default.kdl");
    let packaged_sha = sha256(&fs::read(packaged).unwrap());

    // No user, system or explicit profile: the packaged default, through
    // the supervisor.
    let (ok, seen, stderr) = launch(root, "sophia-hagia-session", &[]);
    assert!(ok, "{stderr}");
    assert_eq!(
        seen,
        format!("mode=packaged-fallback sha={packaged_sha} via=supervisor\n")
    );

    // A system profile (the fixture's, never the host's) wins over it.
    let system = root.join("etc/sophia/desktop.kdl");
    fs::create_dir_all(system.parent().unwrap()).unwrap();
    fs::write(&system, "schema 1\n// system\n").unwrap();
    let system_sha = sha256(&fs::read(system).unwrap());
    let (ok, seen, stderr) = launch(root, "sophia-hagia-session", &[]);
    assert!(ok, "{stderr}");
    assert_eq!(
        seen,
        format!("mode=system sha={system_sha} via=supervisor\n")
    );

    // The user's desktop.kdl.
    let user = root.join("config/sophia/desktop.kdl");
    fs::create_dir_all(user.parent().unwrap()).unwrap();
    fs::write(&user, "schema 1\n// user\n").unwrap();
    let user_sha = sha256(&fs::read(user).unwrap());
    let (ok, seen, stderr) = launch(root, "sophia-hagia-session", &[]);
    assert!(ok, "{stderr}");
    assert_eq!(seen, format!("mode=user sha={user_sha} via=supervisor\n"));

    // An explicit profile.
    let explicit = root.join("explicit.kdl");
    fs::write(&explicit, "schema 1\n// explicit\n").unwrap();
    let explicit_sha = sha256(&fs::read(&explicit).unwrap());
    let (ok, seen, stderr) = launch(
        root,
        "sophia-hagia-session",
        &[("SOPHIA_DESKTOP_PROFILE", explicit.to_str().unwrap())],
    );
    assert!(ok, "{stderr}");
    assert_eq!(
        seen,
        format!("mode=explicit sha={explicit_sha} via=supervisor\n")
    );

    // Promotion ignores both and takes the proof path with the packaged
    // default, even when an explicit profile is exported.
    let (ok, seen, stderr) = launch(
        root,
        "sophia-hagia-promotion-session",
        &[("SOPHIA_DESKTOP_PROFILE", explicit.to_str().unwrap())],
    );
    assert!(ok, "{stderr}");
    assert_eq!(
        seen,
        format!("mode=packaged-promotion sha={packaged_sha} via=adapter\n")
    );
}

#[test]
fn only_auto_or_packaged_promotion_can_be_requested() {
    let dir = Dir::new("profile-mode-invalid");
    release(&dir.0);
    for requested in ["user", "system", "explicit", "packaged-fallback", "legacy"] {
        let (ok, seen, stderr) = launch(
            &dir.0,
            "sophia-hagia-session",
            &[("SOPHIA_DESKTOP_PROFILE_MODE", requested)],
        );
        assert!(!ok, "{requested}");
        assert!(seen.is_empty(), "{requested}");
        assert!(
            stderr.contains("SOPHIA_DESKTOP_PROFILE_MODE must be auto or packaged-promotion"),
            "{requested}: {stderr}"
        );
    }
}

/// The retired names, assembled so that this file does not contain them.
fn retired() -> Vec<String> {
    vec![
        ["SOPHIA_HAGIA", "_PROFILE_MODE"].concat(),
        ["HAGIA", "_POLICY_"].concat(),
        ["hagia-policy", ".checkpoint"].concat(),
        ["sophia_check", "_hagia_profile"].concat(),
        ["hagia/", "config.kdl"].concat(),
    ]
}

/// A frozen, byte-for-byte oracle may quote history; nothing else may.
const FROZEN: [&str; 1] = ["crates/xtask/tests/fixtures/session_arguments_before_t027.sh"];

fn walk(root: &Path, dir: &Path, hits: &mut Vec<String>, names: &[String], scanned: &mut usize) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let relative = path.strip_prefix(root).unwrap().display().to_string();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let meta = fs::symlink_metadata(&path).unwrap();
        if meta.is_dir() {
            if [".git", "target", ".provision", ".artifacts"].contains(&name.as_str()) {
                continue;
            }
            walk(root, &path, hits, names, scanned);
        } else if meta.is_file() && !FROZEN.contains(&relative.as_str()) {
            *scanned += 1;
            let bytes = fs::read(path).unwrap();
            for retired in names {
                if bytes
                    .windows(retired.len())
                    .any(|window| window == retired.as_bytes())
                {
                    hits.push(format!("{relative}: {retired}"));
                }
            }
        }
    }
}

#[test]
fn no_retired_profile_name_remains_in_the_repository() {
    let root = repo();
    let names = retired();
    let mut hits = Vec::new();
    let mut scanned = 0;
    walk(&root, &root, &mut hits, &names, &mut scanned);
    assert!(scanned > 100, "scanned only {scanned} files");
    assert!(hits.is_empty(), "{hits:#?}");
    // The frozen oracle is the only exemption and still exists.
    for path in FROZEN {
        assert!(root.join(path).is_file(), "{path}");
    }
}
