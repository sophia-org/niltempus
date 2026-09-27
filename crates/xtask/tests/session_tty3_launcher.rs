// Provenance: moved from Sophia crates/sophia-cli/tests/session_launcher_recovery.rs
// (tty_adapter_refuses_controls_before_queries_or_privileged_handoff) and
// crates/sophia-cli/tests/launcher_safety.rs (the TTY3_LAUNCHER parts of
// tty3_gate_restores_input_before_activating_the_ready_greetd_vt and
// tty_profile_check_is_independent_of_the_callers_working_directory) at
// a6edbbcad02ad9e9bb4c790e7cfd714cf333d30b (original copy; source unchanged at
// the pin de776c68) (Sophia rule 13). The launcher is now tools/session/start_sophia_tty3.sh.
// Sophia's own refusal of invalid controls stays in Sophia with prepare-controls;
// here a stub `sophia` refuses, and the test proves the launcher asks it before
// any TTY query or privileged handoff. The desktop-comparison assertions move
// with that gate (slice S4); the prepare-arguments assertions now apply to
// tools/session/run_desktop_session.sh.
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

const TTY3_LAUNCHER: &str = include_str!("../../../tools/session/start_sophia_tty3.sh");
const DESKTOP_ADAPTER: &str = include_str!("../../../tools/session/run_desktop_session.sh");

