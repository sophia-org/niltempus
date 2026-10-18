use xtask::shell_timing_campaign::*;

fn manifest() -> Manifest {
    let mut arms = Vec::new();
    for condition in [Condition::Idle, Condition::Load] {
        for (pair, order) in [
            (1, [Variant::C, Variant::B, Variant::A]),
            (2, [Variant::A, Variant::B, Variant::C]),
            (3, [Variant::B, Variant::C, Variant::A]),
        ] {
            for workload in [Workload::Panel, Workload::Bulk] {
                for measurement in [
                    Measurement::Latency,
                    Measurement::CpuOnly,
                    Measurement::RustGlobal,
                ] {
                    for variant in order {
                        arms.push(Arm {
                            id: format!("arm-{}", arms.len()),
                            condition,
                            pair,
                            workload,
                            measurement,
                            variant,
                            plan_sha256: "a".repeat(64),
                            peer_config_sha256: "b".repeat(64),
                            frames: if measurement == Measurement::Latency {
                                1100
                            } else {
                                1000
                            },
                            warmup: if measurement == Measurement::Latency {
                                100
                            } else {
                                0
                            },
                            runtime_ms: 60_000,
                        });
                    }
                }
            }
        }
    }
    Manifest {
        schema: 1,
        id: "fixture".into(),
        connector: "DP-test".into(),
        width: 1920,
        height: 1080,
        mode_millihz: 120_000,
        vrr: false,
        load_workers: 16,
        quiet_window: "Synthetic manifest; never launched".into(),
        arms,
    }
}

#[test]
fn full_matrix_is_bound_without_requiring_distinct_restart_plan_bytes() {
    let m = manifest();
    assert_eq!(m.arms.len(), 108);
    assert!(m.validate().is_ok());
    let digest = m.digest().unwrap();
    let mut changed = manifest();
    changed.arms[0].peer_config_sha256 = "c".repeat(64);
    assert_ne!(changed.digest().unwrap(), digest);
    let encoded = serde_json::to_vec(&m).unwrap();
    let decoded: Manifest = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded.digest().unwrap(), digest);
}

#[test]
fn missing_replacement_reordered_or_duplicate_arms_are_not_a_campaign() {
    let mut m = manifest();
    m.arms.pop();
    assert!(m.validate().is_err());
    let mut m = manifest();
    m.arms.push(m.arms[0].clone());
    assert!(m.validate().is_err());
    let mut m = manifest();
    m.arms.swap(0, 1);
    assert!(m.validate().is_err());
    let mut m = manifest();
    m.arms[1].id = m.arms[0].id.clone();
    assert!(m.validate().is_err());
    let mut m = manifest();
    m.arms[0].plan_sha256 = "a".repeat(63);
    assert!(m.validate().is_err());
}

#[test]
fn sampling_output_and_load_cannot_drift() {
    let mut m = manifest();
    m.arms[0].warmup = 0;
    assert!(m.validate().is_err());
    let mut m = manifest();
    m.arms[3].measurement = Measurement::Latency;
    assert!(m.validate().is_err());
    let mut m = manifest();
    m.arms[3].warmup = 100;
    assert!(m.validate().is_err());
    let mut m = manifest();
    m.mode_millihz = 60_000;
    assert!(m.validate().is_err());
    let mut m = manifest();
    m.vrr = true;
    assert!(m.validate().is_err());
    let mut m = manifest();
    m.width = 2560;
    assert!(m.validate().is_err());
    let mut m = manifest();
    m.load_workers = 15;
    assert!(m.validate().is_err());
}

fn request(arm: &Arm) -> xtask::shell_timing_native::Request {
    use xtask::shell_timing_native::{Artifact, Request, Source};
    Request {
        sophia_source: Source {
            path: "/fixture".into(),
            commit: "a".repeat(40),
        },
        integration_source: Source {
            path: "/fixture".into(),
            commit: "b".repeat(40),
        },
        artifacts: [(
            "peer-config".into(),
            Artifact {
                path: "/fixture/config".into(),
                sha256: arm.peer_config_sha256.clone(),
            },
        )]
        .into(),
        tty: "/dev/tty4".into(),
        display: 94,
        input_seat: "seat0".into(),
        runtime_ms: arm.runtime_ms,
        wire: "ipc".into(),
        source: xtask::shell_timing::ImageSource::Bytes,
        output: 1,
        frames: arm.frames,
        mode_millihz: 120_000,
        instrumentation: "none".into(),
    }
}

