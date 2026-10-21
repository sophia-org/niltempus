//! Bulk cost pilot preparation and report over supplied fixtures. Nothing is
//! launched. Sources are a throwaway repository whose commit is signed with
//! a throwaway SSH key through repo-local allowed signers; artifacts are
//! hashed, never executed; captures are generated text; the environment
//! provider in report tests is a supplied verdict, never authentication.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use xtask::shell_timing_campaign::AttemptOutcome;
use xtask::shell_timing_campaign_import::{
    ArmFiles, MonotonicDomain, RUN_DIRECTORY, VerifiedEnvironment,
};
use xtask::shell_timing_campaign_provenance::ProvenanceTarget;
use xtask::shell_timing_cost_pilot::*;
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
    ipc: bool,
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
            ipc: false,
            sealed: false,
            instrumentation: "cpu-only",
            frames: 1000,
            probe: 4_194_304,
            height: 24,
            readback: true,
            output: OUTPUT,
            sophia: "sophia",
            policy: "",
        }
    }
}

/// A temporary pilot world: signed source, hashed artifacts, plans.
struct World {
    root: PathBuf,
    source: Source,
    artifacts: PathBuf,
}

impl World {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("cost-pilot-{name}-{}", std::process::id()));
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
            "wire": if shape.ipc { "ipc" } else { "files" }, "source": image, "output": shape.output, "frames": shape.frames,
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
            "schema 1\nsession {{\n    shell-component \"timing\" \"bar\" {{ executable \"{}\"; config \"{}\"; transport \"{}\"; gpu \"denied\"; }}\n    startup\n{}}}\n",
            artifacts["peer"].path.display(),
            peer_config.path.display(),
            if shape.ipc {
                "comparison-ipc"
            } else {
                "9p2000.L"
            },
            shape.policy
        );
        let profile = if shape.height == 512 {
            profile.replace(
                "gpu \"denied\";",
                "gpu \"denied\"; reservation \"top\" 512;",
            )
        } else {
            profile
        };
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
            wire: if shape.ipc { "ipc" } else { "files" }.into(),
            source: if shape.sealed {
                ImageSource::Isb
            } else {
                ImageSource::Bytes
            },
            output: shape.output,
            environment_readback: shape.readback,
            external_timing: None,
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

    fn plans(&self) -> BTreeMap<String, PathBuf> {
        self.plans_with_probe(4_194_304)
    }
    fn plans_with_probe(&self, probe: u64) -> BTreeMap<String, PathBuf> {
        PLAN_IDS
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let shape = Shape {
                    ipc: i % 3 == 0,
                    sealed: i % 3 == 2,
                    instrumentation: if i < 3 { "cpu-only" } else { "rust-global" },
                    probe,
                    ..Shape::bytes()
                };
                (id.to_string(), self.plan(id, &shape))
            })
            .collect()
    }
    fn request(&self, plans: &BTreeMap<String, PathBuf>) -> PathBuf {
        let path = self.root.join("request.json");
        std::fs::write(&path, serde_json::json!({"schema":1,"target":target(),"quiet_window":"supplied fixture, never launched","plans":plans}).to_string()).unwrap();
        path
    }
    fn frozen(&self) -> (PathBuf, String) {
        let dir = self.root.join("frozen");
        let line = prepare(&self.request(&self.plans()), &dir).unwrap();
        (dir, line.split("sha256=").nth(1).unwrap()[..64].to_string())
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

fn native(g: u64, completed: u64) -> String {
    format!(
        "sophia_shell_native_binding schema=1 connection_epoch=1 content_grant_epoch=2 output={OUTPUT} candidate_generation={g} native_owner=5 native_frame={g} head=7 target_generation=8 heads=1 mode_refresh_millihz=120000\nsophia_shell_native_completion schema=1 output={OUTPUT} native_owner=5 native_frame={g} heads=1 monotonic_usec={completed} timestamp_source=kernel missing_kernel_timestamp=0\n"
    )
}

/// A bulk capture: journal, counters around every frame and
/// kernel completions.
fn capture(frames: u64, variant: usize, allocations: bool, cpu: u64) -> String {
    capture_workload(frames, variant, allocations, cpu, true)
}
fn capture_workload(
    frames: u64,
    variant: usize,
    allocations: bool,
    cpu: u64,
    bulk: bool,
) -> String {
    let row = |process: &str, stage: &str, sequence: u64, time: u64, count: u64| {
        let pid = if process == "session" { 10 } else { 20 };
        format!(
            "sophia_comparator_counters schema=3 mode={} process={process} pid={pid} stage={stage} sequence={sequence} before_usec={time} after_usec={} cpu_ns={} allocations={} reallocations=0 deallocations=0 bytes_allocated={} minor_faults={count} major_faults=0\n",
            if allocations {
                "rust-global"
            } else {
                "cpu-only"
            },
            time + 1,
            count * cpu,
            if allocations { count } else { 0 },
            if allocations { count * 16 } else { 0 }
        )
    };
    let source = if variant == 2 {
        r#","source":"sealed""#
    } else {
        ""
    };
    let wire = if variant == 0 { "ipc" } else { "files" };
    let probe = if bulk { 4_194_304 } else { 0 };
    let mut journal = format!(
        "{{\"schema\":2,\"event\":\"start\",\"frames\":{frames},\"wire\":\"{wire}\"{source},\"probe_bytes\":{probe}}}\n"
    );
    let mut counters = row("session", "start", 0, 1_000, 0);
    let mut host = String::new();
    let mut t = 10_000;
    for g in 1..=frames {
        counters += &row("peer", "frame_start", g, t, 2 * g);
        t += 2;
        if bulk {
            for row in [
                serde_json::json!({"schema":2,"event":"upload_offer","upload":{"connection_epoch":1,"content_grant_epoch":2,"output":OUTPUT,"candidate_generation":g,"resource_id":1,"resource_generation":g,"width":1024,"height":1024,"total_bytes":4194304,"begin_usec":t}}),
                serde_json::json!({"schema":2,"event":"upload_accepted","resource_id":1,"resource_generation":g,"accepted_usec":t+1}),
                serde_json::json!({"schema":2,"event":"probe_released","resource_id":1,"resource_generation":g,"released_usec":t+2}),
            ] {
                journal += &(row.to_string() + "\n");
            }
            t += 3;
        }
        let (id, generation) = ((g - 1) % 2 + if bulk { 2 } else { 1 }, (g - 1) / 2 + 1);
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

fn retain(frozen: &Path, digest: &str, plans: &[xtask::shell_timing_calibration::VerifiedPlan]) {
    let mut previous: Option<String> = None;
    for (i, plan_index) in [0, 1, 2, 2, 1, 0, 3, 4, 5].into_iter().enumerate() {
        let plan = &plans[plan_index];
        let config: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&plan.request.artifacts["peer-config"].path).unwrap(),
        )
        .unwrap();
        let bulk = config["probe_bytes"] == 4_194_304;
        let dir = frozen.join(ARM_IDS[i]);
        let stage = dir.join(RUN_DIRECTORY);
        std::fs::create_dir_all(&stage).unwrap();
        let claim = serde_json::to_vec(&serde_json::json!({"schema":1,"pilot_sha256":digest,"arm":ARM_IDS[i],"index":i,"plan_sha256":plan.reference.sha256,"previous_result_sha256":previous})).unwrap();
        std::fs::write(dir.join("claim.json"), &claim).unwrap();
        let mut evidence = BTreeMap::new();
        for (name, bytes) in [
            ("plan.json", plan.bytes.clone()),
            (
                "result.json",
                br#"{"schema":1,"status":"captured_only","error":null}"#.to_vec(),
            ),
            (
                "session.log",
                capture_workload(1000, plan_index % 3, i >= 6, 100 + i as u64, bulk)
                    .replace("\"height\":24", &format!("\"height\":{}", config["height"]))
                    .replace(
                        "\"total_bytes\":184320",
                        &format!(
                            "\"total_bytes\":{}",
                            1920 * config["height"].as_u64().unwrap() * 4
                        ),
                    )
                    .into_bytes(),
            ),
            ("environment.json", b"{}".to_vec()),
            ("environment-receipts.json", b"[]".to_vec()),
        ] {
            std::fs::write(stage.join(name), &bytes).unwrap();
            evidence.insert(name.to_string(), sha256(&bytes));
        }
        let finished = serde_json::to_vec(&serde_json::json!({"schema":1,"claim_sha256":sha256(&claim),"outcome":AttemptOutcome::Captured{evidence}})).unwrap();
        std::fs::write(dir.join("attempt.json"), &finished).unwrap();
        previous = Some(sha256(&finished));
    }
}

#[test]
fn six_plans_freeze_nine_arms_and_refuse_policy_or_workload_drift() {
    let w = World::new("freeze");
    let (frozen, digest) = w.frozen();
    let (d, plans) = verify_descriptor(&frozen, &digest).unwrap();
    assert_eq!(d.order, ARM_IDS);
    assert_eq!(plans.len(), 6);
    assert!(verify_descriptor(&frozen, &"0".repeat(64)).is_err());
    assert!(prepare(&w.request(&w.plans()), &frozen).is_err());
    let good = w.plans();
    for (name, shape) in [
        (
            "panel",
            Shape {
                probe: 0,
                ..Shape::bytes()
            },
        ),
        (
            "policy",
            Shape {
                policy: "    unrelated 1\n",
                ..Shape::bytes()
            },
        ),
        (
            "frames",
            Shape {
                frames: 999,
                ..Shape::bytes()
            },
        ),
        (
            "instrumentation",
            Shape {
                instrumentation: "rust-global",
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
    ] {
        let mut bad = good.clone();
        bad.insert("cpu-bytes".into(), w.plan(name, &shape));
        let destination = w.root.join(name);
        assert!(prepare(&w.request(&bad), &destination).is_err(), "{name}");
        assert!(!destination.exists());
    }
}

#[test]
fn unattended_or_out_of_order_launch_never_claims_an_arm() {
    let w = World::new("launch");
    let (frozen, digest) = w.frozen();
    assert!(run_arm(&frozen, &digest, ARM_IDS[1]).is_err());
    assert!(run_arm(&frozen, &digest, ARM_IDS[0]).is_err());
    for id in ARM_IDS {
        assert!(!frozen.join(id).exists());
    }
}

#[test]
fn nine_retained_arms_rederive_and_compare_with_no_acceptance_verdict() {
    let w = World::new("report");
    let (frozen, digest) = w.frozen();
    let (_, plans) = verify_descriptor(&frozen, &digest).unwrap();
    retain(&frozen, &digest, &plans);
    let result = report(&frozen, &digest, &supplied).unwrap();
    assert_eq!(result["arms"].as_array().unwrap().len(), 9);
    assert_eq!(result["schema"], 1);
    assert!(result.get("workload").is_none());
    let comparisons = result["summary"]["comparisons"].as_array().unwrap();
    assert_eq!(comparisons.len(), 9);
    for row in comparisons {
        assert_eq!(row["outcome"], "reported_only");
        assert!(
            row["difference"]["lower"].as_i64().unwrap()
                <= row["difference"]["upper"].as_i64().unwrap()
        );
    }
    assert_eq!(
        result["summary"]["cpu_block_variation"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    for arm in result["arms"].as_array().unwrap() {
        assert_eq!(arm["cost"]["diagnostics"]["counter_schema"], 3);
        assert_eq!(
            arm["cost"]["normalized"]["transfer_probe_bytes"],
            4_194_304_000u64
        );
    }
    let drift = |target: &ProvenanceTarget, files: &ArmFiles<'_>| {
        let mut verified = supplied(target, files)?;
        if files.arm.id == ARM_IDS[1] {
            verified
                .capture
                .sources
                .iter_mut()
                .find(|s| matches!(s.kind, SourceKind::Governor { cpu: 0 }))
                .unwrap()
                .text = "powersave\n".into();
        }
        Ok(verified)
    };
    assert_eq!(
        report(&frozen, &digest, &drift).unwrap_err(),
        "pilot arms differ in static or scheduling environment"
    );
    let path = frozen.join(ARM_IDS[0]).join("run/session.log");
    let original = std::fs::read(&path).unwrap();
    std::fs::write(&path, b"changed").unwrap();
    assert!(
        report(&frozen, &digest, &supplied)
            .unwrap_err()
            .contains("session.log differs from its recorded digest")
    );
    std::fs::write(&path, original).unwrap();
    let claim = frozen.join(ARM_IDS[0]).join("claim.json");
    let original = std::fs::read(&claim).unwrap();
    let mut bad: serde_json::Value = serde_json::from_slice(&original).unwrap();
    bad["previous_result_sha256"] = serde_json::json!("bad predecessor");
    std::fs::write(&claim, bad.to_string()).unwrap();
    assert_eq!(
        report(&frozen, &digest, &supplied).unwrap_err(),
        "pilot attempt claim differs from its predecessor or plan"
    );
    std::fs::write(&claim, &original).unwrap();
    let result_path = frozen.join(ARM_IDS[0]).join("attempt.json");
    let mut result: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&result_path).unwrap()).unwrap();
    result["outcome"] = serde_json::to_value(AttemptOutcome::Failed {
        error: "fixture failure".into(),
    })
    .unwrap();
    std::fs::write(&result_path, result.to_string()).unwrap();
    assert_eq!(
        report(&frozen, &digest, &supplied).unwrap_err(),
        "cpu-1-ipc failed: fixture failure"
    );
    assert_eq!(
        run_arm(&frozen, &digest, ARM_IDS[1]).unwrap_err(),
        "cpu-1-ipc failed: fixture failure"
    );
    std::fs::remove_file(&result_path).unwrap();
    assert!(report(&frozen, &digest, &supplied).is_err());
}

#[test]
fn report_comparison_uses_cross_bounds_and_rejects_legacy_counters() {
    use xtask::shell_timing_cost::{Bounds, measure_journal};
    let mut reports = (0..9)
        .map(|i| {
            let variant = [0, 1, 2, 2, 1, 0, 0, 1, 2][i];
            let log = capture(1000, variant, i >= 6, 100);
            measure_journal(
                &log,
                &log,
                &log.lines()
                    .filter(|line| line.starts_with('{'))
                    .map(|line| line.to_owned() + "\n")
                    .collect::<String>(),
                1000,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    reports[0]
        .normalized
        .as_mut()
        .unwrap()
        .cpu_ns_per_uploaded_mib = Bounds {
        lower: 100,
        upper: 120,
    };
    reports[1]
        .normalized
        .as_mut()
        .unwrap()
        .cpu_ns_per_uploaded_mib = Bounds {
        lower: 110,
        upper: 125,
    };
    let result = comparisons(&reports).unwrap();
    assert_eq!(
        result["comparisons"][0]["difference"],
        serde_json::json!({"lower":-10,"upper":25})
    );
    assert_eq!(
        comparisons(&reports[..8]).unwrap_err(),
        "cost pilot needs all nine reports"
    );
    reports[0]
        .normalized
        .as_mut()
        .unwrap()
        .cpu_ns_per_uploaded_mib = Bounds {
        lower: 121,
        upper: 120,
    };
    assert_eq!(
        comparisons(&reports).unwrap_err(),
        "pilot metric bounds are inverted"
    );
    reports[0]
        .normalized
        .as_mut()
        .unwrap()
        .cpu_ns_per_uploaded_mib = Bounds {
        lower: 100,
        upper: 120,
    };
    reports[0].normalized.as_mut().unwrap().source = xtask::shell_timing::ImageSource::Isb;
    assert_eq!(
        comparisons(&reports).unwrap_err(),
        "pilot reports differ from the declared bulk workload or mode"
    );
    reports[0].normalized.as_mut().unwrap().source = xtask::shell_timing::ImageSource::Bytes;
    reports[0].diagnostics = None;
    assert_eq!(
        comparisons(&reports).unwrap_err(),
        "pilot requires schema-3 process and fault counters"
    );
}

#[test]
fn panel_pilot_freezes_and_replays_all_nine_without_bulk_uploads() {
    let w = World::new("panel-report");
    let request = w.request(&w.plans_with_probe(0));
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&request).unwrap()).unwrap();
    value["schema"] = serde_json::json!(2);
    value["workload"] = serde_json::json!("panel");
    std::fs::write(&request, value.to_string()).unwrap();
    let frozen = w.root.join("panel-frozen");
    let line = prepare(&request, &frozen).unwrap();
    let digest = &line.split("sha256=").nth(1).unwrap()[..64];
    let (descriptor, plans) = verify_descriptor(&frozen, digest).unwrap();
    assert_eq!(descriptor.schema, 2);
    assert_eq!(
        descriptor.workload,
        Some(xtask::shell_timing_campaign::Workload::Panel)
    );
    retain(&frozen, digest, &plans);
    let result = report(&frozen, digest, &supplied).unwrap();
    assert_eq!(result["schema"], 2);
    assert_eq!(result["workload"], "panel");
    assert_eq!(result["arms"].as_array().unwrap().len(), 9);
    for arm in result["arms"].as_array().unwrap() {
        assert_eq!(arm["cost"]["normalized"]["frame_count"], 1000);
        assert_eq!(arm["cost"]["normalized"]["presentation_bytes"], 184_320_000);
        assert_eq!(arm["cost"]["normalized"]["transfer_probe_bytes"], 0);
    }
    assert_eq!(
        result["summary"]["comparisons"].as_array().unwrap().len(),
        9
    );
}

#[test]
fn workload_must_be_explicit_versioned_and_match_the_plans() {
    let w = World::new("workload-refusals");
    let request = w.request(&w.plans_with_probe(0));
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&request).unwrap()).unwrap();
    for (index, (schema, workload)) in [
        (1, Some("panel")),
        (2, None),
        (2, Some("bulk")),
        (3, Some("panel")),
    ]
    .into_iter()
    .enumerate()
    {
        let mut value = original.clone();
        value["schema"] = serde_json::json!(schema);
        if let Some(workload) = workload {
            value["workload"] = serde_json::json!(workload);
        }
        std::fs::write(&request, value.to_string()).unwrap();
        let destination = w.root.join(format!("refusal-{index}"));
        assert!(prepare(&request, &destination).is_err());
        assert!(!destination.exists());
    }
    let request = w.request(&w.plans());
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&request).unwrap()).unwrap();
    value["schema"] = serde_json::json!(2);
    value["workload"] = serde_json::json!("panel");
    std::fs::write(&request, value.to_string()).unwrap();
    assert!(prepare(&request, &w.root.join("bulk-as-panel")).is_err());
}

