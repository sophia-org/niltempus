use std::collections::BTreeMap;
use xtask::shell_timing::ImageSource;
use xtask::shell_timing_campaign_provenance::ProvenanceTarget;
use xtask::shell_timing_native::{Artifact, Request, Source};
use xtask::shell_timing_panel_diagnostic::{Descriptor, policy, run};

fn fixture() -> (Descriptor, Request) {
    let artifact = || Artifact {
        path: "/not-executed".into(),
        sha256: "a".repeat(64),
    };
    let source = || Source {
        path: "/not-executed".into(),
        commit: "a".repeat(40),
    };
    (
        Descriptor {
            schema: 1,
            plan: artifact(),
            python: artifact(),
            sampler: artifact(),
            warmup: 120,
            counted: 2000,
            interval_ms: 5,
            target: ProvenanceTarget {
                card: "card0".into(),
                output: 1,
                scale_milli: 1000,
                connector: "DP-1".into(),
                width: 1920,
                height: 1080,
                mode_millihz: 60_000,
                vrr: false,
            },
        },
        Request {
            sophia_source: source(),
            integration_source: source(),
            artifacts: BTreeMap::new(),
            tty: "/dev/tty4".into(),
            display: 94,
            input_seat: "seat0".into(),
            runtime_ms: 60_000,
            wire: "files".into(),
            source: ImageSource::OwnedRing,
            output: 1,
            frames: 2120,
            mode_millihz: 60_000,
            instrumentation: "none".into(),
            environment_readback: true,
            external_timing: None,
        },
    )
}

#[test]
fn only_matched_uninstrumented_panel_sources_enter_the_diagnostic() {
    let (d, mut r) = fixture();
    policy(&d, &r, 0).unwrap();
    r.source = ImageSource::Isb;
    policy(&d, &r, 0).unwrap();
    r.source = ImageSource::Bytes;
    assert!(policy(&d, &r, 0).is_err());
    r.source = ImageSource::OwnedRing;
    assert!(policy(&d, &r, 4_194_304).is_err());
    r.instrumentation = "cpu-only".into();
    assert!(policy(&d, &r, 0).is_err());
    r.instrumentation = "none".into();
    r.environment_readback = false;
    assert!(policy(&d, &r, 0).is_err());
}

#[test]
fn wrong_frame_denominator_or_target_is_refused() {
    let (mut d, mut r) = fixture();
    r.frames = 2000;
    assert!(policy(&d, &r, 0).is_err());
    r.frames = 2120;
    d.warmup = 0;
    assert!(policy(&d, &r, 0).is_err());
    d.warmup = 120;
    d.target.output = 2;
    assert!(policy(&d, &r, 0).is_err());
    d.target.output = 1;
    d.target.mode_millihz = 120_000;
    assert!(policy(&d, &r, 0).is_err());
}

#[test]
fn changed_descriptor_never_creates_a_stage_or_reaches_launch() {
    let root = std::env::temp_dir().join(format!("panel-diagnostic-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let descriptor = root.join("descriptor.json");
    let stage = root.join("run");
    std::fs::write(&descriptor, b"not a descriptor; must fail digest first").unwrap();
    let error = run(&[
        "capture-panel-diagnostic".into(),
        descriptor.display().to_string(),
        "0".repeat(64),
        stage.display().to_string(),
    ])
    .unwrap_err();
    assert_eq!(error, "diagnostic descriptor changed");
    assert!(!stage.exists());
    std::fs::remove_dir_all(root).unwrap();
}
