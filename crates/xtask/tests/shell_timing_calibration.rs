//! Calibration preparation and report over supplied fixtures. Nothing is
//! launched. Sources are a throwaway repository whose commit is signed with
//! a throwaway SSH key through repo-local allowed signers; artifacts are
//! hashed, never executed; captures are generated text; the environment
//! provider in report tests is a supplied verdict, never authentication.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use xtask::shell_timing_calibration::*;
use xtask::shell_timing_campaign::AttemptOutcome;
use xtask::shell_timing_campaign_import::{
    ArmFiles, MonotonicDomain, RUN_DIRECTORY, VerifiedEnvironment,
};
use xtask::shell_timing_campaign_provenance::ProvenanceTarget;
use xtask::shell_timing_environment::{Capture, Expect, Interval, SourceKind, SourceText, Vrr};
use xtask::shell_timing_native::{Artifact, Request, Source};

const OUTPUT: u64 = 3;

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

fn signed_source(root: &Path) -> Source {
    let key = root.join("signing-key");
    let status = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "fixture", "-f"])
        .arg(&key)
        .stdin(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let public = std::fs::read_to_string(key.with_extension("pub")).unwrap();
    let allowed = root.join("allowed-signers");
    std::fs::write(
        &allowed,
        format!("fixture@example.invalid namespaces=\"git\" {public}"),
    )
    .unwrap();
    let repo = root.join("source");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    for (name, value) in [
        ("user.name", "Fixture"),
        ("user.email", "fixture@example.invalid"),
        ("gpg.format", "ssh"),
        ("user.signingkey", key.to_str().unwrap()),
        ("gpg.ssh.allowedSignersFile", allowed.to_str().unwrap()),
    ] {
        git(&repo, &["config", name, value]);
    }
    std::fs::write(repo.join("README"), "fixture source\n").unwrap();
    git(&repo, &["add", "README"]);
    git(&repo, &["commit", "-q", "-S", "-m", "fixture"]);
    Source {
        commit: git(&repo, &["rev-parse", "HEAD"]).trim().into(),
        path: repo,
    }
}

fn put(directory: &Path, name: &str, bytes: &[u8], mode: u32) -> Artifact {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join(name);
    std::fs::write(&path, bytes).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    Artifact {
        path,
        sha256: sha256(bytes),
    }
}

/// Plan knobs a test may change before the plan is written.
#[derive(Clone)]
struct Shape {
    sealed: bool,
    instrumentation: &'static str,
    frames: usize,
    probe: u64,
    height: u64,
    readback: bool,
    output: u64,
    sophia: &'static str,
    /// Extra session policy, outside the shell component.
    policy: &'static str,
}

impl Shape {
    fn bytes() -> Self {
        Self {
            sealed: false,
            instrumentation: "cpu-only",
            frames: 1000,
            probe: 0,
            height: 24,
            readback: true,
            output: OUTPUT,
            sophia: "sophia",
            policy: "",
        }
    }
    fn sealed() -> Self {
        Self {
            sealed: true,
            ..Self::bytes()
        }
    }
}

/// A temporary calibration world: signed source, hashed artifacts, plans.
struct World {
    root: PathBuf,
    source: Source,
    artifacts: PathBuf,
}

impl World {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("calibration-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let source = signed_source(&root);
        let artifacts = root.join("artifacts");
        std::fs::create_dir(&artifacts).unwrap();
        Self {
            root,
            source,
            artifacts,
        }
    }

