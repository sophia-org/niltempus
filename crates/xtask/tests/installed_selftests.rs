//! Offline gate entries for the moved installed-stack self-tests (Sophia's
//! `cargo xtask check` list at a6edbbcad, unchanged at the pin de776c68):
//! each runs its script from this repository in a clean environment, bounded,
//! with no device, VT, display manager or real product binary.
//!
//! - installed_session_type: tools/check_installed_session_type.sh (X11
//!   session identity, and the opaque label plus absolute adapter, checker and
//!   recipe tool handed to Sophia's supervisor).
//! - hagia_profile_selection: tools/check_hagia_profile_selection.sh.
//! - rehearse_wm_9p: tools/check_rehearse_wm_9p.sh.
//! - live_session_install: tools/check_live_session_install.sh. It exercises
//!   Sophia's retained stop primitive and lifecycle library, so it needs the
//!   staged pinned tree: set SOPHIA_TEST_TREE to an absolute `git archive` of
//!   the pinned revision (the gate stages it). Without it the test is not
//!   run by default and fails loudly when requested.
use std::path::Path;
use std::process::Command;

fn run(script: &str, tree: Option<&str>) -> String {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut command = Command::new("timeout");
    command
        .args(["-s", "KILL", "300", "bash"])
        .arg(repo.join(script))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", std::env::temp_dir());
    if let Some(tree) = tree {
        command.env("SOPHIA_TEST_TREE", tree);
    }
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{script}\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

#[test]
fn installed_session_type() {
    assert!(
        run("tools/check_installed_session_type.sh", None)
            .contains("sophia_installed_session_type schema=1 status=x11 wayland_display=unset")
    );
}

#[test]
fn hagia_profile_selection() {
    assert!(
        run("tools/check_hagia_profile_selection.sh", None)
            .contains("profile selection checks passed")
    );
}

#[test]
fn rehearse_wm_9p() {
    assert!(run("tools/check_rehearse_wm_9p.sh", None).contains("offline checks passed"));
}

#[test]
#[ignore = "needs SOPHIA_TEST_TREE: the staged pinned Sophia tree (the gate stages it)"]
fn live_session_install() {
    let tree = std::env::var("SOPHIA_TEST_TREE")
        .expect("SOPHIA_TEST_TREE must name the staged pinned Sophia tree");
    assert!(tree.starts_with('/'), "SOPHIA_TEST_TREE must be absolute");
    assert!(
        run("tools/check_live_session_install.sh", Some(&tree))
            .contains("install, activation, and rollback checks passed")
    );
}