fn on_pty(command: &mut Command) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Keep the pseudo-terminal's upstream input open until the adapter exits;
    // an immediate EOF can make script hang up its child before exec.
    let _input = child.stdin.take().unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn tty_adapter_refuses_controls_before_queries_or_privileged_handoff() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "sophia-handoff-refusal-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("tools/session")).unwrap();
    fs::create_dir_all(root.join("sophia/tools/lib")).unwrap();
    fs::create_dir_all(root.join("bin")).unwrap();
    let launcher = fs::read_to_string(source.join("tools/session/start_sophia_tty3.sh"))
        .unwrap()
        .replace(
            "LAUNCH_LOG=\"/tmp/sophia-${SESSION_PROFILE}-tty${TARGET_VT}-launch.log\"",
            &format!("LAUNCH_LOG=\"{}/launch.log\"", root.display()),
        );
    fs::write(root.join("tools/session/start_sophia_tty3.sh"), launcher).unwrap();
    // Test double of Sophia's retained session_preparation.sh contract: run
    // `$SOPHIA_BIN session <command>`, refuse on failure.
    fs::write(
        root.join("sophia/tools/lib/session_preparation.sh"),
        "sophia_load_preparation() { local expected=\"$1\" command=\"$2\"; shift 2; \
         \"$SOPHIA_BIN\" session \"$command\" \"$@\" >/dev/null || { echo \"The installed binary refused $command.\" >&2; return 1; }; }\n",
    )
    .unwrap();
    for (name, body) in [
        ("tty", "echo /dev/tty3"),
        (
            "sophia",
            "printf '%s\\n' \"$*\" > \"$HOME/preparation\"; echo 'refused: invalid guard arming' >&2; exit 1",
        ),
        ("preflight", "exit 0"),
        ("python3", "echo forbidden >> \"$HOME/takeover\"; exit 99"),
        ("sudo", "echo forbidden >> \"$HOME/takeover\"; exit 99"),
    ] {
        let path = root.join("bin").join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let output = on_pty(
        Command::new("/usr/bin/timeout")
            .args(["--kill-after=2s", "20s", "script", "-qefc"])
            .arg(format!(
                "exec bash '{}'",
                root.join("tools/session/start_sophia_tty3.sh").display()
            ))
            .arg("/dev/null")
            .env_clear()
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", root.join("bin").display()),
            )
            .env("HOME", &root)
            .env("SOPHIA_TTY_PROFILE", "native")
            .env("SOPHIA_ROOT", root.join("sophia"))
            .env("SOPHIA_BIN", root.join("bin/sophia"))
            .env("SOPHIA_SESSION_PREFLIGHT", root.join("bin/preflight"))
            .env("SOPHIA_INPUT_GUARD_ARMING", "invalid"),
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    // A fast refusal can exit before the launcher's process-substitution tee
    // drains. Observe the synchronous call, not that best-effort terminal log.
    assert_eq!(
        fs::read_to_string(root.join("preparation")).unwrap(),
        "session prepare-controls --profile=native\n"
    );
    assert!(!root.join("takeover").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tty3_gate_restores_input_before_activating_the_ready_greetd_vt() {
    let restore_origin = TTY3_LAUNCHER.find("if ! restore_origin_tty").unwrap();
    let restore_manager_tty = TTY3_LAUNCHER.find("if ! restore_greetd_tty").unwrap();
    let restore_manager = TTY3_LAUNCHER
        .find("sudo -n sv up \"$display_manager\"")
        .unwrap();
    let greeter_ready = TTY3_LAUNCHER.find("ps -C tuigreet -o tty=").unwrap();
    let verify_manager_tty = TTY3_LAUNCHER
        .find("elif ! verify_greetd_tty_ready")
        .unwrap();
    let reactivate_tty = TTY3_LAUNCHER
        .find("sudo -n chvt \"$activation_vt\"")
        .unwrap();

    assert!(TTY3_LAUNCHER.contains("origin_tty=\"$(tty)\""));
    assert!(TTY3_LAUNCHER.contains("origin_vt=\"${origin_tty#/dev/tty}\""));
    assert!(
        TTY3_LAUNCHER
            .contains("origin_keyboard_mode=\"$(python3 \"$TTY_MODE_HELPER\" get-keyboard)\"")
    );
    assert!(TTY3_LAUNCHER.contains("display_manager_keyboard_mode=\"$("));
    assert!(TTY3_LAUNCHER.contains("verify_greetd_tty_prestart"));
    assert!(TTY3_LAUNCHER.contains("establish_safe_greetd_tty"));
    assert!(TTY3_LAUNCHER.contains("sudo -n stty sane -F \"$display_manager_tty\""));
    assert!(TTY3_LAUNCHER.contains("phase=exact_prestart"));
    assert!(TTY3_LAUNCHER.contains("phase=safe_prestart"));
    assert!(TTY3_LAUNCHER.contains("phase=live_ready"));
    assert!(TTY3_LAUNCHER.contains("keyboard_mode\" =~ ^[0-3]$"));
    assert!(TTY3_LAUNCHER.contains("stable_samples\" -ge 3"));
    assert!(TTY3_LAUNCHER.contains("manager_restore=%s"));
    assert!(TTY3_LAUNCHER.contains("manager_keyboard=%s"));
    assert!(restore_origin < restore_manager_tty);
    assert!(restore_manager_tty < restore_manager);
    assert!(restore_manager < greeter_ready);
    assert!(greeter_ready < verify_manager_tty);
    assert!(verify_manager_tty < reactivate_tty);
    // Formerly asserted of Sophia's session launcher; the named argument
    // assembly now lives in the external adapter.
    assert!(TTY3_LAUNCHER.contains("prepare-controls"));
    assert!(DESKTOP_ADAPTER.contains("session-recipe prepare-arguments"));
    assert!(DESKTOP_ADAPTER.contains("sophia_session_arguments schema=1 status=prepared"));
}

#[test]
fn tty_profile_check_is_independent_of_the_callers_working_directory() {
    // The profile check needs Sophia's prebuilt xtask from the pinned tree:
    // no Cargo invocation, no alias lookup in the caller's directory.
    assert!(TTY3_LAUNCHER.contains("SOPHIA_PROFILE_CHECK_XTASK"));
    assert!(TTY3_LAUNCHER.contains("\"$checker\" profile check"));
    assert!(!TTY3_LAUNCHER.contains("cargo xtask profile check"));
    assert!(!TTY3_LAUNCHER.contains("cargo run"));
    assert!(!TTY3_LAUNCHER.contains("cargo build"));
}