    /// Write a prepared plan valid under `verify_prepared` and return its path.
    fn plan(&self, name: &str, shape: &Shape) -> PathBuf {
        use xtask::shell_timing::ImageSource;
        let mut artifacts = BTreeMap::new();
        for key in [
            "peer",
            "wm",
            "factotum",
            "pam-helper",
            "preflight",
            "runner",
        ] {
            let bytes = format!("{key}: hashed only, never executed");
            artifacts.insert(
                key.to_string(),
                put(&self.artifacts, key, bytes.as_bytes(), 0o700),
            );
        }
        let sophia = format!("{}: hashed only, never executed", shape.sophia);
        artifacts.insert(
            "sophia".into(),
            put(&self.artifacts, shape.sophia, sophia.as_bytes(), 0o700),
        );
        artifacts.insert(
            "core".into(),
            put(&self.artifacts, "core", b"schema 2\n", 0o600),
        );
        let image = if shape.sealed { "sealed" } else { "bytes" };
        let mut config = serde_json::json!({
            "wire": "files", "source": image, "output": shape.output, "frames": shape.frames,
            "height": shape.height, "hold_msec": 60000, "probe_bytes": shape.probe,
        });
        if shape.sealed {
            config["image"] = serde_json::json!({
                "max_resource_bytes": 4194304, "max_width": 4096, "max_height": 4096,
                "max_admitted_outputs": 8, "transfer_slots": 4, "transfer_timeout_ms": 2000,
            });
        }
        let peer_config = put(
            &self.artifacts,
            &format!("{name}-peer.json"),
            config.to_string().as_bytes(),
            0o600,
        );
        let profile = format!(
            "schema 1\nsession {{\n    shell-component \"timing\" \"bar\" {{ executable \"{}\"; config \"{}\"; transport \"9p2000.L\"; gpu \"denied\"; }}\n    startup\n{}}}\n",
            artifacts["peer"].path.display(),
            peer_config.path.display(),
            shape.policy
        );
        artifacts.insert("peer-config".into(), peer_config);
        artifacts.insert(
            "profile".into(),
            put(
                &self.artifacts,
                &format!("{name}-profile.kdl"),
                profile.as_bytes(),
                0o600,
            ),
        );
        let same = |s: &Source| Source {
            path: s.path.clone(),
            commit: s.commit.clone(),
        };
        let request = Request {
            sophia_source: same(&self.source),
            integration_source: same(&self.source),
            artifacts,
            tty: "/dev/tty4".into(),
            display: 94,
            input_seat: "seat0".into(),
            runtime_ms: 60_000,
            wire: "files".into(),
            source: if shape.sealed {
                ImageSource::Sealed
            } else {
                ImageSource::Bytes
            },
            output: shape.output,
            environment_readback: shape.readback,
            frames: shape.frames,
            mode_millihz: 120_000,
            instrumentation: shape.instrumentation.into(),
        };
        let argv = xtask::shell_timing_native::validate(&request).unwrap();
        let path = self.root.join(format!("{name}.plan.json"));
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({"schema":1,"request":request,"argv":argv}))
                .unwrap(),
        )
        .unwrap();
        path
    }

    fn request(&self, bytes: &Path, sealed: &Path, target: &ProvenanceTarget) -> PathBuf {
        let path = self.root.join(format!(
            "request-{}.json",
            sha256(format!("{}{}", bytes.display(), sealed.display()).as_bytes())
        ));
        std::fs::write(
            &path,
            serde_json::json!({"schema":1,"target":target,"quiet_window":"fixture, never launched",
                "bytes":bytes,"sealed":sealed})
            .to_string(),
        )
        .unwrap();
        path
    }

    /// Prepare a valid calibration and return its directory and digest.
    fn frozen(&self) -> (PathBuf, String) {
        let bytes = self.plan("bytes", &Shape::bytes());
        let sealed = self.plan("sealed", &Shape::sealed());
        let directory = self.root.join("frozen");
        let line = prepare(&self.request(&bytes, &sealed, &target()), &directory).unwrap();
        let digest = line.split("sha256=").nth(1).unwrap()[..64].to_string();
        (directory, digest)
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn target() -> ProvenanceTarget {
    ProvenanceTarget {
        card: "card0".into(),
        output: OUTPUT,
        scale_milli: 1000,
        connector: "DP-1".into(),
        width: 1920,
        height: 1080,
        mode_millihz: 120_000,
        vrr: false,
    }
}

#[test]
fn a_matching_pair_freezes_and_reopens_by_digest() {
    let world = World::new("freeze");
    let (directory, digest) = world.frozen();
    let (descriptor, plans) = verify_descriptor(&directory, &digest).unwrap();
    assert_eq!(descriptor.target, target());
    let labels: Vec<_> = descriptor.plans.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, LABELS);
    assert_eq!(plans.len(), 2);
    // The directory is create-only.
    let bytes = world.root.join("bytes.plan.json");
    let sealed = world.root.join("sealed.plan.json");
    assert!(prepare(&world.request(&bytes, &sealed, &target()), &directory).is_err());
}

