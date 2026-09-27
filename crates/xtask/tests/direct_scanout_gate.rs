// Provenance: the Probe and terminal cases moved from Sophia
// crates/sophia-conformance/tests/direct_scanout.rs at
// de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13), with the
// gate they test (crates/xtask/src/direct_scanout_gate.rs); the explicit-input
// refusals are new.
//! The direct-scanout physical gate: typed arguments, the terminal rule, and
//! refusal of every missing explicit input before anything is built.
use std::path::Path;
use std::process::Command;
use xtask::direct_scanout_gate;

#[test]
fn probe_arguments_are_typed_before_the_display_is_taken() {
    let error = direct_scanout_gate::Probe::from_arguments(&[
        "zero".to_owned(),
        "1440".to_owned(),
        "20".to_owned(),
        "kitty".to_owned(),
    ])
    .unwrap_err();
    assert!(error.contains("width must be a positive integer"));

    let error = direct_scanout_gate::Probe::from_arguments(&[
        "2560".to_owned(),
        "1440".to_owned(),
        "20".to_owned(),
        "unknown".to_owned(),
    ])
    .unwrap_err();
    assert!(error.contains("workload must be"));
}

/// The gate and the probe share one argument vocabulary, so the flag has to
/// survive alongside the positional arguments rather than displacing one.
#[test]
fn the_overlay_flag_parses_beside_the_positional_arguments() {
    let plain = direct_scanout_gate::Probe::from_arguments(&[]).unwrap();
    assert!(!plain.overlay_proof, "overlay proof is off by default");

    let flagged =
        direct_scanout_gate::Probe::from_arguments(&["--overlay-proof".to_owned()]).unwrap();
    assert!(flagged.overlay_proof);
    assert_eq!(flagged.width, plain.width, "the flag is not a width");
    assert_eq!(flagged.height, plain.height);

    let mixed = direct_scanout_gate::Probe::from_arguments(&[
        "1920".to_owned(),
        "--overlay-proof".to_owned(),
        "1080".to_owned(),
    ])
    .unwrap();
    assert!(mixed.overlay_proof);
    assert_eq!(mixed.width, 1920);
    assert_eq!(mixed.height, 1080, "the flag must not consume a position");
}

#[test]
fn the_gate_terminal_is_tty3() {
    assert!(direct_scanout_gate::is_gate_terminal(Path::new(
        "/dev/tty3"
    )));
    assert!(!direct_scanout_gate::is_gate_terminal(Path::new(
        "/dev/tty1"
    )));
}

/// The gate's `--cost` implies the overlay proof: without the window there is
/// no composed population to measure.
#[test]
fn asking_for_cost_asks_for_the_overlay_that_produces_it() {
    let probe = direct_scanout_gate::Probe::from_arguments(&["--cost".to_owned()]).unwrap();
    assert!(probe.cost);
    assert!(
        probe.overlay_proof,
        "a cost run must open the overlay it measures the composed side of"
    );
    assert!(
        probe.hold_seconds >= 35,
        "and must hold it long enough to have a distribution"
    );
}

/// Every source is explicit and absolute, and a missing one refuses before the
/// terminal is checked or anything is built.
#[test]
fn missing_explicit_inputs_refuse_before_any_effect() {
    let executable = std::env::current_exe().unwrap();
    let executable = executable.to_str().unwrap();
    let full = [
        ("SOPHIA_SOURCE", "/nonexistent/sophia"),
        ("SOPHIA_SESSION_PREFLIGHT", executable),
        ("SOPHIA_INTEGRATION_XTASK", executable),
    ];
    for (name, _) in full {
        for value in [None, Some("relative/path")] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
            command
                .arg("direct-scanout-gate")
                .env_clear()
                .env("PATH", "/usr/bin:/bin");
            for (other, default) in full {
                if other != name {
                    command.env(other, default);
                }
            }
            if let Some(value) = value {
                command.env(name, value);
            }
            let output = command.output().unwrap();
            assert!(!output.status.success(), "{name} {value:?}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains(&format!("{name} must be an absolute path")),
                "{name} {value:?}: {stderr}"
            );
        }
    }
}
