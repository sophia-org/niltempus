//! Offline gate entries for the moved physical-family and installed-stack
//! self-tests (Sophia rule 13; Sophia's `cargo xtask check` list at the pin
//! de776c68): one cargo test per script, each run from this repository in a
//! clean environment, bounded, with no device, VT, display manager or real
//! product binary. The identity self-tests (the G1 design) and the
//! mixed-output archive test need Sophia's own signed pinned commit and its
//! retained files: set SOPHIA_TEST_SOURCE (an absolute Sophia repository
//! holding the pin; each test makes a temporary shared no-checkout clone) and
//! SOPHIA_TEST_TREE (the staged pinned tree). Without them those tests are not
//! run by default and fail loudly when requested; the gate supplies both.
use std::path::Path;
use std::process::Command;

fn run(script: &str, pinned: bool) {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut command = Command::new("timeout");
    command
        .args(["-s", "KILL", "600", "bash"])
        .arg(repo.join("tools").join(script))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        // Real signature checks read the operator's public keyring.
        .env("HOME", std::env::var_os("HOME").unwrap_or_default())
        .env("PYTHONDONTWRITEBYTECODE", "1");
    if pinned {
        for name in ["SOPHIA_TEST_SOURCE", "SOPHIA_TEST_TREE"] {
            let value =
                std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set for {script}"));
            assert!(value.starts_with('/'), "{name} must be absolute");
            command.env(name, value);
        }
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{script}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

macro_rules! self_test {
    ($name:ident) => {
        #[test]
        fn $name() {
            run(concat!(stringify!($name), ".sh"), false);
        }
    };
}

macro_rules! pinned_self_test {
    ($name:ident) => {
        #[test]
        #[ignore = "needs SOPHIA_TEST_SOURCE and SOPHIA_TEST_TREE (the gate supplies them)"]
        fn $name() {
            run(concat!(stringify!($name), ".sh"), true);
        }
    };
}

self_test!(check_firefox_m10_dialog_page);
self_test!(check_firefox_m10_kitty_probe);
self_test!(check_firefox_m10_primary_kitty_probe);
self_test!(check_firefox_m10_primary_page);
self_test!(check_firefox_m10_promotion_page);
self_test!(check_firefox_m10_rendering_page);
self_test!(check_firefox_m10_selection_kitty_probe);
self_test!(check_firefox_m10_selection_page);
self_test!(check_installed_fallback_verifier);
self_test!(check_installed_hagia_ledger);
self_test!(check_installed_login_cycle_verifier);
self_test!(check_installed_native_chrome_verifier);
self_test!(check_installed_native_verifiers);
self_test!(check_installed_session_lifecycle_verifier);
self_test!(check_installed_watchdog_recovery);
self_test!(check_installed_xterm_verifier);
self_test!(check_keyboard_independence_session_verifier);
self_test!(check_keyboard_independence_verifier);
self_test!(check_sophia_firefox_dialog_verifier);
self_test!(check_sophia_firefox_lifecycle_verifier);
self_test!(check_sophia_firefox_physical_verifier);
self_test!(check_sophia_firefox_primary_verifier);
self_test!(check_sophia_firefox_rendering_verifier);
self_test!(check_sophia_firefox_selection_verifier);
self_test!(check_sophia_glxgears_performance_reporter);
self_test!(check_sophia_native_chrome_verifier);
self_test!(check_sophia_rendering_performance_reporter);
self_test!(check_sophia_standalone_vkcube_verifier);
self_test!(check_session_terminal_arguments);
self_test!(check_sophia_terminal_performance_reporter);
self_test!(check_truecolor_verifier);
self_test!(check_xserver_rendering_performance_reporter);

pinned_self_test!(check_frame_fed_output_verifier);
pinned_self_test!(check_hagia_native_matchers);
pinned_self_test!(check_hagia_physical_matchers);
pinned_self_test!(check_live_session_milestone5_verifier);
pinned_self_test!(check_mirror_group_physical_verifier);
pinned_self_test!(check_retired_milestone_launchers);
pinned_self_test!(test_verify_mixed_output_evidence);

/// The production preflights and the native dry run of the Hagia gates
/// (tools/tests/physical_gate_identity_test.py).
#[test]
fn physical_gate_identity() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("timeout")
        .args(["-s", "KILL", "600", "python3", "-B", "-m", "unittest"])
        .arg("tools.tests.physical_gate_identity_test")
        .current_dir(&repo)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Sophia's `cargo xtask check` ran `tools/run_sophia_terminal_gate_tty3.sh
/// --self-test` (the single-attempt visual-verdict contract); the script
/// moved, so its self-test runs here. Pure shell: no device, VT or binary.
#[test]
fn run_sophia_terminal_gate_tty3_self_test() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("timeout")
        .args(["-s", "KILL", "60", "bash"])
        .arg(repo.join("tools/run_sophia_terminal_gate_tty3.sh"))
        .arg("--self-test")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        // The script resolves its state directory from HOME before the
        // self-test branch (nothing is written there).
        .env("HOME", std::env::var_os("HOME").unwrap_or_default())
        .output()
        .unwrap();
    assert!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout)
                .contains("terminal gate single-attempt contract passed"),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