#[test]
fn each_plan_must_be_its_declared_files_panel_cpu_variant() {
    let world = World::new("shape");
    let sealed = world.plan("sealed", &Shape::sealed());
    let bytes = world.plan("bytes", &Shape::bytes());
    let refuse = |b: &Path, s: &Path, name: &str| -> String {
        prepare(&world.request(b, s, &target()), &world.root.join(name)).expect_err("accepted")
    };
    // Swapped order.
    assert!(refuse(&sealed, &bytes, "swapped").contains("bytes plan is not"));
    let shape_error = "bytes plan is not a files bytes panel cpu-only calibration for the target";
    for (name, shape) in [
        (
            "rust-global",
            Shape {
                instrumentation: "rust-global",
                ..Shape::bytes()
            },
        ),
        (
            "frames",
            Shape {
                frames: 1100,
                ..Shape::bytes()
            },
        ),
        (
            "probe",
            Shape {
                probe: 4194304,
                ..Shape::bytes()
            },
        ),
        (
            "height",
            Shape {
                height: 48,
                ..Shape::bytes()
            },
        ),
        (
            "readback",
            Shape {
                readback: false,
                ..Shape::bytes()
            },
        ),
        (
            "output",
            Shape {
                output: 4,
                ..Shape::bytes()
            },
        ),
    ] {
        let plan = world.plan(name, &shape);
        assert_eq!(refuse(&plan, &sealed, name), shape_error, "{name}");
    }
    // Plans must differ only in their variant.
    let other = world.plan(
        "other-owner",
        &Shape {
            sophia: "sophia-other",
            ..Shape::sealed()
        },
    );
    assert_eq!(
        refuse(&bytes, &other, "identity"),
        "calibration plans differ beyond their variant"
    );
    // Profile policy outside the component config path is compared.
    let policy = world.plan(
        "policy",
        &Shape {
            policy: "    idle-timeout-ms 300000\n",
            ..Shape::sealed()
        },
    );
    assert_eq!(
        refuse(&bytes, &policy, "policy"),
        "calibration plans differ beyond their variant"
    );
    // Relative plan paths are refused.
    assert!(refuse(Path::new("bytes.plan.json"), &sealed, "relative").contains("absolute"));
}

#[test]
fn the_target_is_validated_and_bound_to_the_plans() {
    let world = World::new("target");
    let bytes = world.plan("bytes", &Shape::bytes());
    let sealed = world.plan("sealed", &Shape::sealed());
    for (name, t) in [
        (
            "vrr",
            ProvenanceTarget {
                vrr: true,
                ..target()
            },
        ),
        (
            "mode",
            ProvenanceTarget {
                mode_millihz: 60_000,
                ..target()
            },
        ),
        (
            "card",
            ProvenanceTarget {
                card: "gpu0".into(),
                ..target()
            },
        ),
    ] {
        assert!(
            prepare(&world.request(&bytes, &sealed, &t), &world.root.join(name)).is_err(),
            "{name}"
        );
        assert!(
            !world.root.join(name).exists(),
            "{name}: refusal created state"
        );
    }
    let other_output = ProvenanceTarget {
        output: 4,
        ..target()
    };
    assert!(
        prepare(
            &world.request(&bytes, &sealed, &other_output),
            &world.root.join("o")
        )
        .unwrap_err()
        .contains("calibration for the target")
    );
}

