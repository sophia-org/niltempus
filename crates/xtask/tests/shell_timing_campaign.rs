use xtask::shell_timing_campaign::*;

#[test]
fn overlong_environment_socket_is_refused_before_reading_plans_or_creating_campaign() {
    let m = manifest();
    let root = std::env::temp_dir().join(format!(
        "campaign-path-limit-{}-{}",
        std::process::id(),
        "x".repeat(120)
    ));
    let plans = m
        .arms
        .iter()
        .map(|a| (a.id.clone(), root.join("absent-plan.json")))
        .collect();
    let error = freeze(&m, &plans, &root).unwrap_err();
    assert!(
        error.contains("campaign environment socket path is invalid"),
        "{error}"
    );
    assert!(!root.exists());
}

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
        schema: 2,
        id: "fixture".into(),
        connector: "DP-test".into(),
        width: 1920,
        height: 1080,
        mode_millihz: 120_000,
        vrr: false,
        environment: EnvironmentTarget {
            card: "card0".into(),
            output: 1,
            scale_milli: 1000,
        },
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
fn isb_only_matrix_preserves_both_rates_and_three_balanced_restarts() {
    for mode in [60_000, 120_000] {
        let mut m = manifest();
        m.schema = 3;
        m.mode_millihz = mode;
        m.arms.retain(|arm| arm.variant != Variant::B);
        assert_eq!(m.arms.len(), 72);
        m.validate().unwrap();
        let digest = m.digest().unwrap();
        m.environment.scale_milli = 2000;
        assert!(m.validate().is_err());
        m.environment.scale_milli = 1000;
        m.arms.swap(0, 1);
        assert!(m.validate().is_err());
        m.arms.swap(0, 1);
        assert_eq!(m.digest().unwrap(), digest);
        m.arms.pop();
        assert!(m.validate().is_err());
    }
    let mut legacy = manifest();
    legacy.mode_millihz = 60_000;
    assert!(
        legacy.validate().is_err(),
        "legacy captures retain their shape"
    );
}

#[test]
fn environment_target_is_frozen_and_legacy_manifests_are_refused() {
    let original = manifest().digest().unwrap();
    let mut m = manifest();
    m.schema = 1;
    assert!(m.validate().is_err());
    let mut m = manifest();
    m.environment.card = "card1".into();
    assert_ne!(m.digest().unwrap(), original);
    m.environment.output = 2;
    assert_ne!(m.digest().unwrap(), original);
    for card in ["", "card", "../card0", "card01", "renderD128"] {
        m.environment.card = card.into();
        assert!(m.validate().is_err());
    }
    let mut m = manifest();
    m.environment.output = 0;
    assert!(m.validate().is_err());
    for scale in [0, 999, 1500] {
        let mut m = manifest();
        m.environment.scale_milli = scale;
        assert!(m.validate().is_err());
    }
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
        environment_readback: true,
        external_timing: None,
    }
}

#[test]
fn arm_labels_cannot_substitute_for_prepared_workload() {
    let m = manifest();
    let a = &m.arms[2];
    let mut r = request(a);
    assert!(validate_arm_plan(&m, a, &r, 0).is_ok());
    assert!(validate_arm_plan(&m, a, &r, 4_194_304).is_err());
    r.source = xtask::shell_timing::ImageSource::Isb;
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
    let mut r = request(a);
    r.environment_readback = false;
    assert!(validate_arm_plan(&m, a, &r, 0).is_err());
    let mut r = request(a);
    r.output = m.environment.output + 1;
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
            Variant::C => ("files", ImageSource::Isb),
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
            ImageSource::Isb
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

struct AttemptFixture {
    root: std::path::PathBuf,
    directory: std::path::PathBuf,
    digest: String,
}

impl AttemptFixture {
    fn new() -> Self {
        use sha2::{Digest, Sha256};
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "campaign-attempt-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let mut m = manifest();
        let plan = b"integrity fixture; no session is executable";
        for arm in &mut m.arms {
            arm.plan_sha256 = format!("{:x}", Sha256::digest(plan));
        }
        let directory = root.join(&m.id);
        std::fs::create_dir(&directory).unwrap();
        for arm in &m.arms {
            let dir = directory.join(&arm.id);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("plan.json"), plan).unwrap();
        }
        let digest = m.digest().unwrap();
        std::fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec(&m).unwrap(),
        )
        .unwrap();
        std::fs::write(directory.join("READY"), &digest).unwrap();
        Self {
            root,
            directory,
            digest,
        }
    }

    fn claim(&self, index: usize) -> Result<Attempt, String> {
        claim_attempt(&self.directory, &self.digest, &format!("arm-{index}"))
    }
}

