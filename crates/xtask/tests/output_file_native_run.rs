//! Run planning and restoration refusal checks. No Session or device is opened.
use std::collections::BTreeMap;
use xtask::output_file_native::Paths;
use xtask::output_file_native_run::{arguments, check_foreground_tty, check_recovery};

#[test]
fn attached_console_must_also_be_the_foreground_console() {
    check_foreground_tty("/dev/tty4", "tty4\n").unwrap();
    assert!(
        check_foreground_tty("/dev/tty4", "tty2\n")
            .unwrap_err()
            .contains("Return to /dev/tty4")
    );
    for active in [
        "",
        "tty0",
        "tty64",
        "tty04",
        "tty+4",
        "tty4 tty2",
        "tty4\ntty2",
        " tty4",
        "tty4 ",
    ] {
        assert!(
            check_foreground_tty("/dev/tty4", active).is_err(),
            "{active:?}"
        );
    }
}

fn inputs() -> (BTreeMap<String, String>, Paths) {
    let values = [
        ("inputs", "/sealed"),
        ("inputs-manifest-sha256", "unused"),
        ("preparation", "/prepared"),
        ("preparation-sha256", "unused"),
        ("profile", "integration/output.kdl"),
        ("out", "/private/plan"),
        ("tty", "/dev/tty4"),
        ("display", ":91"),
        ("input-seat", "seat0"),
        ("runtime-ms", "60000"),
        ("a-topology-epoch", "7"),
        ("a-heads", "1:1:normal:disabled"),
        ("a-groups", "1@0,0,800x600=1/exact"),
        ("a-primary", "0"),
        ("b-heads", "1:2:normal:disabled"),
        ("b-groups", "1@0,0,800x600=1/exact"),
        ("b-primary", "0"),
        ("preflight", "/prepared/preflight"),
        ("preflight-sha256", "unused"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect();
    (
        values,
        Paths {
            sophia: "/sealed/bin/sophia".into(),
            hagia: "/sealed/bin/hagia".into(),
            profile: "/sealed/profiles/integration/output.kdl".into(),
            peer: "/prepared/peer".into(),
        },
    )
}

#[test]
fn exact_arguments_keep_roles_and_death_control_separate() {
    let (values, paths) = inputs();
    let stages = arguments(&values, &paths).unwrap();
    assert_eq!(stages.len(), 4);
    for (index, argv) in stages.iter().enumerate() {
        assert_eq!(&argv[..3], ["/sealed/bin/sophia", "session", "run"]);
        assert_eq!(
            argv.iter()
                .filter(|a| *a == "--output-proof-peer-loss-after-apply")
                .count(),
            usize::from(index == 3)
        );
        assert!(argv.contains(&"--input-seat=seat0".into()));
        assert!(argv.contains(&"--display=:91".into()));
        assert!(argv.contains(&"--output-process=/prepared/peer".into()));
        assert_eq!(
            argv.iter()
                .any(|a| a.starts_with("--output-process-arg=--b-heads=")),
            index != 1
        );
    }
}

#[test]
fn unsafe_or_ambiguous_run_context_is_refused_without_starting() {
    let (values, paths) = inputs();
    for (key, value) in [
        ("tty", "/dev/pts/1"),
        ("tty", "/dev/tty0"),
        ("tty", "/dev/tty64"),
        ("display", ":77"),
        ("display", ":89"),
        ("display", ":100"),
        ("display", ":-1"),
        ("display", ":65536"),
        ("input-seat", "seat0;cmd"),
        ("runtime-ms", "59999"),
        ("runtime-ms", "600001"),
        ("runtime-ms", "+60000"),
        ("a-topology-epoch", "0"),
        ("a-primary", "16"),
    ] {
        let mut changed = values.clone();
        changed.insert(key.into(), value.into());
        assert!(arguments(&changed, &paths).is_err(), "{key}={value}");
    }
    let mut missing = values;
    missing.remove("tty");
    assert!(arguments(&missing, &paths).is_err());
}

const RECOVERY: &str = "sophia_tty_recovery schema=3 profile=output-file-native kd_mode_before=0 kd_mode_after=0 termios_restored=true emergency=false session_shutdown=not_requested session_exit_status=none\nsophia_tty_recovery_verification schema=1 profile=output-file-native keyboard_mode_before=1 keyboard_mode_after=1 keyd_seen=false keyd_restored=true\n";

#[test]
fn recovery_requires_positive_matching_observations() {
    check_recovery(RECOVERY).unwrap();
    for (from, to) in [
        ("kd_mode_after=0", "kd_mode_after=1"),
        ("termios_restored=true", "termios_restored=false"),
        ("emergency=false", "emergency=true"),
        ("keyboard_mode_after=1", "keyboard_mode_after=2"),
        ("keyd_restored=true", "keyd_restored=false"),
        ("schema=3", "schema=2"),
        (
            "kd_mode_before=0 kd_mode_after=0",
            "kd_mode_before=unavailable kd_mode_after=unavailable",
        ),
        ("emergency=false", "emergency=false emergency=false"),
    ] {
        assert!(
            check_recovery(&RECOVERY.replace(from, to)).is_err(),
            "{from}"
        );
    }
    assert!(check_recovery(RECOVERY.lines().next().unwrap()).is_err());
    assert!(check_recovery(&format!("{RECOVERY}{RECOVERY}")).is_err());
}