#[test]
fn a_changed_descriptor_or_plan_cannot_reopen() {
    let world = World::new("reopen");
    let (directory, digest) = world.frozen();
    assert!(verify_descriptor(&directory, &"f".repeat(64)).is_err());
    let path = directory.join(DESCRIPTOR);
    let original = std::fs::read(&path).unwrap();
    std::fs::write(&path, [&original[..], b" "].concat()).unwrap();
    assert_eq!(
        verify_descriptor(&directory, &digest).err().as_deref(),
        Some("calibration descriptor differs from its digest")
    );
    std::fs::write(&path, &original).unwrap();
    let plan = world.root.join("sealed.plan.json");
    let bytes = std::fs::read(&plan).unwrap();
    std::fs::write(&plan, [&bytes[..], b"\n"].concat()).unwrap();
    assert_eq!(
        verify_descriptor(&directory, &digest).err().as_deref(),
        Some("sealed plan digest differs")
    );
}

#[test]
fn a_run_is_refused_before_any_directory_without_an_attended_console() {
    let world = World::new("run");
    let (directory, digest) = world.frozen();
    let run = world.root.join("run-bytes");
    assert!(run_arm(&directory, &"0".repeat(64), "bytes", &run).is_err());
    assert_eq!(
        run_arm(&directory, &digest, "ipc", &run).err().as_deref(),
        Some("calibration label must be bytes or sealed")
    );
    // The test process owns no attended console, so launch checks refuse
    // before the run directory or any attempt record exists.
    assert!(run_arm(&directory, &digest, "bytes", &run).is_err());
    assert!(!run.exists());
}

// Report fixtures: generated cost captures, retained run files and a
// supplied environment verdict.

fn native(g: u64, completed: u64) -> String {
    format!(
        "sophia_shell_native_binding schema=1 connection_epoch=1 content_grant_epoch=2 output={OUTPUT} candidate_generation={g} native_owner=5 native_frame={g} head=7 target_generation=8 heads=1 mode_refresh_millihz=120000\nsophia_shell_native_completion schema=1 output={OUTPUT} native_owner=5 native_frame={g} heads=1 monotonic_usec={completed} timestamp_source=kernel missing_kernel_timestamp=0\n"
    )
}

