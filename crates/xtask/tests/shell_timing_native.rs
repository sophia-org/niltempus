use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::PathBuf};
use xtask::shell_timing_native::{Artifact, Request, Source, split_capture, validate};

struct Fixture {
    root: PathBuf,
    request: Request,
}
impl Fixture {
    fn new(name: &str, wire: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("native-timing-{}-{name}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let mut this = Self {
            root: root.clone(),
            request: Request {
                sophia_source: Source {
                    path: root.clone(),
                    commit: "a".repeat(40),
                },
                integration_source: Source {
                    path: root,
                    commit: "b".repeat(40),
                },
                artifacts: BTreeMap::new(),
                tty: "/dev/tty4".into(),
                display: 94,
                input_seat: "seat0".into(),
                runtime_ms: 60_000,
                wire: wire.into(),
                source: xtask::shell_timing::ImageSource::Bytes,
                output: 1,
                frames: 3,
                mode_millihz: 60_000,
                instrumentation: "none".into(),
                environment_readback: false,
                external_timing: None,
                reference_workload: None,
                producer: xtask::shell_timing_native::ProducerKind::Cpu,
            },
        };
        // These fixtures are hashed only. None may be executed by validation.
        for key in [
            "sophia",
            "peer",
            "wm",
            "factotum",
            "pam-helper",
            "preflight",
            "runner",
        ] {
            this.put(key, b"must not execute", true);
        }
        this.put("core", b"schema 2\n", false);
        this.put(
            "peer-config",
            format!(r#"{{"wire":"{wire}","output":1,"frames":3,"height":24,"hold_msec":60000}}"#)
                .as_bytes(),
            false,
        );
        let transport = if wire == "files" {
            "9p2000.L"
        } else {
            "comparison-ipc"
        };
        let profile = format!(
            r#"schema 1
session {{
    shell-component "timing" "bar" {{ executable "{}"; config "{}"; transport "{transport}"; gpu "denied"; }}
    startup
}}
"#,
            this.request.artifacts["peer"].path.display(),
            this.request.artifacts["peer-config"].path.display()
        );
        this.put("profile", profile.as_bytes(), false);
        this
    }
    fn put(&mut self, key: &str, bytes: &[u8], executable: bool) {
        let path = self.root.join(key);
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(
            &path,
            fs::Permissions::from_mode(if executable { 0o700 } else { 0o600 }),
        )
        .unwrap();
        self.request.artifacts.insert(
            key.into(),
            Artifact {
                path,
                sha256: format!("{:x}", Sha256::digest(bytes)),
            },
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn capture_preserves_failed_client_records_and_refuses_interleaved_or_partial_writes() {
    let client = "{\"schema\":1,\"event\":\"failed\",\"error\":\"timeout\"}\n";
    let native = "sophia_shell_native_completion schema=1\n";
    let (actual, host) = split_capture(&format!(
        "unrelated host record\n{client}2026 INFO target: {native}"
    ))
    .unwrap();
    assert_eq!(actual, client);
    assert_eq!(host, native);
    assert!(xtask::shell_timing::collect(&actual, 1).is_err());
    for invalid in [
        format!("{client}{native}").trim_end().to_owned(),
        format!("{{\"schema\":1HOST_INTERLEAVED\n{native}"),
        client.into(),
        native.into(),
    ] {
        assert!(split_capture(&invalid).is_err());
    }
}

#[test]
fn capture_accepts_profile_volume_but_keeps_record_bounds() {
    let evidence = "{\"schema\":1,\"event\":\"failed\",\"error\":\"timeout\"}\nsophia_shell_native_completion schema=1\n";
    let profile = "sophia_comparator_profile schema=1 phase=owner\n";
    let capture = format!("{}{evidence}", profile.repeat(180_000));
    let (client, host) = split_capture(&capture).unwrap();
    assert_eq!(client.lines().count(), 1);
    assert_eq!(host, "sophia_shell_native_completion schema=1\n");
    assert!(split_capture(&format!("{}{evidence}", profile.repeat(500_000))).is_err());
    assert!(split_capture(&format!("{}\n{evidence}", "x".repeat(65537))).is_err());
}

#[test]
fn environment_capture_preserves_unknown_and_duplicate_records_for_validation() {
    use xtask::shell_timing_native::environment_capture;
    let output = "sophia_shell_environment_output schema=1\n";
    let unavailable = "sophia_shell_environment_unavailable schema=1\n";
    let json = "{\"error\":\"sophia_shell_environment_output schema=1\"}\n";
    let text = format!("2026 INFO host: {output}{json}2026 WARN host: {unavailable}{output}");
    assert_eq!(
        environment_capture(&text).unwrap(),
        format!("{output}{unavailable}{output}")
    );
    assert!(environment_capture(json).is_err());
    assert!(environment_capture(output.trim_end()).is_err());
    assert!(environment_capture(&format!("{}\n{output}", "x".repeat(65537))).is_err());
}

#[test]
fn environment_observation_is_explicit_in_the_frozen_host_arguments() {
    let mut f = Fixture::new("environment-argv", "files");
    let original = validate(&f.request).unwrap();
    assert!(
        !original
            .iter()
            .any(|arg| arg == "--output-environment-readback")
    );
    f.request.environment_readback = true;
    let observed = validate(&f.request).unwrap();
    assert_eq!(&observed[..original.len()], original);
    assert_eq!(observed.last().unwrap(), "--output-environment-readback");
    let mut legacy = serde_json::to_value(&f.request).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("environment_readback");
    let legacy: Request = serde_json::from_value(legacy).unwrap();
    assert!(!legacy.environment_readback);
    assert_eq!(validate(&legacy).unwrap(), original);
}

#[test]
fn instrumentation_modes_are_explicit_and_bounded() {
    for mode in ["rust-global", "cpu-only", "profile", "progress"] {
        let mut f = Fixture::new(mode, "files");
        f.request.instrumentation = mode.into();
        assert!(validate(&f.request).is_ok());
        f.request.runtime_ms = 60_001;
        assert!(validate(&f.request).is_err());
    }
    let mut f = Fixture::new("invalid-mode", "files");
    f.request.instrumentation = "unknown".into();
    assert!(validate(&f.request).is_err());
}

#[test]
fn sealed_preparation_requires_matching_files_source_without_running_artifacts() {
    use xtask::shell_timing::ImageSource;
    let mut f = Fixture::new("sealed", "files");
    f.request.source = ImageSource::Isb;
    assert!(validate(&f.request).is_err());
    f.put("peer-config", br#"{"wire":"files","source":"sealed","output":1,"frames":3,"height":24,"hold_msec":60000}"#, false);
    assert!(validate(&f.request).is_err());
    let config = serde_json::json!({
        "wire":"files", "source":"sealed", "output":1, "frames":3,
        "height":24, "hold_msec":60000,
        "image": { "max_resource_bytes":4194304, "max_width":4096,
            "max_height":4096, "max_admitted_outputs":8,
            "transfer_slots":4, "transfer_timeout_ms":2000 }
    });
    f.put("peer-config", config.to_string().as_bytes(), false);
    assert!(validate(&f.request).is_ok());
    for key in [
        "max_resource_bytes",
        "max_width",
        "max_height",
        "max_admitted_outputs",
        "transfer_slots",
        "transfer_timeout_ms",
    ] {
        for value in [
            serde_json::Value::Null,
            serde_json::json!(0),
            serde_json::json!(-1),
        ] {
            let mut bad = config.clone();
            bad["image"][key] = value;
            f.put("peer-config", bad.to_string().as_bytes(), false);
            assert!(validate(&f.request).is_err(), "accepted invalid {key}");
        }
        let mut bad = config.clone();
        bad["image"].as_object_mut().unwrap().remove(key);
        f.put("peer-config", bad.to_string().as_bytes(), false);
        assert!(validate(&f.request).is_err(), "accepted missing {key}");
    }
    let mut bad = config.clone();
    bad["image"]["extra"] = serde_json::json!(1);
    f.put("peer-config", bad.to_string().as_bytes(), false);
    assert!(validate(&f.request).is_err());
    bad = config.clone();
    bad["image"]["transfer_slots"] = serde_json::json!(u64::from(u32::MAX) + 1);
    f.put("peer-config", bad.to_string().as_bytes(), false);
    assert!(validate(&f.request).is_err());
    f.put("peer-config", config.to_string().as_bytes(), false);
    f.request.source = ImageSource::Bytes;
    assert!(validate(&f.request).is_err());
    bad = config.clone();
    bad["source"] = serde_json::json!("bytes");
    f.put("peer-config", bad.to_string().as_bytes(), false);
    assert!(validate(&f.request).is_err());
    f.request.source = ImageSource::Isb;
    f.request.wire = "ipc".into();
    f.put(
        "peer-config",
        br#"{"wire":"ipc","source":"sealed","output":1,"frames":3,"height":24,"hold_msec":60000}"#,
        false,
    );
    assert!(validate(&f.request).is_err());
}

#[test]
fn dma_buf_preparation_requires_files_fresh_direct_and_no_image_bounds() {
    use xtask::shell_timing::ImageSource;
    use xtask::shell_timing_native::ProducerKind;
    let mut f = Fixture::new("dma", "files");
    f.request.source = ImageSource::DmaBuf;
    f.request.producer = ProducerKind::Gpu;
    // The profile must grant the GPU directly for a DMA peer.
    let profile = fs::read_to_string(f.request.artifacts["profile"].path.clone()).unwrap();
    f.put(
        "profile",
        profile
            .replace("gpu \"denied\"", "gpu \"direct\"")
            .as_bytes(),
        false,
    );
    let good = serde_json::json!({
        "wire":"files", "source":"dma_buf", "output":1, "frames":3,
        "height":24, "hold_msec":60000
    });
    f.put("peer-config", good.to_string().as_bytes(), false);
    assert!(validate(&f.request).is_ok());
    for (key, value) in [
        (
            "image",
            serde_json::json!({ "max_resource_bytes":4194304, "max_width":4096,
            "max_height":4096, "max_admitted_outputs":8,
            "transfer_slots":4, "transfer_timeout_ms":2000 }),
        ),
        ("probe_bytes", serde_json::json!(4194304)),
        ("ring_slots", serde_json::json!(2)),
        ("interval_msec", serde_json::json!(250)),
        ("workload", serde_json::json!("retained")),
        ("retained_resources", serde_json::json!(2)),
    ] {
        let mut bad = good.clone();
        bad[key] = value;
        f.put("peer-config", bad.to_string().as_bytes(), false);
        assert!(
            validate(&f.request).is_err(),
            "accepted dma config with {key}"
        );
    }
    f.put("peer-config", good.to_string().as_bytes(), false);
    f.request.source = ImageSource::Isb;
    assert!(validate(&f.request).is_err());
    f.request.source = ImageSource::DmaBuf;
    f.request.wire = "ipc".into();
    f.put(
        "peer-config",
        br#"{"wire":"ipc","source":"dma_buf","output":1,"frames":3,"height":24,"hold_msec":60000}"#,
        false,
    );
    assert!(validate(&f.request).is_err());
}

#[test]
fn producer_binding_controls_the_gpu_grant_for_every_source() {
    use xtask::shell_timing::ImageSource;
    use xtask::shell_timing_native::ProducerKind;
    // DMA without the GPU binding is refused before any profile check.
    let mut f = Fixture::new("binding-dma-cpu", "files");
    f.request.source = ImageSource::DmaBuf;
    f.put(
        "peer-config",
        br#"{"wire":"files","source":"dma_buf","output":1,"frames":3,"height":24,"hold_msec":60000}"#,
        false,
    );
    assert!(validate(&f.request).is_err());
    // GPU binding with a denied profile is refused for sealed (S)...
    let mut f = Fixture::new("binding-sealed", "files");
    f.request.source = ImageSource::Isb;
    f.request.producer = ProducerKind::Gpu;
    let config = serde_json::json!({
        "wire":"files", "source":"sealed", "output":1, "frames":3,
        "height":24, "hold_msec":60000,
        "image": { "max_resource_bytes":4194304, "max_width":4096,
            "max_height":4096, "max_admitted_outputs":8,
            "transfer_slots":4, "transfer_timeout_ms":2000 }
    });
    f.put("peer-config", config.to_string().as_bytes(), false);
    assert!(validate(&f.request).is_err());
    let profile = fs::read_to_string(f.request.artifacts["profile"].path.clone()).unwrap();
    f.put(
        "profile",
        profile
            .replace("gpu \"denied\"", "gpu \"direct\"")
            .as_bytes(),
        false,
    );
    assert!(validate(&f.request).is_ok());
    // ...and for the IPC bytes arm (I) as well.
    let mut f = Fixture::new("binding-ipc", "ipc");
    f.request.producer = ProducerKind::Gpu;
    assert!(validate(&f.request).is_err());
    let profile = fs::read_to_string(f.request.artifacts["profile"].path.clone()).unwrap();
    f.put(
        "profile",
        profile
            .replace("gpu \"denied\"", "gpu \"direct\"")
            .as_bytes(),
        false,
    );
    assert!(validate(&f.request).is_ok());
    // The SDK owned-ring arm (R) uses the same GPU grant as DMA.
    let mut f = Fixture::new("binding-ring", "files");
    f.request.source = ImageSource::OwnedRing;
    f.request.producer = ProducerKind::Gpu;
    f.put("peer-config", br#"{"wire":"files","source":"owned_ring","output":1,"frames":3,"height":24,"hold_msec":60000,"ring_slots":2}"#, false);
    assert!(validate(&f.request).is_err());
    let profile = fs::read_to_string(f.request.artifacts["profile"].path.clone()).unwrap();
    f.put(
        "profile",
        profile
            .replace("gpu \"denied\"", "gpu \"direct\"")
            .as_bytes(),
        false,
    );
    assert!(validate(&f.request).is_ok());
    // Historical default: CPU binding with a direct profile is refused.
    let mut f = Fixture::new("binding-cpu-direct", "files");
    let profile = fs::read_to_string(f.request.artifacts["profile"].path.clone()).unwrap();
    f.put(
        "profile",
        profile
            .replace("gpu \"denied\"", "gpu \"direct\"")
            .as_bytes(),
        false,
    );
    assert!(validate(&f.request).is_err());
}

#[test]
fn external_timing_requires_matching_interval_and_a_complete_idle_hold() {
    use xtask::shell_timing::ImageSource;
    use xtask::shell_timing_native::ExternalTiming;
    let mut f = Fixture::new("external-timing", "files");
    f.request.source = ImageSource::Isb;
    let mut config = serde_json::json!({
        "wire":"files", "source":"sealed", "output":1, "frames":3,
        "height":24, "hold_msec":60000, "interval_msec":250,
        "image": { "max_resource_bytes":4194304, "max_width":4096,
            "max_height":4096, "max_admitted_outputs":8,
            "transfer_slots":4, "transfer_timeout_ms":2000 }
    });
    f.put("peer-config", config.to_string().as_bytes(), false);
    assert!(validate(&f.request).is_err()); // no accidental old fresh plan
    f.request.external_timing = Some(ExternalTiming::Bursty {
        interval_msec: 250,
        warmup: 1,
    });
    assert!(validate(&f.request).is_ok());
    for interval in [
        serde_json::json!(0),
        serde_json::json!(251),
        serde_json::Value::Null,
    ] {
        let mut bad = config.clone();
        bad["interval_msec"] = interval;
        f.put("peer-config", bad.to_string().as_bytes(), false);
        assert!(validate(&f.request).is_err());
    }
    f.put("peer-config", config.to_string().as_bytes(), false);
    f.request.external_timing = Some(ExternalTiming::Bursty {
        interval_msec: 250,
        warmup: 3,
    });
    assert!(validate(&f.request).is_err());
    f.request.external_timing = Some(ExternalTiming::Idle { seconds: 60 });
    f.request.frames = 1;
    config["frames"] = 1.into();
    config.as_object_mut().unwrap().remove("interval_msec");
    f.put("peer-config", config.to_string().as_bytes(), false);
    assert!(validate(&f.request).is_err()); // 60s hold cannot cover +2..+62s
    f.request.runtime_ms = 90_000;
    config["hold_msec"] = 90_000.into();
    f.put("peer-config", config.to_string().as_bytes(), false);
    assert!(validate(&f.request).is_ok());
    f.request.instrumentation = "cpu-only".into();
    assert!(validate(&f.request).is_err());
}

#[test]
fn retained_peer_config_requires_files_sealed_and_an_explicit_pool() {
    use xtask::shell_timing::ImageSource;
    let mut f = Fixture::new("retained-config", "files");
    f.request.source = ImageSource::Isb;
    let config = |extra: &str| {
        format!(
            r#"{{"wire":"files","source":"sealed","output":1,"frames":3,"height":24,"hold_msec":60000,"image":{{"max_resource_bytes":1,"max_width":1,"max_height":1,"max_admitted_outputs":1,"transfer_slots":1,"transfer_timeout_ms":1}}{extra}}}"#
        )
    };
    for (extra, valid) in [
        (r#","workload":"retained","retained_resources":2"#, true),
        (r#","workload":"retained","retained_resources":8"#, true),
        (r#","workload":"fresh""#, true),
        ("", true),
        (r#","workload":"retained""#, false),
        (r#","workload":"retained","retained_resources":1"#, false),
        (r#","workload":"retained","retained_resources":9"#, false),
        (r#","workload":"retained","retained_resources":0"#, false),
        (r#","retained_resources":4"#, false),
        (r#","workload":"fresh","retained_resources":4"#, false),
        (r#","workload":"unknown","retained_resources":4"#, false),
        // Explicit nulls are not absence (peer retained::parse parity).
        (r#","workload":null"#, false),
        (r#","retained_resources":null"#, false),
        (r#","workload":"retained","retained_resources":null"#, false),
        (r#","workload":"retained","retained_resources":4.5"#, false),
    ] {
        f.put("peer-config", config(extra).as_bytes(), false);
        assert_eq!(validate(&f.request).is_ok(), valid, "config {extra}");
    }
    // Retained over bytes or ipc is refused even with a valid pool.
    f.request.source = ImageSource::Bytes;
    f.put(
        "peer-config",
        config(r#","workload":"retained","retained_resources":4"#)
            .replace("sealed", "bytes")
            .as_bytes(),
        false,
    );
    assert!(validate(&f.request).is_err());
}

#[test]
fn owned_ring_plan_requires_explicit_slots_and_refuses_unsupported_workloads() {
    use xtask::shell_timing::ImageSource;
    let mut f = Fixture::new("ring-config", "files");
    f.request.source = ImageSource::OwnedRing;
    let base = serde_json::json!({"wire":"files","source":"owned_ring","output":1,
        "frames":3,"height":24,"hold_msec":60000,"ring_slots":2});
    f.put("peer-config", base.to_string().as_bytes(), false);
    assert!(validate(&f.request).is_ok());
    for (key, value) in [
        ("ring_slots", serde_json::json!(1)),
        ("ring_slots", serde_json::json!(5)),
        ("ring_slots", serde_json::Value::Null),
        ("workload", serde_json::json!("retained")),
        ("interval_msec", serde_json::json!(250)),
        ("probe_bytes", serde_json::json!(4194304)),
        (
            "image",
            serde_json::json!({"max_resource_bytes":1,"max_width":1,"max_height":1,
            "max_admitted_outputs":1,"transfer_slots":1,"transfer_timeout_ms":1}),
        ),
    ] {
        let mut config = base.clone();
        config[key] = value;
        f.put("peer-config", config.to_string().as_bytes(), false);
        assert!(validate(&f.request).is_err(), "accepted {config}");
    }
    let mut config = base.clone();
    config["workload"] = "retained".into();
    config["retained_resources"] = 8.into();
    f.put("peer-config", config.to_string().as_bytes(), false);
    assert!(validate(&f.request).is_ok());
    config["interval_msec"] = 250.into();
    f.put("peer-config", config.to_string().as_bytes(), false);
    assert!(validate(&f.request).is_err());
    let mut config = base.clone();
    config.as_object_mut().unwrap().remove("ring_slots");
    f.put("peer-config", config.to_string().as_bytes(), false);
    assert!(validate(&f.request).is_err());
    f.put("peer-config", base.to_string().as_bytes(), false);
    f.request.instrumentation = "cpu-only".into();
    assert!(validate(&f.request).is_err());
    f.request.instrumentation = "none".into();
    f.request.source = ImageSource::Bytes;
    config = base;
    config["source"] = "bytes".into();
    f.put("peer-config", config.to_string().as_bytes(), false);
    assert!(validate(&f.request).is_err());
}

#[test]
fn owned_ring_accepts_external_idle_and_bursty_timing_only_with_matching_plans() {
    use xtask::shell_timing::ImageSource;
    use xtask::shell_timing_native::ExternalTiming;
    let mut f = Fixture::new("ring-external", "files");
    f.request.source = ImageSource::OwnedRing;
    let mut config = serde_json::json!({"wire":"files","source":"owned_ring","output":1,
        "frames":3,"height":24,"hold_msec":60000,"ring_slots":2,"interval_msec":250});
    f.put("peer-config", config.to_string().as_bytes(), false);
    assert!(validate(&f.request).is_err()); // a paced peer needs a paced plan
    f.request.external_timing = Some(ExternalTiming::Bursty {
        interval_msec: 250,
        warmup: 1,
    });
    assert!(validate(&f.request).is_ok());
    for (key, value) in [
        ("interval_msec", serde_json::json!(251)),
        ("height", serde_json::json!(23)),
        ("workload", serde_json::json!("retained")),
    ] {
        let mut bad = config.clone();
        bad[key] = value;
        f.put("peer-config", bad.to_string().as_bytes(), false);
        assert!(validate(&f.request).is_err(), "accepted {bad}");
    }
    f.request.external_timing = Some(ExternalTiming::Idle { seconds: 60 });
    f.request.frames = 1;
    f.request.runtime_ms = 90_000;
    config["frames"] = 1.into();
    config["hold_msec"] = 90_000.into();
    f.put("peer-config", config.to_string().as_bytes(), false);
    assert!(validate(&f.request).is_err()); // idle declares no interval
    config.as_object_mut().unwrap().remove("interval_msec");
    f.put("peer-config", config.to_string().as_bytes(), false);
    assert!(validate(&f.request).is_ok());
    f.request.instrumentation = "cpu-only".into();
    assert!(validate(&f.request).is_err());
}

#[test]
fn captured_source_cannot_be_substituted_after_preparation() {
    use xtask::shell_timing::{ImageSource, Journal, WorkloadMode};
    use xtask::shell_timing_native::validate_captured_workload;
    let mut f = Fixture::new("capture-source", "files");
    f.request.source = ImageSource::Isb;
    // Parsing and native-custody joins have their own complete-journal controls.
    // This guard binds the parsed arm to the prepared request.
    let mut metrics = Journal {
        ring: None,
        draw: None,
        schema: 2,
        wire: "files".into(),
        source: ImageSource::Bytes,
        probe_bytes: 0,
        workload: WorkloadMode::Fresh,
        retained_resources: None,
        interval_msec: None,
        offers: vec![],
        uploads: vec![],
        permits: vec![],
        terminal_usec: vec![],
        placements: vec![],
        live_resources: vec![],
    };
    let fresh = |m: &Journal, probe: u64| {
        validate_captured_workload(m, &f.request, probe, WorkloadMode::Fresh, None)
    };
    assert!(fresh(&metrics, 0).is_err());
    metrics.source = ImageSource::Isb;
    assert!(fresh(&metrics, 0).is_ok());
    metrics.wire = "ipc".into();
    assert!(fresh(&metrics, 0).is_err());
    metrics.wire = "files".into();
    assert!(fresh(&metrics, 4194304).is_err());
    // A retained journal matches only a retained plan with the exact pool.
    metrics.schema = 3;
    metrics.workload = WorkloadMode::Retained;
    metrics.retained_resources = Some(4);
    assert!(fresh(&metrics, 0).is_err());
    assert!(
        validate_captured_workload(&metrics, &f.request, 0, WorkloadMode::Retained, Some(4))
            .is_ok()
    );
    assert!(
        validate_captured_workload(&metrics, &f.request, 0, WorkloadMode::Retained, Some(3))
            .is_err()
    );
    assert!(
        validate_captured_workload(&metrics, &f.request, 0, WorkloadMode::Retained, None).is_err()
    );
    assert!(
        validate_captured_workload(&metrics, &f.request, 0, WorkloadMode::Fresh, Some(4)).is_err()
    );
    metrics.workload = WorkloadMode::Fresh;
    metrics.retained_resources = None;
    metrics.interval_msec = Some(250);
    assert!(
        validate_captured_workload(&metrics, &f.request, 0, WorkloadMode::Fresh, None).is_err()
    );
    f.request.external_timing = Some(xtask::shell_timing_native::ExternalTiming::Bursty {
        interval_msec: 250,
        warmup: 1,
    });
    assert!(validate_captured_workload(&metrics, &f.request, 0, WorkloadMode::Fresh, None).is_ok());
    metrics.interval_msec = Some(251);
    assert!(
        validate_captured_workload(&metrics, &f.request, 0, WorkloadMode::Fresh, None).is_err()
    );
}

#[test]
fn both_wires_bind_exact_workload_without_executing_artifacts() {
    for wire in ["files", "ipc"] {
        let f = Fixture::new(wire, wire);
        let args = validate(&f.request).unwrap();
        assert_eq!(args.len(), 13);
        assert_eq!(
            args[0],
            f.request.artifacts["sophia"].path.to_str().unwrap()
        );
        assert!(args.contains(&"--max-runtime-ms=60000".into()));
        assert!(args.contains(&"--input-seat=seat0".into()));
        for (key, flag) in [
            ("factotum", "factotum-agent"),
            ("pam-helper", "factotum-pam-helper"),
        ] {
            assert!(args.contains(&format!(
                "--{flag}={}",
                f.request.artifacts[key].path.display()
            )));
        }
    }
}

#[test]
fn changed_files_and_workload_cannot_pass_a_prepared_identity() {
    let mut f = Fixture::new("changed", "files");
    fs::write(&f.request.artifacts["peer"].path, b"changed").unwrap();
    assert!(
        validate(&f.request)
            .unwrap_err()
            .contains("artifact changed")
    );
    f.put("peer", b"must not execute", true);
    f.request.frames = 4;
    assert!(validate(&f.request).is_err());
    f.request.frames = 3;
    f.request.wire = "ipc".into();
    assert!(validate(&f.request).is_err());
    f.request.wire = "files".into();
    f.request.runtime_ms = 70_000;
    assert!(validate(&f.request).is_err());
    f.request.runtime_ms = 60_000;
    f.request.input_seat = "seat0 --extra".into();
    assert!(validate(&f.request).is_err());
    f.request.input_seat = "seat0".into();
    let helper = f.request.artifacts.remove("pam-helper").unwrap();
    assert!(validate(&f.request).is_err());
    f.request.artifacts.insert("pam-helper".into(), helper);
    fs::set_permissions(
        &f.request.artifacts["factotum"].path,
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(validate(&f.request).is_err());
}

#[test]
fn helper_in_private_user_directory_is_refused_even_with_matching_hash() {
    let mut f = Fixture::new("helper-custody", "files");
    f.put("pam-helper", b"must not execute", true);
    assert!(
        xtask::shell_timing_native::validate_pam_helper(&f.request.artifacts["pam-helper"].path)
            .unwrap_err()
            .contains("root-owned, protected")
    );
    let link = f.root.join("helper-link");
    std::os::unix::fs::symlink(&f.request.artifacts["pam-helper"].path, &link).unwrap();
    f.request.artifacts.get_mut("pam-helper").unwrap().path = link;
    assert!(
        xtask::shell_timing_native::validate_pam_helper(&f.request.artifacts["pam-helper"].path)
            .unwrap_err()
            .contains("root-owned, protected")
    );
    let input = f.root.join("request.json");
    fs::write(&input, serde_json::to_vec(&f.request).unwrap()).unwrap();
    let output = f.root.join("plan.json");
    let result = xtask::shell_timing_native::run(&[
        "prepare-native".into(),
        input.display().to_string(),
        output.display().to_string(),
    ]);
    assert!(result.unwrap_err().contains("root-owned, protected"));
    assert!(!output.exists());
}

#[test]
fn peer_height_fixture_bound_is_512() {
    // Fixture bound only: 512 is the large presented panel; 513 must
    // not imply a larger (fullscreen-class) surface is supported.
    for wire in ["files", "ipc"] {
        let mut f = Fixture::new(&format!("height-{wire}"), wire);
        for (height, accepted) in [(24, true), (512, true), (513, false), (0, false)] {
            let config = serde_json::json!({"wire":wire,"output":1,"frames":3,"height":height,"hold_msec":60000});
            f.put("peer-config", config.to_string().as_bytes(), false);
            assert_eq!(
                validate(&f.request).is_ok(),
                accepted,
                "{wire} height {height}"
            );
        }
    }
}

#[test]
fn probe_configuration_is_bounded_and_bound_to_the_plan() {
    for wire in ["files", "ipc"] {
        let mut f = Fixture::new(&format!("probe-{wire}"), wire);
        for bytes in [0, 4194304, 1, 4194305] {
            let config = serde_json::json!({"wire":wire,"output":1,"frames":3,"height":24,"hold_msec":60000,"probe_bytes":bytes});
            f.put("peer-config", config.to_string().as_bytes(), false);
            assert_eq!(validate(&f.request).is_ok(), matches!(bytes, 0 | 4194304));
        }
    }
}

#[test]
fn profile_indirection_extra_clients_and_startup_are_refused() {
    let mut f = Fixture::new("profiles", "files");
    let profile = fs::read_to_string(&f.request.artifacts["profile"].path).unwrap();
    for bad in [
        profile.clone() + "include \"/tmp/elsewhere\"\n",
        profile.replace("startup", "startup \"/bin/true\""),
        profile.replace("startup", "startup {};"),
        profile.replace("startup", "shell-component \"other\" \"bar\" {}; startup"),
        profile.replace("gpu \"denied\"", "gpu \"direct\""),
        profile.replace("transport \"9p2000.L\"", "transport \"comparison-ipc\""),
        profile.replace("startup", "include \"/tmp/elsewhere\"; startup"),
    ] {
        f.put("profile", bad.as_bytes(), false);
        assert!(validate(&f.request).is_err(), "accepted {bad}");
    }
}

#[test]
fn plan_cli_rejects_tampering_before_any_run_directory_exists() {
    let f = Fixture::new("cli", "files");
    let path = f.root.join("plan.json");
    fs::write(&path, b"{}").unwrap();
    let out = f.root.join("not-created");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["shell-timing", "run-native"])
        .arg(&path)
        .arg("0".repeat(64))
        .arg(&out)
        .env_remove("SOPHIA_SHELL_TIMING_ARM")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("plan digest differs"));
    assert!(!out.exists());
}

#[test]
fn physical_launch_requires_arm_and_actual_console_before_source_or_process_work() {
    let f = Fixture::new("unarmed", "files");
    let argv = validate(&f.request).unwrap();
    let bytes =
        serde_json::to_vec(&serde_json::json!({"schema":1,"request":f.request,"argv":argv}))
            .unwrap();
    let path = f.root.join("plan.json");
    fs::write(&path, &bytes).unwrap();
    let out = f.root.join("not-created");
    for armed in [false, true] {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"));
        command
            .args(["shell-timing", "run-native"])
            .arg(&path)
            .arg(format!("{:x}", Sha256::digest(&bytes)))
            .arg(&out)
            .stdin(std::process::Stdio::null())
            .env_remove("SOPHIA_SHELL_TIMING_ARM");
        if armed {
            command.env("SOPHIA_SHELL_TIMING_ARM", "1");
        }
        let result = command.output().unwrap();
        assert!(!result.status.success());
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(
            error.contains(if armed {
                "run from the prepared console"
            } else {
                "SOPHIA_SHELL_TIMING_ARM=1"
            }),
            "{error}"
        );
        assert!(!out.exists());
    }
}

#[test]
fn observation_cannot_start_before_launch_prerequisites_or_without_its_frozen_flag() {
    use xtask::shell_timing_native::{CampaignObserver, run, run_observed};
    struct MustNotStart;
    impl CampaignObserver for MustNotStart {
        fn prepare(
            &mut self,
            _: &Request,
            _: &[String],
            _: &std::path::Path,
        ) -> Result<PathBuf, String> {
            panic!("observer must not start before attended/source checks")
        }
        fn observe(&mut self, _: u32, _: &std::path::Path) -> Result<(), String> {
            panic!("no child may have launched")
        }
        fn finish(&mut self, _: &std::path::Path) -> Result<(), String> {
            panic!("observer must not be entered before preparation")
        }
    }
    let mut f = Fixture::new("observed-unarmed", "files");
    let path = f.root.join("plan.json");
    let out = f.root.join("not-created");
    for enabled in [false, true] {
        f.request.environment_readback = enabled;
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schema":1,"request":f.request,"argv":validate(&f.request).unwrap()
        }))
        .unwrap();
        fs::write(&path, &bytes).unwrap();
        let args = vec![
            "run-native".into(),
            path.display().to_string(),
            format!("{:x}", Sha256::digest(&bytes)),
            out.display().to_string(),
        ];
        if enabled {
            assert_eq!(
                run(&args).unwrap_err(),
                "environment readback requires the campaign observer and its frozen flag"
            );
        }
        let error = run_observed(&args, &mut MustNotStart).unwrap_err();
        if !enabled {
            assert_eq!(
                error,
                "environment readback requires the campaign observer and its frozen flag"
            );
        }
        assert!(!out.exists());
    }
}

#[test]
fn campaign_plan_verification_rechecks_argv_and_artifacts_without_execution() {
    let mut fixture = Fixture::new("campaign-plan", "files");
    let argv = validate(&fixture.request).unwrap();
    let mut plan = serde_json::json!({"schema":1,"request":fixture.request,"argv":argv});
    let bytes = serde_json::to_vec(&plan).unwrap();
    let (request, probe) = xtask::shell_timing_native::verify_prepared(&bytes).unwrap();
    assert_eq!(request.wire, "files");
    assert_eq!(probe, 0);
    plan["argv"][0] = serde_json::json!("/different-owner");
    assert!(
        xtask::shell_timing_native::verify_prepared(&serde_json::to_vec(&plan).unwrap()).is_err()
    );
    fixture.put("peer-config", b"changed", false);
    assert!(xtask::shell_timing_native::verify_prepared(&bytes).is_err());
}

#[test]
fn campaign_profiles_allow_wire_paths_but_not_policy_or_panel_size_drift() {
    use xtask::shell_timing_native::campaign_profile;
    let mut files = Fixture::new("campaign-profile-files", "files");
    let ipc = Fixture::new("campaign-profile-ipc", "ipc");
    validate(&files.request).unwrap();
    validate(&ipc.request).unwrap();
    assert_eq!(
        campaign_profile(&files.request).unwrap(),
        campaign_profile(&ipc.request).unwrap()
    );
    let profile = fs::read_to_string(&files.request.artifacts["profile"].path).unwrap();
    files.put(
        "profile",
        profile
            .replace("startup", "different-policy 1; startup")
            .as_bytes(),
        false,
    );
    validate(&files.request).unwrap();
    assert_ne!(
        campaign_profile(&files.request).unwrap(),
        campaign_profile(&ipc.request).unwrap()
    );
    let peer = fs::read_to_string(&files.request.artifacts["peer-config"].path).unwrap();
    files.put(
        "peer-config",
        peer.replace("\"height\":24", "\"height\":25").as_bytes(),
        false,
    );
    assert!(campaign_profile(&files.request).is_err());
}

#[test]
fn campaign_profile_properties_are_not_erased_with_arm_values() {
    use xtask::shell_timing_native::campaign_profile;
    let mut fixture = Fixture::new("campaign-profile-properties", "files");
    let profile = fs::read_to_string(&fixture.request.artifacts["profile"].path).unwrap();
    fixture.put(
        "profile",
        profile
            .replace("transport ", "transport extra=1 ")
            .as_bytes(),
        false,
    );
    validate(&fixture.request).unwrap();
    let first = campaign_profile(&fixture.request).unwrap();
    fixture.put(
        "profile",
        profile
            .replace("transport ", "transport extra=2 ")
            .as_bytes(),
        false,
    );
    validate(&fixture.request).unwrap();
    assert_ne!(first, campaign_profile(&fixture.request).unwrap());
}

#[test]
fn sealed_campaign_policy_signature_tracks_every_adapter_bound() {
    use xtask::shell_timing_native::campaign_image_bounds;
    let mut fixture = Fixture::new("campaign-image-bounds", "files");
    assert!(campaign_image_bounds(&fixture.request).unwrap().is_none());
    let mut peer: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.request.artifacts["peer-config"].path).unwrap())
            .unwrap();
    peer["source"] = serde_json::json!("sealed");
    peer["image"] = serde_json::json!({"max_resource_bytes":4194304,"max_width":8192,"max_height":4096,"max_admitted_outputs":16,"transfer_slots":4,"transfer_timeout_ms":2000});
    fixture.request.source = xtask::shell_timing::ImageSource::Isb;
    fixture.put("peer-config", &serde_json::to_vec(&peer).unwrap(), false);
    validate(&fixture.request).unwrap();
    let expected = campaign_image_bounds(&fixture.request).unwrap();
    for field in [
        "max_resource_bytes",
        "max_width",
        "max_height",
        "max_admitted_outputs",
        "transfer_slots",
        "transfer_timeout_ms",
    ] {
        let mut changed = peer.clone();
        changed["image"][field] = serde_json::json!(changed["image"][field].as_u64().unwrap() + 1);
        fixture.put("peer-config", &serde_json::to_vec(&changed).unwrap(), false);
        assert_ne!(
            campaign_image_bounds(&fixture.request).unwrap(),
            expected,
            "{field}"
        );
    }
}

#[test]
fn reference_config_validates_against_declared_workload() {
    use xtask::shell_timing::{DrawWorkload, ImageSource};
    let mut f = Fixture::new("reference", "files");
    f.request.source = ImageSource::Mixed;
    f.request.reference_workload = Some(DrawWorkload::Bursty);
    f.put(
        "peer-config",
        br#"{"wire":"files","output":1,"frames":3,"source":"mixed","workload":"bursty"}"#,
        false,
    );
    assert!(validate(&f.request).is_ok());
    // Workload mismatch against the declared plan workload refuses.
    f.put(
        "peer-config",
        br#"{"wire":"files","output":1,"frames":3,"source":"mixed","workload":"idle"}"#,
        false,
    );
    assert!(validate(&f.request).is_err());
    // Journal spelling ring resolves to plan spelling owned_ring.
    f.request.source = ImageSource::OwnedRing;
    f.request.reference_workload = Some(DrawWorkload::Retained);
    f.put(
        "peer-config",
        br#"{"wire":"files","output":1,"frames":3,"source":"ring","workload":"retained"}"#,
        false,
    );
    assert!(validate(&f.request).is_ok());
    // Unknown keys refuse (the reference client refuses them too).
    f.put(
        "peer-config",
        br#"{"wire":"files","output":1,"frames":3,"source":"ring","workload":"retained","height":24}"#,
        false,
    );
    assert!(validate(&f.request).is_err());
}

#[test]
fn reference_capture_uses_its_config_and_checks_source_cadence_and_hold() {
    use xtask::shell_timing::{
        DrawEvidence, DrawSource, DrawWorkload, ImageSource, Journal, Offer, WorkloadMode,
    };
    use xtask::shell_timing_native::validate_captured_config;
    let mut f = Fixture::new("reference-capture", "files");
    f.request.source = ImageSource::Mixed;
    f.request.reference_workload = Some(DrawWorkload::Retained);
    let config = br#"{"wire":"files","output":1,"frames":3,"source":"mixed","workload":"retained","hold_ms":1000}"#;
    let mut metrics = Journal {
        schema: 6,
        wire: "files".into(),
        source: ImageSource::Mixed,
        probe_bytes: 0,
        workload: WorkloadMode::Fresh,
        retained_resources: None,
        interval_msec: None,
        ring: None,
        draw: Some(DrawEvidence {
            source: DrawSource::Mixed,
            workload: DrawWorkload::Retained,
            draw_ready_usec: Some(1),
            ring_ready: None,
            batches: vec![],
            value_dues: vec![],
            hold_usec: 1_000_000,
        }),
        offers: (1..=3)
            .map(|generation| Offer {
                connection_epoch: 1,
                content_grant_epoch: 1,
                output: 1,
                candidate_generation: generation,
                submitted_monotonic_usec: generation,
                outcome: "presented".into(),
            })
            .collect(),
        uploads: vec![],
        permits: vec![],
        terminal_usec: vec![],
        placements: vec![],
        live_resources: vec![],
    };
    validate_captured_config(&metrics, &f.request, config).unwrap();
    metrics.draw.as_mut().unwrap().hold_usec -= 1;
    assert!(validate_captured_config(&metrics, &f.request, config).is_err());
    metrics.draw.as_mut().unwrap().hold_usec += 1;
    metrics.draw.as_mut().unwrap().workload = DrawWorkload::Continuous;
    assert!(validate_captured_config(&metrics, &f.request, config).is_err());
    metrics.draw.as_mut().unwrap().workload = DrawWorkload::Retained;
    metrics.source = ImageSource::Draw;
    assert!(validate_captured_config(&metrics, &f.request, config).is_err());
    metrics.source = ImageSource::Mixed;
    metrics.offers[0].output = 2;
    assert!(validate_captured_config(&metrics, &f.request, config).is_err());
    metrics.offers[0].output = 1;
    f.request.reference_workload = None;
    assert!(validate_captured_config(&metrics, &f.request, config).is_err());
}

#[test]
fn prepared_reference_arms_use_reference_decoder_through_reverification() {
    use xtask::shell_timing::{DrawWorkload, ImageSource};
    use xtask::shell_timing_native::{verify_panel_prepared, verify_prepared};
    let mut fixture = Fixture::new("reference-reverify", "files");
    for (source, expected) in [
        ("ring", ImageSource::OwnedRing),
        ("draw", ImageSource::Draw),
        ("mixed", ImageSource::Mixed),
    ] {
        fixture.request.source = expected;
        fixture.request.reference_workload = Some(DrawWorkload::Retained);
        let config = serde_json::json!({"wire":"files","output":1,"frames":3,"source":source,"workload":"retained","hold_ms":5000});
        fixture.put("peer-config", &serde_json::to_vec(&config).unwrap(), false);
        let argv = validate(&fixture.request).unwrap();
        let plan = serde_json::json!({"schema":1,"request":fixture.request,"argv":argv});
        let bytes = serde_json::to_vec(&plan).unwrap();
        assert_eq!(
            verify_prepared(&bytes).unwrap_err(),
            "reference plans require the panel diagnostic"
        );
        let (request, probe) = verify_panel_prepared(&bytes).unwrap();
        assert_eq!(request.source, expected);
        assert_eq!(probe, 0);
        assert_eq!(request.reference_workload, Some(DrawWorkload::Retained));
        // A config mutation after preparation still fails artifact binding.
        fs::write(&fixture.request.artifacts["peer-config"].path, b"changed").unwrap();
        assert!(
            verify_panel_prepared(&bytes)
                .unwrap_err()
                .contains("artifact changed")
        );
    }
}