impl Drop for AttemptFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn attempts_cannot_skip_retry_or_overlap_and_failures_remain() {
    let f = AttemptFixture::new();
    assert!(f.claim(1).is_err());
    let first = f.claim(0).unwrap();
    assert_eq!(first.arm().id, "arm-0");
    assert!(f.claim(0).is_err());
    assert!(f.claim(1).is_err());
    first
        .finish(AttemptOutcome::Failed {
            error: "fixture launch failure".into(),
        })
        .unwrap();
    let bytes = std::fs::read(f.directory.join("arm-0/attempt/result.json")).unwrap();
    let (outcome, digest) = read_finished(&f.directory, &f.digest, "arm-0").unwrap();
    assert!(matches!(outcome, AttemptOutcome::Failed { .. }));
    use sha2::{Digest, Sha256};
    assert_eq!(digest, format!("{:x}", Sha256::digest(&bytes)));
    assert!(f.claim(0).is_err(), "a failed arm cannot be replaced");
    let next = f.claim(1).unwrap();
    assert_eq!(
        std::fs::read(f.directory.join("arm-0/attempt/result.json")).unwrap(),
        bytes
    );
    drop(next);
    assert!(
        f.claim(1).is_err(),
        "dropping a handle does not remove its claim"
    );
    assert!(f.claim(2).is_err(), "unfinished arm blocks progression");
    assert!(read_finished(&f.directory, &f.digest, "arm-1").is_err());
}

#[test]
fn attempt_results_are_bound_to_claims_and_previous_results() {
    let f = AttemptFixture::new();
    f.claim(0)
        .unwrap()
        .finish(AttemptOutcome::Captured {
            evidence: std::collections::BTreeMap::from([("fixture.json".into(), "a".repeat(64))]),
        })
        .unwrap();
    f.claim(1)
        .unwrap()
        .finish(AttemptOutcome::Failed {
            error: "first cause".into(),
        })
        .unwrap();
    let path = f.directory.join("arm-0/attempt/result.json");
    let original = std::fs::read(&path).unwrap();
    let changed = String::from_utf8(original.clone())
        .unwrap()
        .replace(&"a".repeat(64), &"b".repeat(64));
    std::fs::write(&path, changed).unwrap();
    assert!(read_finished(&f.directory, &f.digest, "arm-1").is_err());
    assert!(
        f.claim(2).is_err(),
        "previous result hash binds the sequence"
    );
    std::fs::write(&path, original).unwrap();
    let third = f.claim(2).unwrap();
    std::fs::write(f.directory.join("arm-2/attempt/claim.json"), b"replaced").unwrap();
    assert!(
        third
            .finish(AttemptOutcome::Failed {
                error: "fixture".into()
            })
            .is_err()
    );
}

#[test]
fn finishing_refuses_a_predecessor_replaced_during_the_run() {
    let f = AttemptFixture::new();
    f.claim(0)
        .unwrap()
        .finish(AttemptOutcome::Failed {
            error: "first".into(),
        })
        .unwrap();
    let second = f.claim(1).unwrap();
    let path = f.directory.join("arm-0/attempt/result.json");
    let changed = std::fs::read_to_string(&path)
        .unwrap()
        .replace("first", "other");
    std::fs::write(path, changed).unwrap();
    assert!(
        second
            .finish(AttemptOutcome::Failed {
                error: "second".into()
            })
            .is_err()
    );
    assert!(!f.directory.join("arm-1/attempt/result.json").exists());
}

#[test]
fn concurrent_claimants_cannot_both_start_the_same_arm() {
    let f = AttemptFixture::new();
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            barrier.wait();
            f.claim(0)
        });
        let b = scope.spawn(|| {
            barrier.wait();
            f.claim(0)
        });
        let results = [a.join().unwrap(), b.join().unwrap()];
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    });
    assert!(f.claim(0).is_err());
}