/// A cpu-only panel capture: journal, counters around every frame and
/// kernel completions.
fn capture(frames: u64, sealed: bool, cpu: u64) -> String {
    let row = |process: &str, stage: &str, sequence: u64, time: u64, count: u64| {
        let pid = if process == "session" { 10 } else { 20 };
        format!(
            "sophia_comparator_counters schema=2 mode=cpu-only process={process} pid={pid} stage={stage} sequence={sequence} before_usec={time} after_usec={} cpu_ns={} allocations=0 reallocations=0 deallocations=0 bytes_allocated=0\n",
            time + 1,
            count * cpu
        )
    };
    let source = if sealed { r#","source":"sealed""# } else { "" };
    let mut journal = format!(
        "{{\"schema\":2,\"event\":\"start\",\"frames\":{frames},\"wire\":\"files\"{source},\"probe_bytes\":0}}\n"
    );
    let mut counters = row("session", "start", 0, 1_000, 0);
    let mut host = String::new();
    let mut t = 10_000;
    for g in 1..=frames {
        counters += &row("peer", "frame_start", g, t, 2 * g);
        t += 2;
        let (id, generation) = ((g - 1) % 2 + 1, (g - 1) / 2 + 1);
        journal += &format!(
            "{{\"schema\":2,\"event\":\"upload_offer\",\"upload\":{{\"connection_epoch\":1,\"content_grant_epoch\":2,\"output\":{OUTPUT},\"candidate_generation\":{g},\"resource_id\":{id},\"resource_generation\":{generation},\"width\":1920,\"height\":24,\"total_bytes\":184320,\"begin_usec\":{t}}}}}\n{{\"schema\":2,\"event\":\"upload_accepted\",\"resource_id\":{id},\"resource_generation\":{generation},\"accepted_usec\":{}}}\n",
            t + 1
        );
        t += 2;
        journal += &format!(
            "{{\"schema\":2,\"event\":\"permit\",\"generation\":{g},\"demand_usec\":{t},\"received_usec\":{}}}\n{{\"schema\":2,\"event\":\"offer\",\"connection_epoch\":1,\"content_grant_epoch\":2,\"output\":{OUTPUT},\"candidate_generation\":{g},\"submitted_monotonic_usec\":{}}}\n",
            t + 1,
            t + 2
        );
        host += &native(g, t + 3);
        journal += &format!(
            "{{\"schema\":2,\"event\":\"terminal\",\"connection_epoch\":1,\"content_grant_epoch\":2,\"output\":{OUTPUT},\"candidate_generation\":{g},\"outcome\":\"presented\",\"kind\":2,\"reason\":0,\"observed_monotonic_usec\":{}}}\n",
            t + 4
        );
        t += 5;
        counters += &row("peer", "frame_end", g, t, 2 * g + 1);
        t += 8_333;
    }
    journal += &format!(
        "{{\"schema\":2,\"event\":\"end\",\"offered\":{frames},\"completed\":{frames}}}\n"
    );
    counters += &row("session", "end", 1, t, 2 * frames + 2);
    journal + &counters + &host
}

/// Write one retained run beside its attempt record, as run-calibration-arm
/// would after a capture.
fn retain_run(world: &World, digest: &str, label: &str, cpu: u64) -> PathBuf {
    let run = world.root.join(format!("run-{label}"));
    let stage = run.join(RUN_DIRECTORY);
    std::fs::create_dir_all(&stage).unwrap();
    let plan = std::fs::read(world.root.join(format!("{label}.plan.json"))).unwrap();
    let session = capture(1000, label == "sealed", cpu);
    let mut evidence = BTreeMap::new();
    for (name, bytes) in [
        ("plan.json", plan.clone()),
        (
            "result.json",
            br#"{"schema":1,"status":"captured_only","error":null}"#.to_vec(),
        ),
        ("session.log", session.into_bytes()),
        ("environment.json", b"{}".to_vec()),
        ("environment-receipts.json", b"[]".to_vec()),
    ] {
        std::fs::write(stage.join(name), &bytes).unwrap();
        evidence.insert(name.to_string(), sha256(&bytes));
    }
    let attempt = CalibrationAttempt {
        schema: 1,
        descriptor_sha256: digest.into(),
        label: label.into(),
        plan_sha256: sha256(&plan),
        outcome: AttemptOutcome::Captured { evidence },
    };
    std::fs::write(run.join(ATTEMPT), serde_json::to_vec(&attempt).unwrap()).unwrap();
    run
}

fn environment_capture() -> Capture {
    let stat = |pid: u32, comm: &str| {
        let fields: Vec<String> = (3..=52)
            .map(|n| if n == 3 { "S".into() } else { "0".into() })
            .collect();
        format!("{pid} ({comm}) {}\n", fields.join(" "))
    };
    let status =
        |pid: u32| format!("Name:\tx\nTgid:\t{pid}\nPid:\t{pid}\nCpus_allowed_list:\t0-3\n");
    let log = format!(
        "sophia_shell_environment_output schema=1 pid=4100 output={OUTPUT} card=card0 connector=DP-1 width=1920 height=1080 refresh_millihz=120000 timing_refresh_millihz=120000 scale_milli=1000 vrr=off monotonic_usec=1500\nsophia_shell_environment_renderer schema=1 pid=4100 output={OUTPUT} card=card0 driver=amdgpu vendor=Mesa renderer=fixture version=4.6 monotonic_usec=1600\n"
    );
    let mut sources: Vec<(SourceKind, String, String)> = vec![
        (
            SourceKind::Osrelease,
            "/proc/sys/kernel/osrelease".into(),
            "6.18.55_1\n".into(),
        ),
        (
            SourceKind::ProcVersion,
            "/proc/version".into(),
            "Linux version 6.18.55_1 (fixture) #1 SMP\n".into(),
        ),
        (
            SourceKind::CpuOnline,
            "/sys/devices/system/cpu/online".into(),
            "0-3\n".into(),
        ),
        (
            SourceKind::ProcStat { pid: 4100 },
            "/proc/4100/stat".into(),
            stat(4100, "sophia"),
        ),
        (
            SourceKind::ProcStatus { pid: 4100 },
            "/proc/4100/status".into(),
            status(4100),
        ),
        (
            SourceKind::ProcStat { pid: 4200 },
            "/proc/4200/stat".into(),
            stat(4200, "peer"),
        ),
        (
            SourceKind::ProcStatus { pid: 4200 },
            "/proc/4200/status".into(),
            status(4200),
        ),
        (
            SourceKind::DrmDriver {
                card: "card0".into(),
            },
            "/sys/class/drm/card0/device/driver".into(),
            "amdgpu\n".into(),
        ),
        (SourceKind::HostLog, "host.log".into(), log),
    ];
    for cpu in 0..4 {
        sources.push((
            SourceKind::Governor { cpu },
            format!("/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor"),
            "performance\n".into(),
        ));
    }
    Capture {
        sources: sources
            .into_iter()
            .map(|(kind, origin, text)| SourceText {
                kind,
                origin,
                captured_usec: 1_200,
                text,
            })
            .collect(),
    }
}

/// Supplied verdict: reads nothing, authenticates nothing.
fn supplied(target: &ProvenanceTarget, _: &ArmFiles<'_>) -> Result<VerifiedEnvironment, String> {
    Ok(VerifiedEnvironment {
        capture: environment_capture(),
        expect: Expect {
            output: target.output,
            card: target.card.clone(),
            connector: target.connector.clone(),
            width: target.width,
            height: target.height,
            refresh_millihz: target.mode_millihz,
            scale_milli: target.scale_milli,
            vrr: Vrr::Off,
            host_pid: 4100,
            peer_pid: 4200,
            capture: Interval {
                start_usec: 1_000,
                end_usec: 100_000_000_000,
            },
            measurement: Interval {
                start_usec: 2_000,
                end_usec: 99_000_000_000,
            },
        },
        host_starttime: 1,
        peer_starttime: 2,
        clock: MonotonicDomain {
            reference: 0xEFFFFFFA,
            host: 0xEFFFFFFA,
            peer: 0xEFFFFFFA,
            capturer: 0xEFFFFFFA,
            monotonic_offset_seconds: 0,
            monotonic_offset_nanoseconds: 0,
        },
    })
}

#[test]
fn both_runs_are_rebuilt_verified_and_compared_by_source() {
    let world = World::new("report");
    let (directory, digest) = world.frozen();
    let bytes = retain_run(&world, &digest, "bytes", 100);
    let sealed = retain_run(&world, &digest, "sealed", 80);
    let report = report(&directory, &digest, [&bytes, &sealed], &supplied).unwrap();
    assert_eq!(report["calibration_sha256"], digest.as_str());
    assert!(
        report["scope"]
            .as_str()
            .unwrap()
            .contains("not a campaign condition or gate")
    );
    assert_eq!(report["arms"].as_array().unwrap().len(), 2);
    let comparison = &report["comparison"];
    assert_eq!(comparison["measure"], "cpu_ns_per_uploaded_mib");
    assert_eq!(comparison["sealed_arm"], "calibration-sealed");
    assert_eq!(comparison["bytes_arm"], "calibration-bytes");
    for arm in report["arms"].as_array().unwrap() {
        assert_eq!(arm["cost"]["instrumentation"], "cpu-only");
        assert_eq!(arm["cost"]["frames"].as_array().unwrap().len(), 1000);
        assert!(arm["cost"]["normalized"].is_object());
    }
}

#[test]
fn missing_failed_swapped_or_changed_runs_refuse_the_report() {
    let world = World::new("refuse");
    let (directory, digest) = world.frozen();
    let bytes = retain_run(&world, &digest, "bytes", 100);
    let sealed = retain_run(&world, &digest, "sealed", 80);
    let refuse = |runs: [&Path; 2]| -> String {
        report(&directory, &digest, runs, &supplied).expect_err("accepted")
    };
    assert_eq!(
        refuse([&bytes, &bytes]),
        "bytes and sealed runs must be different directories"
    );
    assert_eq!(
        refuse([&sealed, &bytes]),
        "bytes run: attempt belongs to another calibration plan"
    );
    // A changed retained capture.
    let log = sealed.join(RUN_DIRECTORY).join("session.log");
    let original = std::fs::read(&log).unwrap();
    std::fs::write(&log, [&original[..], b"x\n"].concat()).unwrap();
    assert_eq!(
        refuse([&bytes, &sealed]),
        "sealed run: session.log differs from its recorded digest"
    );
    std::fs::write(&log, &original).unwrap();
    assert!(report(&directory, &digest, [&bytes, &sealed], &supplied).is_ok());
    // A failed attempt is never a comparable run.
    let attempt = bytes.join(ATTEMPT);
    let saved = std::fs::read(&attempt).unwrap();
    let failed = CalibrationAttempt {
        schema: 1,
        descriptor_sha256: digest.clone(),
        label: "bytes".into(),
        plan_sha256: sha256(&std::fs::read(world.root.join("bytes.plan.json")).unwrap()),
        outcome: AttemptOutcome::Failed {
            error: "fixture launch failure".into(),
        },
    };
    std::fs::write(&attempt, serde_json::to_vec(&failed).unwrap()).unwrap();
    assert_eq!(
        refuse([&bytes, &sealed]),
        "bytes run: attempt failed: fixture launch failure"
    );
    std::fs::remove_file(&attempt).unwrap();
    assert!(refuse([&bytes, &sealed]).starts_with("bytes run: "));
    std::fs::write(&attempt, &saved).unwrap();
    // An attempt from another calibration.
    let other: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    let mut other = other;
    other["descriptor_sha256"] = serde_json::json!("e".repeat(64));
    std::fs::write(&attempt, other.to_string()).unwrap();
    assert_eq!(
        refuse([&bytes, &sealed]),
        "bytes run: attempt belongs to another calibration plan"
    );
}

#[test]
fn the_environment_verdict_must_match_the_target_and_bracket_the_run() {
    let world = World::new("environment");
    let (directory, digest) = world.frozen();
    let bytes = retain_run(&world, &digest, "bytes", 100);
    let sealed = retain_run(&world, &digest, "sealed", 80);
    let failing = |_: &ProvenanceTarget, _: &ArmFiles<'_>| -> Result<VerifiedEnvironment, String> {
        Err("no bound capture".into())
    };
    assert_eq!(
        report(&directory, &digest, [&bytes, &sealed], &failing)
            .err()
            .as_deref(),
        Some("bytes run: no bound capture")
    );
    let other_connector = |t: &ProvenanceTarget, f: &ArmFiles<'_>| {
        let mut v = supplied(t, f)?;
        v.expect.connector = "DP-2".into();
        Ok(v)
    };
    assert_eq!(
        report(&directory, &digest, [&bytes, &sealed], &other_connector)
            .err()
            .as_deref(),
        Some("bytes run: environment expectation differs from the calibration target")
    );
    let short = |t: &ProvenanceTarget, f: &ArmFiles<'_>| {
        let mut v = supplied(t, f)?;
        v.expect.measurement.end_usec = 20_000;
        Ok(v)
    };
    assert_eq!(
        report(&directory, &digest, [&bytes, &sealed], &short)
            .err()
            .as_deref(),
        Some("bytes run: run timestamps fall outside the declared measurement")
    );
}

