use std::collections::BTreeMap;
use xtask::shell_timing::{DrawWorkload, ImageSource};
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
            raster: None,
            workload: None,
            bar_height: None,
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
            reference_workload: None,
            producer: Default::default(),
            managed_destination_reuse: false,
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
fn longer_reference_window_requires_matching_frames_and_runtime() {
    let (mut d, mut r) = fixture();
    d.counted = 6000;
    r.frames = 6120;
    r.runtime_ms = 120_000;
    assert!(policy(&d, &r, 0).is_err()); // Frozen schema 1 remains 2000/60 s.
    d.schema = 2;
    d.workload = Some(DrawWorkload::Retained);
    r.reference_workload = d.workload;
    d.raster = Some(Artifact {
        path: "/not-executed/sophia-draw-raster".into(),
        sha256: "a".repeat(64),
    });
    for source in [
        ImageSource::OwnedRing,
        ImageSource::Draw,
        ImageSource::Mixed,
    ] {
        r.source = source;
        policy(&d, &r, 0).unwrap();
    }
    r.runtime_ms = 60_000;
    assert!(policy(&d, &r, 0).is_err());
    r.runtime_ms = 120_000;
    r.frames = 2120;
    assert!(policy(&d, &r, 0).is_err());
    r.frames = 6120;
    d.counted = 4000;
    r.frames = 4120;
    assert!(policy(&d, &r, 0).is_err());
    d.counted = usize::MAX;
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
    assert_eq!(
        run(&[
            "check-panel-diagnostic".into(),
            descriptor.display().to_string(),
            "0".repeat(64),
        ])
        .unwrap_err(),
        "diagnostic descriptor changed"
    );
    assert!(!stage.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn capture_profiles_are_only_gpu_small_panel_diagnostics() {
    let (mut d, mut r) = fixture();
    d.schema = 3;
    d.bar_height = Some(24);
    r.producer = xtask::shell_timing_native::ProducerKind::Gpu;
    r.instrumentation = "capture-profile".into();
    for source in [ImageSource::OwnedRing, ImageSource::DmaBuf] {
        r.source = source;
        assert!(policy(&d, &r, 0).is_ok());
    }
    r.source = ImageSource::Isb;
    assert!(policy(&d, &r, 0).is_err());
    r.source = ImageSource::DmaBuf;
    d.bar_height = Some(512);
    assert!(policy(&d, &r, 0).is_err());
    d.bar_height = Some(24);
    d.schema = 1;
    assert!(policy(&d, &r, 0).is_err());
}

#[test]
fn gpu_schema3_admits_only_explicit_gpu_producer_arms() {
    use xtask::shell_timing_native::ProducerKind;
    let (mut d, mut r) = fixture();
    d.schema = 3;
    d.bar_height = Some(24);
    r.producer = ProducerKind::Gpu;
    // I: IPC bytes.
    r.source = ImageSource::Bytes;
    r.wire = "ipc".into();
    policy(&d, &r, 0).unwrap();
    // S: files sealed; R: files owned_ring; G: files dma_buf.
    r.wire = "files".into();
    r.source = ImageSource::Isb;
    policy(&d, &r, 0).unwrap();
    r.source = ImageSource::OwnedRing;
    policy(&d, &r, 0).unwrap();
    d.bar_height = Some(512);
    policy(&d, &r, 0).unwrap();
    r.wire = "ipc".into();
    assert!(policy(&d, &r, 0).is_err());
    r.wire = "files".into();
    r.source = ImageSource::DmaBuf;
    policy(&d, &r, 0).unwrap();
    d.bar_height = Some(512);
    policy(&d, &r, 0).unwrap();
    // The binding, not the source, qualifies: CPU producer refused.
    r.producer = ProducerKind::Cpu;
    assert!(policy(&d, &r, 0).is_err());
    r.producer = ProducerKind::Gpu;
    // Wrong wire for the source refused both ways.
    r.source = ImageSource::Bytes;
    assert!(policy(&d, &r, 0).is_err());
    r.wire = "ipc".into();
    r.source = ImageSource::DmaBuf;
    assert!(policy(&d, &r, 0).is_err());
    // Unpinned or wrong bar height refused.
    d.bar_height = None;
    r.source = ImageSource::Bytes;
    assert!(policy(&d, &r, 0).is_err());
    d.bar_height = Some(108);
    assert!(policy(&d, &r, 0).is_err());
    // Frozen schemas refuse the GPU binding and the new field.
    d.bar_height = None;
    d.schema = 1;
    r.wire = "files".into();
    r.source = ImageSource::Isb;
    assert!(policy(&d, &r, 0).is_err());
    d.bar_height = Some(24);
    r.producer = ProducerKind::Cpu;
    assert!(policy(&d, &r, 0).is_err());
}

#[test]
fn draw_diagnostic_requires_helper_pin_and_explicit_updating_workload() {
    let (mut d, mut r) = fixture();
    d.schema = 2;
    d.workload = Some(DrawWorkload::Retained);
    r.reference_workload = d.workload;
    r.source = ImageSource::Draw;
    assert!(policy(&d, &r, 0).is_err());
    d.raster = Some(Artifact {
        path: "/not-executed/sophia-draw-raster".into(),
        sha256: "a".repeat(64),
    });
    policy(&d, &r, 0).unwrap();
    for arm in [ImageSource::Mixed, ImageSource::OwnedRing] {
        r.source = arm;
        policy(&d, &r, 0).unwrap();
    }
    r.source = ImageSource::Isb;
    assert!(policy(&d, &r, 0).is_err());
    r.source = ImageSource::Draw;
    d.workload = Some(DrawWorkload::Idle);
    assert!(policy(&d, &r, 0).is_err()); // Idle needs time anchors, not frame intervals.
    d.schema = 1;
    d.workload = None;
    r.source = ImageSource::OwnedRing;
    assert!(policy(&d, &r, 0).is_err()); // No silent alteration of frozen schema 1.
}