#[test]
fn arm_labels_cannot_substitute_for_prepared_workload() {
    let m = manifest();
    let a = &m.arms[2];
    let mut r = request(a);
    assert!(validate_arm_plan(&m, a, &r, 0).is_ok());
    assert!(validate_arm_plan(&m, a, &r, 4_194_304).is_err());
    r.source = xtask::shell_timing::ImageSource::Sealed;
    assert!(validate_arm_plan(&m, a, &r, 0).is_err());
    let mut r = request(a);
    r.instrumentation = "rust-global".into();
    assert!(validate_arm_plan(&m, a, &r, 0).is_err());
    let mut r = request(a);
    r.frames = 1000;
    assert!(validate_arm_plan(&m, a, &r, 0).is_err());
    let mut r = request(a);
    r.artifacts.get_mut("peer-config").unwrap().sha256 = "c".repeat(64);
    assert!(validate_arm_plan(&m, a, &r, 0).is_err());
}

#[test]
fn freeze_refuses_missing_plans_before_creating_state() {
    let m = manifest();
    let root = std::env::temp_dir().join(format!("campaign-no-write-{}", std::process::id()));
    assert!(!root.exists());
    assert!(freeze(&m, &Default::default(), &root).is_err());
    assert!(!root.exists());
}

#[test]
fn all_variant_workload_and_measurement_bindings_are_checked() {
    use xtask::shell_timing::ImageSource;
    let m = manifest();
    for arm in &m.arms[..18] {
        let mut r = request(arm);
        let (wire, source) = match arm.variant {
            Variant::A => ("ipc", ImageSource::Bytes),
            Variant::B => ("files", ImageSource::Bytes),
            Variant::C => ("files", ImageSource::Sealed),
        };
        r.wire = wire.into();
        r.source = source;
        r.instrumentation = match arm.measurement {
            Measurement::Latency => "none",
            Measurement::CpuOnly => "cpu-only",
            Measurement::RustGlobal => "rust-global",
        }
        .into();
        let probe = if arm.workload == Workload::Bulk {
            4_194_304
        } else {
            0
        };
        assert!(validate_arm_plan(&m, arm, &r, probe).is_ok(), "{}", arm.id);
        r.wire = if wire == "ipc" { "files" } else { "ipc" }.into();
        assert!(validate_arm_plan(&m, arm, &r, probe).is_err());
        r.wire = wire.into();
        r.source = if source == ImageSource::Bytes {
            ImageSource::Sealed
        } else {
            ImageSource::Bytes
        };
        assert!(validate_arm_plan(&m, arm, &r, probe).is_err());
        r.source = source;
        assert!(validate_arm_plan(&m, arm, &r, probe + 1).is_err());
        r.runtime_ms += 1;
        assert!(validate_arm_plan(&m, arm, &r, probe).is_err());
        r.runtime_ms -= 1;
        r.mode_millihz = 60_000;
        assert!(validate_arm_plan(&m, arm, &r, probe).is_err());
    }
}

#[test]
fn accepted_pair_order_keeps_both_files_variants_on_the_required_side_of_ipc() {
    let m = manifest();
    for group in m.arms.chunks_exact(3) {
        let ipc = group.iter().position(|a| a.variant == Variant::A).unwrap();
        assert_eq!(ipc, if group[0].pair == 2 { 0 } else { 2 });
    }
    let mut m = manifest();
    m.arms.swap(0, 2);
    assert!(
        m.validate().is_err(),
        "superseded ABC first pair must refuse"
    );
}

#[test]
fn reopening_requires_the_original_complete_evidence() {
    use sha2::{Digest, Sha256};
    use std::fs;
    // Integrity-only fixture. These bytes deliberately are not native plans;
    // verify_arm must still reject them before any execution can be considered.
    let root = std::env::temp_dir().join(format!("campaign-integrity-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let mut m = manifest();
    let bytes = b"integrity fixture, not an executable plan";
    for arm in &mut m.arms {
        arm.plan_sha256 = format!("{:x}", Sha256::digest(bytes));
    }
    let dir = root.join(&m.id);
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("manifest.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    let digest = m.digest().unwrap();
    for arm in &m.arms {
        fs::create_dir(dir.join(&arm.id)).unwrap();
        fs::write(dir.join(&arm.id).join("plan.json"), bytes).unwrap();
    }
    assert!(
        verify_frozen(&dir, &digest).is_err(),
        "no READY means partial preparation"
    );
    fs::write(dir.join("READY"), &digest).unwrap();
    assert!(verify_frozen(&dir, &digest).is_ok());
    assert!(
        verify_arm(&dir, &digest, &m.arms[0].id).is_err(),
        "integrity is not plan validity"
    );
    assert!(verify_frozen(&dir, &"f".repeat(64)).is_err());
    let last = dir.join(&m.arms.last().unwrap().id).join("plan.json");
    fs::write(&last, b"replacement").unwrap();
    assert!(
        verify_frozen(&dir, &digest).is_err(),
        "even the last arm is checked"
    );
    fs::write(&last, bytes).unwrap();
    m.quiet_window.push_str(" changed");
    fs::write(dir.join("manifest.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    assert!(verify_frozen(&dir, &digest).is_err());
    fs::remove_dir_all(root).unwrap();
}