type Edit = fn(&mut VerifiedEnvironment);

/// Supplied verdict for the sealed arm only, changed by `edit`.
fn sealed_differs(
    edit: Edit,
) -> impl Fn(&ProvenanceTarget, &ArmFiles<'_>) -> Result<VerifiedEnvironment, String> {
    move |target, files| {
        let mut verified = supplied(target, files)?;
        if files.arm.id == "calibration-sealed" {
            edit(&mut verified);
        }
        Ok(verified)
    }
}

fn source_text<'a>(verified: &'a mut VerifiedEnvironment, kind: &SourceKind) -> &'a mut SourceText {
    verified
        .capture
        .sources
        .iter_mut()
        .find(|source| &source.kind == kind)
        .unwrap()
}

#[test]
fn both_arms_must_share_static_and_scheduling_fact_values() {
    let world = World::new("comparable");
    let (directory, digest) = world.frozen();
    let bytes = retain_run(&world, &digest, "bytes", 100);
    let sealed = retain_run(&world, &digest, "sealed", 80);
    let differs = "bytes and sealed runs differ in static or scheduling environment";
    let edits: [(&str, Edit); 5] = [
        ("governor", |v| {
            source_text(v, &SourceKind::Governor { cpu: 2 }).text = "powersave\n".into()
        }),
        ("kernel", |v| {
            source_text(v, &SourceKind::Osrelease).text = "6.18.56_1\n".into();
            source_text(v, &SourceKind::ProcVersion).text =
                "Linux version 6.18.56_1 (fixture) #1 SMP\n".into();
        }),
        ("renderer", |v| {
            let log = &mut source_text(v, &SourceKind::HostLog).text;
            *log = log.replace("version=4.6", "version=4.5");
        }),
        ("affinity", |v| {
            source_text(v, &SourceKind::ProcStatus { pid: 4200 }).text =
                "Name:\tx\nTgid:\t4200\nPid:\t4200\nCpus_allowed_list:\t0-1\n".into()
        }),
        ("nice", |v| {
            let stat = &mut source_text(v, &SourceKind::ProcStat { pid: 4100 }).text;
            let mut fields: Vec<String> = stat.split(' ').map(String::from).collect();
            // Field 19 is nice; fields[0] is pid and comm is one token here.
            fields[18] = "5".into();
            *stat = fields.join(" ");
        }),
    ];
    for (name, edit) in edits {
        assert_eq!(
            report(
                &directory,
                &digest,
                [&bytes, &sealed],
                &sealed_differs(edit)
            )
            .err()
            .as_deref(),
            Some(differs),
            "{name}"
        );
    }
    // Capture times, pids, start times and origins are not compared.
    let incidental = sealed_differs(|v| {
        for source in &mut v.capture.sources {
            source.captured_usec = 1_300;
        }
        v.host_starttime = 11;
        v.peer_starttime = 12;
        let log = &mut source_text(v, &SourceKind::HostLog).text;
        *log = log
            .replace("monotonic_usec=1500", "monotonic_usec=1700")
            .replace("pid=4100", "pid=5100");
        for (old, new) in [(4100u32, 5100u32), (4200, 5200)] {
            for source in &mut v.capture.sources {
                source.kind = match &source.kind {
                    SourceKind::ProcStat { pid } if *pid == old => {
                        SourceKind::ProcStat { pid: new }
                    }
                    SourceKind::ProcStatus { pid } if *pid == old => {
                        SourceKind::ProcStatus { pid: new }
                    }
                    kind => kind.clone(),
                };
                source.text = source
                    .text
                    .replace(&format!("{old} ("), &format!("{new} ("))
                    .replace(&format!("\t{old}\n"), &format!("\t{new}\n"));
                source.origin = source.origin.replace(&old.to_string(), &new.to_string());
            }
        }
        v.expect.host_pid = 5100;
        v.expect.peer_pid = 5200;
    });
    report(&directory, &digest, [&bytes, &sealed], &incidental).unwrap();
}