#[test]
fn large_surface_is_explicitly_frozen_and_rederived_as_presented_pixels() {
    let w = World::new("large-surface");
    let plans = PLAN_IDS
        .iter()
        .enumerate()
        .map(|(i, id)| {
            (
                id.to_string(),
                w.plan(
                    id,
                    &Shape {
                        ipc: i % 3 == 0,
                        sealed: i % 3 == 2,
                        instrumentation: if i < 3 { "cpu-only" } else { "rust-global" },
                        height: 512,
                        probe: 0,
                        ..Shape::bytes()
                    },
                ),
            )
        })
        .collect();
    let request = w.request(&plans);
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&request).unwrap()).unwrap();
    value["schema"] = 3.into();
    value["workload"] = "panel".into();
    value["presentation_height"] = 512.into();
    for (i, (schema, height, workload)) in [
        (1, Some(512), "panel"),
        (2, Some(512), "panel"),
        (3, None, "panel"),
        (3, Some(24), "panel"),
        (3, Some(1080), "panel"),
        (3, Some(512), "bulk"),
    ]
    .iter()
    .enumerate()
    {
        let mut bad = value.clone();
        bad["schema"] = (*schema).into();
        bad["workload"] = (*workload).into();
        bad["presentation_height"] = serde_json::json!(height);
        std::fs::write(&request, bad.to_string()).unwrap();
        assert!(prepare(&request, &w.root.join(format!("bad-large-{i}"))).is_err());
    }
    std::fs::write(&request, value.to_string()).unwrap();
    let frozen = w.root.join("large-frozen");
    let line = prepare(&request, &frozen).unwrap();
    let digest = &line.split("sha256=").nth(1).unwrap()[..64];
    let (_, plans) = verify_descriptor(&frozen, digest).unwrap();
    retain(&frozen, digest, &plans);
    let result = report(&frozen, digest, &supplied).unwrap();
    assert_eq!(result["presentation_height"], 512);
    for arm in result["arms"].as_array().unwrap() {
        assert_eq!(
            arm["cost"]["normalized"]["presentation_bytes"],
            3_932_160_000u64
        );
        assert_eq!(arm["cost"]["normalized"]["transfer_probe_bytes"], 0);
        let timing = &arm["presentation_timing_diagnostic"];
        assert_eq!(timing["offered"], 1000);
        assert_eq!(timing["candidate_to_native_presented"]["count"], 1000);
        for upload in timing["journal_metrics"]["uploads"].as_array().unwrap() {
            assert_eq!(upload["purpose"], "presentation");
            assert_eq!(upload["upload"]["height"], 512);
        }
    }
    // Same byte total but a different shape must not satisfy the frozen surface.
    let arm_dir = frozen.join(ARM_IDS[0]);
    let log = arm_dir.join("run/session.log");
    let original = std::fs::read_to_string(&log).unwrap();
    let changed = original.replacen(
        "\"width\":1920,\"height\":512",
        "\"width\":3840,\"height\":256",
        1,
    );
    assert_ne!(original, changed);
    std::fs::write(&log, &changed).unwrap();
    let record = arm_dir.join("attempt.json");
    let mut attempt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
    attempt["outcome"]["evidence"]["session.log"] = sha256(changed.as_bytes()).into();
    std::fs::write(record, attempt.to_string()).unwrap();
    assert_eq!(
        report(&frozen, digest, &supplied).unwrap_err(),
        "presented upload geometry differs from the frozen pilot"
    );

    // A digest-consistent profile with the old reservation fails before launch.
    let path = &plans[0].request.artifacts["profile"].path;
    let changed = std::fs::read_to_string(path)
        .unwrap()
        .replace("reservation \"top\" 512", "reservation \"top\" 24");
    std::fs::write(path, &changed).unwrap();
    let plan_path = &plans[0].reference.path;
    let mut plan: serde_json::Value =
        serde_json::from_slice(&std::fs::read(plan_path).unwrap()).unwrap();
    plan["request"]["artifacts"]["profile"]["sha256"] = sha256(changed.as_bytes()).into();
    std::fs::write(plan_path, plan.to_string()).unwrap();
    assert_eq!(
        prepare(&request, &w.root.join("bad-reservation")).unwrap_err(),
        "large presented surface requires reservation top 512"
    );
}
