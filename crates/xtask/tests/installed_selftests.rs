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
use std::path::Path;
use std::process::Command;

fn run(script: &str) -> String {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut command = Command::new("timeout");
    command
        .args(["-s", "KILL", "300", "bash"])
        .arg(repo.join(script))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", std::env::temp_dir());
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
        run("tools/check_installed_session_type.sh")
            .contains("sophia_installed_session_type schema=1 status=x11 wayland_display=unset")
    );
}

#[test]
fn hagia_profile_selection() {
    assert!(
        run("tools/check_hagia_profile_selection.sh").contains("profile selection checks passed")
    );
}

#[test]
fn rehearse_wm_9p() {
    assert!(run("tools/check_rehearse_wm_9p.sh").contains("offline checks passed"));
}
