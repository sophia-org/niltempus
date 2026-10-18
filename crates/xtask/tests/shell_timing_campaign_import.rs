//! Campaign import over supplied fixtures. No devices, no real campaign.
//! Provenance values here are supplied verdicts, never authentication.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use xtask::shell_timing_campaign::{
    Arm, AttemptOutcome, Condition, EnvironmentTarget, Manifest, Measurement, Variant,
    Workload as ArmWorkload, claim_attempt,
};
use xtask::shell_timing_campaign_import::*;
use xtask::shell_timing_compare::Workload;
use xtask::shell_timing_environment::{Capture, Expect, Interval, Vrr};
use xtask::shell_timing_native::{Artifact, Source};

const STEP: u64 = 5_000_000;
const WORKLOAD: &str = "captured upload workload differs from the arm";

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// A latency-capable session capture: client journal then native records.
fn latency_session(frames: u64, wire: &str, probe: bool) -> String {
    let mut journal = format!(
        "{{\"schema\":2,\"event\":\"start\",\"frames\":{frames},\"wire\":\"{wire}\",\"probe_bytes\":{}}}\n",
        if probe { 4194304 } else { 0 }
    );
    let mut host = String::new();
    for g in 1..=frames {
        let mut t = g * STEP;
        let upload = |id: u64, generation: u64, w: u32, h: u32, begin: u64| {
            format!(
                "{{\"schema\":2,\"event\":\"upload_offer\",\"upload\":{{\"connection_epoch\":1,\"content_grant_epoch\":2,\"output\":3,\"candidate_generation\":{g},\"resource_id\":{id},\"resource_generation\":{generation},\"width\":{w},\"height\":{h},\"total_bytes\":{},\"begin_usec\":{begin}}}}}\n",
                u64::from(w) * u64::from(h) * 4
            )
        };
        if probe {
            journal += &upload(1, g, 1024, 1024, t);
            t += 20_000;
            journal += &format!(
                "{{\"schema\":2,\"event\":\"upload_accepted\",\"resource_id\":1,\"resource_generation\":{g},\"accepted_usec\":{t}}}\n"
            );
            t += 1;
            journal += &format!(
                "{{\"schema\":2,\"event\":\"probe_released\",\"resource_id\":1,\"resource_generation\":{g},\"released_usec\":{t}}}\n"
            );
        }
        let (id, generation) = ((g - 1) % 2 + 1 + u64::from(probe), (g - 1) / 2 + 1);
        journal += &upload(id, generation, 1920, 24, t);
        t += 100;
        journal += &format!(
            "{{\"schema\":2,\"event\":\"upload_accepted\",\"resource_id\":{id},\"resource_generation\":{generation},\"accepted_usec\":{t}}}\n"
        );
        let demand = t + 1;
        t = demand + 10;
        journal += &format!(
            "{{\"schema\":2,\"event\":\"permit\",\"generation\":{g},\"demand_usec\":{demand},\"received_usec\":{t}}}\n"
        );
        let submitted = t + 1;
        journal += &format!(
            "{{\"schema\":2,\"event\":\"offer\",\"connection_epoch\":1,\"content_grant_epoch\":2,\"output\":3,\"candidate_generation\":{g},\"submitted_monotonic_usec\":{submitted}}}\n"
        );
        let completed = submitted + 1_000;
        journal += &format!(
            "{{\"schema\":2,\"event\":\"terminal\",\"connection_epoch\":1,\"content_grant_epoch\":2,\"output\":3,\"candidate_generation\":{g},\"outcome\":\"presented\",\"kind\":2,\"reason\":0,\"observed_monotonic_usec\":{}}}\n",
            completed + 1
        );
        host += &native(g, completed);
    }
    journal += &format!(
        "{{\"schema\":2,\"event\":\"end\",\"offered\":{frames},\"completed\":{frames}}}\n"
    );
    journal + &host
}

fn native(g: u64, completed: u64) -> String {
    format!(
        "sophia_shell_native_binding schema=1 connection_epoch=1 content_grant_epoch=2 output=3 candidate_generation={g} native_owner=5 native_frame={g} head=7 target_generation=8 heads=1 mode_refresh_millihz=120000\nsophia_shell_native_completion schema=1 output=3 native_owner=5 native_frame={g} heads=1 monotonic_usec={completed} timestamp_source=kernel missing_kernel_timestamp=0\n"
    )
}

fn counter(process: &str, stage: &str, sequence: u64, time: u64, count: u64) -> String {
    let pid = if process == "session" { 10 } else { 20 };
    format!(
        "sophia_comparator_counters schema=1 process={process} pid={pid} stage={stage} sequence={sequence} before_usec={time} after_usec={} cpu_ns={} allocations={count} reallocations=0 deallocations=0 bytes_allocated={}\n",
        time + 1,
        count * 100,
        count * 16
    )
}

/// A two-frame rust-global cost capture with counters and kernel completions.
fn cost_session() -> String {
    let mut text = String::new();
    text += &serde_json::json!({"schema":2,"event":"start","frames":2,"wire":"files"}).to_string();
    text += "\n";
    for (index, (begin, accepted, submitted, terminal)) in
        [(27u64, 28u64, 35u64, 50u64), (60, 61, 62, 64)]
            .into_iter()
            .enumerate()
    {
        let g = index + 1;
        for row in [
            serde_json::json!({"schema":2,"event":"upload_offer","upload":{"connection_epoch":1,"content_grant_epoch":2,"output":3,"candidate_generation":g,"resource_id":g,"resource_generation":1,"width":1920,"height":24,"total_bytes":184320,"begin_usec":begin}}),
            serde_json::json!({"schema":2,"event":"upload_accepted","resource_id":g,"resource_generation":1,"accepted_usec":accepted}),
            serde_json::json!({"schema":2,"event":"permit","generation":g,"demand_usec":accepted,"received_usec":accepted}),
            serde_json::json!({"schema":2,"event":"offer","connection_epoch":1,"content_grant_epoch":2,"output":3,"candidate_generation":g,"submitted_monotonic_usec":submitted}),
            serde_json::json!({"schema":2,"event":"terminal","connection_epoch":1,"content_grant_epoch":2,"output":3,"candidate_generation":g,"outcome":"presented","kind":2,"reason":0,"observed_monotonic_usec":terminal}),
        ] {
            text += &row.to_string();
            text += "\n";
        }
    }
    text += &serde_json::json!({"schema":2,"event":"end","offered":2,"completed":2}).to_string();
    text += "\n";
    for i in 0..7u64 {
        let stage = match i {
            0 => "start",
            6 => "end",
            _ => "sample",
        };
        text += &counter("session", stage, i, 10 + i * 10, i * 10);
    }
    text += &counter("peer", "frame_start", 1, 25, 5);
    text += &counter("peer", "frame_end", 1, 55, 15);
    text += &counter("peer", "frame_start", 2, 58, 16);
    text += &counter("peer", "frame_end", 2, 65, 20);
    text += &native(1, 45);
    text += &native(2, 63);
    text
}

fn arm(
    id: &str,
    condition: Condition,
    pair: u8,
    measurement: Measurement,
    variant: Variant,
) -> Arm {
    Arm {
        id: id.into(),
        condition,
        pair,
        workload: ArmWorkload::Panel,
        measurement,
        variant,
        plan_sha256: "0".repeat(64),
        peer_config_sha256: "0".repeat(64),
        frames: if measurement == Measurement::Latency {
            6
        } else {
            2
        },
        warmup: if measurement == Measurement::Latency {
            2
        } else {
            0
        },
        runtime_ms: 1_000,
    }
}

fn manifest(arms: Vec<Arm>) -> Manifest {
    Manifest {
        schema: 2,
        id: "campaign".into(),
        connector: "DP-2".into(),
        width: 2560,
        height: 1440,
        mode_millihz: 120000,
        vrr: false,
        environment: EnvironmentTarget {
            card: "card1".into(),
            output: 3,
            scale_milli: 1000,
        },
        load_workers: 16,
        quiet_window: "test".into(),
        arms,
    }
}

fn run_files(session: &str) -> RunFiles {
    RunFiles {
        plan: b"plan".to_vec(),
        result: br#"{"schema":1,"status":"captured_only","error":null}"#.to_vec(),
        session_log: session.as_bytes().to_vec(),
        environment: b"{}".to_vec(),
        environment_receipts: b"{}".to_vec(),
        load: None,
    }
}

fn expect() -> Expect {
    Expect {
        output: 3,
        card: "card1".into(),
        connector: "DP-2".into(),
        width: 2560,
        height: 1440,
        refresh_millihz: 120000,
        scale_milli: 1000,
        vrr: Vrr::Off,
        host_pid: 4100,
        peer_pid: 4200,
        capture: Interval {
            start_usec: 1,
            end_usec: 100_000_000,
        },
        measurement: Interval {
            start_usec: 2,
            end_usec: 90_000_000,
        },
    }
}

fn domain() -> MonotonicDomain {
    MonotonicDomain {
        reference: 0xEFFFFFFA,
        host: 0xEFFFFFFA,
        peer: 0xEFFFFFFA,
        capturer: 0xEFFFFFFA,
        monotonic_offset_seconds: 0,
        monotonic_offset_nanoseconds: 0,
    }
}

/// Supplied verdict: claims authentication it does not perform.
fn verified() -> VerifiedEnvironment {
    VerifiedEnvironment {
        capture: Capture::default(),
        expect: expect(),
        host_starttime: 1,
        peer_starttime: 2,
        clock: domain(),
    }
}

/// A run directory removed when the test ends, pass or fail.
struct Scratch(PathBuf);

impl std::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(name: &str) -> Scratch {
    let directory =
        std::env::temp_dir().join(format!("campaign-import-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(directory.join(RUN_DIRECTORY)).unwrap();
    Scratch(directory)
}

/// Write the named run files and return their digests.
fn retain(directory: &Path, files: &[(&str, &[u8])]) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|(name, bytes)| {
            std::fs::write(directory.join(RUN_DIRECTORY).join(name), bytes).unwrap();
            (name.to_string(), sha256(bytes))
        })
        .collect()
}

fn base_files() -> Vec<(&'static str, &'static [u8])> {
    vec![
        ("plan.json", b"plan"),
        (
            "result.json",
            br#"{"schema":1,"status":"captured_only","error":null}"#,
        ),
        ("session.log", b"session\n"),
        ("environment.json", b"{}"),
        ("environment-receipts.json", b"[]"),
    ]
}

#[test]
fn evidence_names_are_exact_and_load_only_on_load_arms() {
    let idle = required_evidence(Condition::Idle);
    assert_eq!(idle.len(), 5);
    assert!(!idle.contains(LOAD_EVIDENCE));
    assert!(required_evidence(Condition::Load).contains(LOAD_EVIDENCE));
}

#[test]
fn run_files_are_read_once_against_their_recorded_digests() {
    let idle = arm("i", Condition::Idle, 1, Measurement::Latency, Variant::A);
    let directory = scratch("digests");
    let evidence = retain(&directory, &base_files());
    let run = read_run(&directory, &idle, &evidence).unwrap();
    assert_eq!(run.session_log, b"session\n");
    assert!(run.load.is_none());

    // A changed file, a missing name, an extra name and a missing file.
    std::fs::write(
        directory.join(RUN_DIRECTORY).join("session.log"),
        b"other\n",
    )
    .unwrap();
    assert!(
        read_run(&directory, &idle, &evidence)
            .err()
            .unwrap()
            .contains("session.log")
    );
    let evidence = retain(&directory, &base_files());
    let mut short = evidence.clone();
    short.remove("environment-receipts.json");
    assert!(read_run(&directory, &idle, &short).is_err());
    let mut extra = evidence.clone();
    extra.insert(LOAD_EVIDENCE.into(), sha256(b"load"));
    assert!(read_run(&directory, &idle, &extra).is_err());
    std::fs::remove_file(directory.join(RUN_DIRECTORY).join("environment.json")).unwrap();
    assert!(read_run(&directory, &idle, &evidence).is_err());

    // An idle arm with an unnamed load record on disk is refused.
    let evidence = retain(&directory, &base_files());
    std::fs::write(directory.join(RUN_DIRECTORY).join(LOAD_EVIDENCE), b"{}").unwrap();
    assert_eq!(
        read_run(&directory, &idle, &evidence).err().as_deref(),
        Some("idle arm retains a load record")
    );

    // A load arm requires and reads its load record.
    let load = arm("l", Condition::Load, 1, Measurement::Latency, Variant::A);
    let directory = scratch("load");
    let mut files = base_files();
    assert!(read_run(&directory, &load, &retain(&directory, &files)).is_err());
    files.push((LOAD_EVIDENCE, b"{\"workers\":16}"));
    let run = read_run(&directory, &load, &retain(&directory, &files)).unwrap();
    assert_eq!(run.load.as_deref(), Some(&b"{\"workers\":16}"[..]));

    // A link is not a retained file.
    let directory = scratch("link");
    let evidence = retain(&directory, &base_files());
    let target = directory.join(RUN_DIRECTORY).join("session.log");
    let moved = directory.join("elsewhere.log");
    std::fs::rename(&target, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &target).unwrap();
    assert!(
        read_run(&directory, &idle, &evidence)
            .err()
            .unwrap()
            .contains("regular file")
    );
}

#[test]
fn the_run_must_use_the_frozen_plan_and_finish_capturing() {
    let run = run_files("");
    assert!(check_run_record(b"plan", &run).is_ok());
    assert_eq!(
        check_run_record(b"other plan", &run).err().as_deref(),
        Some("run plan differs from the frozen plan")
    );
    for result in [
        r#"{"schema":1,"status":"failed","error":"x"}"#,
        r#"{"schema":1,"status":"captured_only","error":"x"}"#,
        r#"{"schema":2,"status":"captured_only","error":null}"#,
        r#"{"schema":1,"status":"captured_only","error":null,"extra":1}"#,
    ] {
        let mut run = run_files("");
        run.result = result.as_bytes().to_vec();
        assert!(check_run_record(b"plan", &run).is_err(), "{result}");
    }
}

#[test]
fn latency_and_cost_arms_are_rebuilt_from_the_retained_capture() {
    let m = manifest(Vec::new());
    let latency = arm("lat", Condition::Idle, 1, Measurement::Latency, Variant::B);
    let rebuilt = derive_arm(
        &m,
        &latency,
        &run_files(&latency_session(6, "files", false)),
    )
    .unwrap();
    let DerivedArm::Latency(arm_latency) = rebuilt.arm else {
        panic!("latency arm expected");
    };
    assert_eq!(arm_latency.label, "lat");
    assert_eq!(arm_latency.workload, Workload::Panel);
    assert_eq!(arm_latency.latency.window.warmup, 2);
    assert_eq!(arm_latency.latency.window.measured, 4);
    // The span runs from the first upload to the last completion or terminal.
    assert_eq!(rebuilt.run.start_usec, STEP);
    assert_eq!(
        rebuilt.run.end_usec,
        6 * STEP + 100 + 1 + 10 + 1 + 1_000 + 1
    );

    let cost = arm(
        "cost",
        Condition::Idle,
        1,
        Measurement::RustGlobal,
        Variant::B,
    );
    let rebuilt = derive_arm(&m, &cost, &run_files(&cost_session())).unwrap();
    let DerivedArm::Cost(arm_cost) = rebuilt.arm else {
        panic!("cost arm expected");
    };
    assert_eq!(arm_cost.report.instrumentation, "rust-global");
    assert_eq!((rebuilt.run.start_usec, rebuilt.run.end_usec), (27, 64));
}

#[test]
fn rebuilt_arms_must_match_their_frozen_identity() {
    let m = manifest(Vec::new());
    let session = latency_session(6, "files", false);
    // Wire, source and probe come from the variant and workload.
    let ipc = arm("a", Condition::Idle, 1, Measurement::Latency, Variant::A);
    assert_eq!(
        derive_arm(&m, &ipc, &run_files(&session)).err().as_deref(),
        Some(WORKLOAD)
    );
    let sealed = arm("c", Condition::Idle, 1, Measurement::Latency, Variant::C);
    assert_eq!(
        derive_arm(&m, &sealed, &run_files(&session))
            .err()
            .as_deref(),
        Some(WORKLOAD)
    );
    let mut bulk = arm("b", Condition::Idle, 1, Measurement::Latency, Variant::B);
    bulk.workload = ArmWorkload::Bulk;
    assert_eq!(
        derive_arm(&m, &bulk, &run_files(&session)).err().as_deref(),
        Some(WORKLOAD)
    );
    let bulk_session = latency_session(6, "files", true);
    assert!(derive_arm(&m, &bulk, &run_files(&bulk_session)).is_ok());
    // Frames, warmup and instrumentation.
    let mut frames = arm("b", Condition::Idle, 1, Measurement::Latency, Variant::B);
    frames.frames = 5;
    assert_eq!(
        derive_arm(&m, &frames, &run_files(&session))
            .err()
            .as_deref(),
        Some("wrong start count or wire")
    );
    let mut warm = arm("b", Condition::Idle, 1, Measurement::Latency, Variant::B);
    warm.warmup = 6;
    assert_eq!(
        derive_arm(&m, &warm, &run_files(&session)).err().as_deref(),
        Some("warmup must leave measured frames")
    );
    let cpu = arm("b", Condition::Idle, 1, Measurement::CpuOnly, Variant::B);
    assert_eq!(
        derive_arm(&m, &cpu, &run_files(&cost_session()))
            .err()
            .as_deref(),
        Some("counter mode differs from the arm")
    );
    let mut counters_missing = arm("b", Condition::Idle, 1, Measurement::RustGlobal, Variant::B);
    counters_missing.frames = 6;
    assert_eq!(
        derive_arm(&m, &counters_missing, &run_files(&session))
            .err()
            .as_deref(),
        Some("incomplete host or peer counter lifecycle")
    );
    // Progress diagnostics and fallback completion clocks are refused.
    let progress = format!("{session}sophia_comparator_progress schema=1\n");
    let latency = arm("b", Condition::Idle, 1, Measurement::Latency, Variant::B);
    assert_eq!(
        derive_arm(&m, &latency, &run_files(&progress))
            .err()
            .as_deref(),
        Some("progress diagnostics cannot supply cost or latency arms")
    );
    let fallback = session.replacen(
        "timestamp_source=kernel missing_kernel_timestamp=0",
        "timestamp_source=observation_fallback missing_kernel_timestamp=1",
        1,
    );
    assert_eq!(
        derive_arm(&m, &latency, &run_files(&fallback))
            .err()
            .as_deref(),
        Some("native completion lacks a kernel timestamp")
    );
    let missing_kernel = session.replacen(
        "missing_kernel_timestamp=0",
        "missing_kernel_timestamp=1",
        1,
    );
    assert_eq!(
        derive_arm(&m, &latency, &run_files(&missing_kernel))
            .err()
            .as_deref(),
        Some("native completion lacks a kernel timestamp")
    );
}

#[test]
fn the_provider_expectation_must_be_the_frozen_target() {
    let m = manifest(Vec::new());
    assert!(check_expect(&m, &expect()).is_ok());
    let mutations: [fn(&mut Expect); 8] = [
        |e| e.connector = "HDMI-A-1".into(),
        |e| e.width = 1920,
        |e| e.height = 1080,
        |e| e.refresh_millihz = 60000,
        |e| e.vrr = Vrr::On,
        |e| e.card = "card0".into(),
        |e| e.output = 4,
        |e| e.scale_milli = 2000,
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut e = expect();
        mutate(&mut e);
        assert_eq!(
            check_expect(&m, &e).err().as_deref(),
            Some("environment expectation differs from the frozen campaign"),
            "mutation {index}"
        );
    }
}

#[test]
fn qualified_load_must_cover_the_measurement_with_the_declared_workers() {
    let m = manifest(Vec::new());
    let e = expect();
    let load = || VerifiedLoad {
        workers: 16,
        interval: Interval {
            start_usec: 1,
            end_usec: 95_000_000,
        },
        measurement: e.measurement,
        worker_cpu_ticks: vec![1; 16],
    };
    assert!(check_load(&m, &e, &load()).is_ok());
    let mut workers = load();
    workers.workers = 15;
    assert!(check_load(&m, &e, &workers).is_err());
    let mut other = load();
    other.measurement.end_usec -= 1;
    assert!(check_load(&m, &e, &other).is_err());
    let mut late = load();
    late.interval.start_usec = 3;
    assert!(check_load(&m, &e, &late).is_err());
    let mut early = load();
    early.interval.end_usec = 89_999_999;
    assert!(check_load(&m, &e, &early).is_err());
}

#[test]
fn the_run_span_is_compared_only_in_a_proven_unshifted_clock() {
    let run = Interval {
        start_usec: 10,
        end_usec: 1_000,
    };
    assert!(check_measured_interval(&verified(), run).is_ok());
    let mutations: [fn(&mut MonotonicDomain); 7] = [
        |d| d.reference = 0,
        |d| d.host = 8,
        |d| d.peer = 8,
        |d| d.capturer = 8,
        // A shared namespace with a shifted clock is not the kernel's clock.
        |d| d.monotonic_offset_seconds = 1,
        |d| d.monotonic_offset_nanoseconds = -1,
        |d| {
            d.reference = 0;
            d.host = 0;
            d.peer = 0;
            d.capturer = 0;
        },
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut v = verified();
        mutate(&mut v.clock);
        assert_eq!(
            check_measured_interval(&v, run).err().as_deref(),
            Some("measured interval clock domain is unproven"),
            "mutation {index}"
        );
    }
    for outside in [
        Interval {
            start_usec: 1,
            end_usec: 1_000,
        },
        Interval {
            start_usec: 10,
            end_usec: 90_000_001,
        },
        Interval {
            start_usec: 1_000,
            end_usec: 10,
        },
    ] {
        assert_eq!(
            check_measured_interval(&verified(), outside)
                .err()
                .as_deref(),
            Some("run timestamps fall outside the declared measurement")
        );
    }
}

#[test]
fn pairs_follow_the_manifest_and_every_arm_must_be_rebuilt() {
    let m = manifest(vec![
        arm("i1", Condition::Idle, 1, Measurement::Latency, Variant::B),
        arm("l1", Condition::Load, 1, Measurement::Latency, Variant::B),
        arm(
            "i2",
            Condition::Idle,
            2,
            Measurement::RustGlobal,
            Variant::B,
        ),
        arm("i3", Condition::Idle, 3, Measurement::Latency, Variant::B),
    ]);
    let rebuild = |id: &str| {
        let a = m.arms.iter().find(|a| a.id == id).unwrap();
        let session = if a.measurement == Measurement::Latency {
            latency_session(6, "files", false)
        } else {
            cost_session()
        };
        derive_arm(&m, a, &run_files(&session)).unwrap().arm
    };
    let derived = || {
        ["i1", "i2", "i3"]
            .into_iter()
            .map(|id| (id.to_string(), rebuild(id)))
            .collect::<BTreeMap<_, _>>()
    };
    let pairs = pair_inputs(&m, Condition::Idle, derived()).unwrap();
    let shape: Vec<_> = pairs
        .iter()
        .map(|p| (p.index, p.order.clone(), p.latency.len(), p.cost.len()))
        .collect();
    assert_eq!(
        shape,
        [
            (1, vec!["i1".to_string()], 1, 0),
            (2, vec!["i2".to_string()], 0, 1),
            (3, vec!["i3".to_string()], 1, 0),
        ]
    );
    let mut missing = derived();
    missing.remove("i2");
    assert!(pair_inputs(&m, Condition::Idle, missing).is_err());
    let mut foreign = derived();
    foreign.insert("l1".into(), rebuild("l1"));
    assert!(pair_inputs(&m, Condition::Idle, foreign).is_err());
}

#[test]
fn the_captured_output_must_be_the_frozen_output() {
    let mut m = manifest(Vec::new());
    m.environment.output = 4;
    let latency = arm("lat", Condition::Idle, 1, Measurement::Latency, Variant::B);
    assert_eq!(
        derive_arm(
            &m,
            &latency,
            &run_files(&latency_session(6, "files", false))
        )
        .err()
        .as_deref(),
        Some("captured output differs from the frozen output")
    );
}

#[test]
fn a_fifo_in_place_of_a_run_file_is_refused_without_blocking() {
    let idle = arm("i", Condition::Idle, 1, Measurement::Latency, Variant::A);
    let directory = scratch("fifo");
    let evidence = retain(&directory, &base_files());
    let target = directory.join(RUN_DIRECTORY).join("session.log");
    std::fs::remove_file(&target).unwrap();
    let status = std::process::Command::new("mkfifo")
        .arg(&target)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(
        read_run(&directory, &idle, &evidence)
            .err()
            .unwrap()
            .contains("regular file")
    );
}

// End-to-end: a frozen 108-arm campaign whose prepared plans pass the same
// verification as a real one. The sources are a throwaway repository whose
// commit is signed with a throwaway SSH key, verified through repo-local
// allowed signers, so no user key or agent is touched. Artifacts are hashed,
// never executed. Captures are generated text, and the environment provider
// is a supplied verdict.

const E2E_OUTPUT: u64 = 3;

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

/// A clean repository at a commit `git verify-commit` accepts.
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

fn same_source(source: &Source) -> Source {
    Source {
        path: source.path.clone(),
        commit: source.commit.clone(),
    }
}

fn same_artifact(artifact: &Artifact) -> Artifact {
    Artifact {
        path: artifact.path.clone(),
        sha256: artifact.sha256.clone(),
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

fn identity(variant: Variant) -> (&'static str, &'static str) {
    match variant {
        Variant::A => ("ipc", "bytes"),
        Variant::B => ("files", "bytes"),
        Variant::C => ("files", "sealed"),
    }
}

/// A prepared plan for one arm, valid under `verify_prepared` and
/// `validate_arm_plan`.
fn prepared_plan(
    artifacts: &Path,
    shared: &BTreeMap<String, Artifact>,
    source: &Source,
    arm: &Arm,
) -> (Vec<u8>, String) {
    use xtask::shell_timing::ImageSource;
    let (wire, image) = identity(arm.variant);
    let probe = if arm.workload == ArmWorkload::Bulk {
        4194304
    } else {
        0
    };
    let mut config = serde_json::json!({
        "wire": wire, "source": image, "output": E2E_OUTPUT, "frames": arm.frames,
        "height": 24, "hold_msec": 60000, "probe_bytes": probe,
    });
    if image == "sealed" {
        config["image"] = serde_json::json!({
            "max_resource_bytes": 4194304, "max_width": 4096, "max_height": 4096,
            "max_admitted_outputs": 8, "transfer_slots": 4, "transfer_timeout_ms": 2000,
        });
    }
    let shape = format!("{wire}-{image}-{}-{probe}", arm.frames);
    let peer_config = put(
        artifacts,
        &format!("peer-config-{shape}.json"),
        config.to_string().as_bytes(),
        0o600,
    );
    let transport = if wire == "files" {
        "9p2000.L"
    } else {
        "comparison-ipc"
    };
    let profile = format!(
        "schema 1\nsession {{\n    shell-component \"timing\" \"bar\" {{ executable \"{}\"; config \"{}\"; transport \"{transport}\"; gpu \"denied\"; }}\n    startup\n}}\n",
        shared["peer"].path.display(),
        peer_config.path.display()
    );
    let profile = put(
        artifacts,
        &format!("profile-{shape}.kdl"),
        profile.as_bytes(),
        0o600,
    );
    let mut all: BTreeMap<_, _> = shared
        .iter()
        .map(|(key, artifact)| (key.clone(), same_artifact(artifact)))
        .collect();
    all.insert("peer-config".into(), peer_config);
    all.insert("profile".into(), profile);
    let request = xtask::shell_timing_native::Request {
        sophia_source: same_source(source),
        integration_source: same_source(source),
        artifacts: all,
        tty: "/dev/tty4".into(),
        display: 94,
        input_seat: "seat0".into(),
        runtime_ms: 60_000,
        wire: wire.into(),
        source: if image == "sealed" {
            ImageSource::Sealed
        } else {
            ImageSource::Bytes
        },
        output: E2E_OUTPUT,
        environment_readback: true,
        frames: arm.frames,
        mode_millihz: 120_000,
        instrumentation: match arm.measurement {
            Measurement::Latency => "none",
            Measurement::CpuOnly => "cpu-only",
            Measurement::RustGlobal => "rust-global",
        }
        .into(),
    };
    let argv = xtask::shell_timing_native::validate(&request).unwrap();
    let plan =
        serde_json::to_vec(&serde_json::json!({"schema":1,"request":request,"argv":argv})).unwrap();
    let peer_config_sha256 = request.artifacts["peer-config"].sha256.clone();
    (plan, peer_config_sha256)
}

/// A cost capture for `frames` frames: client journal, one counter frame
/// around each candidate, Session snapshots before and after, and kernel
/// completions.
fn cost_capture(frames: u64, wire: &str, sealed: bool, probe: bool, mode: &str) -> String {
    let schema = if mode == "cpu-only" {
        "schema=2 mode=cpu-only"
    } else {
        "schema=1"
    };
    let allocations = |count: u64| if mode == "cpu-only" { 0 } else { count };
    let row = |process: &str, stage: &str, sequence: u64, time: u64, count: u64| {
        let pid = if process == "session" { 10 } else { 20 };
        format!(
            "sophia_comparator_counters {schema} process={process} pid={pid} stage={stage} sequence={sequence} before_usec={time} after_usec={} cpu_ns={} allocations={} reallocations=0 deallocations=0 bytes_allocated={}\n",
            time + 1,
            count * 100,
            allocations(count),
            allocations(count) * 16
        )
    };
    let source = if sealed { r#","source":"sealed""# } else { "" };
    let mut journal = format!(
        "{{\"schema\":2,\"event\":\"start\",\"frames\":{frames},\"wire\":\"{wire}\"{source},\"probe_bytes\":{}}}\n",
        if probe { 4194304 } else { 0 }
    );
    let mut counters = row("session", "start", 0, 1_000, 0);
    let mut host = String::new();
    let mut t = 10_000;
    for g in 1..=frames {
        counters += &row("peer", "frame_start", g, t, 2 * g);
        t += 2;
        let upload = |id: u64, generation: u64, side: u32, t: u64| {
            format!(
                "{{\"schema\":2,\"event\":\"upload_offer\",\"upload\":{{\"connection_epoch\":1,\"content_grant_epoch\":2,\"output\":3,\"candidate_generation\":{g},\"resource_id\":{id},\"resource_generation\":{generation},\"width\":{},\"height\":{side},\"total_bytes\":{},\"begin_usec\":{t}}}}}\n",
                if side == 24 { 1920 } else { side },
                u64::from(if side == 24 { 1920 } else { side }) * u64::from(side) * 4
            ) + &format!(
                "{{\"schema\":2,\"event\":\"upload_accepted\",\"resource_id\":{id},\"resource_generation\":{generation},\"accepted_usec\":{}}}\n",
                t + 1
            )
        };
        if probe {
            journal += &upload(1, g, 1024, t);
            journal += &format!(
                "{{\"schema\":2,\"event\":\"probe_released\",\"resource_id\":1,\"resource_generation\":{g},\"released_usec\":{}}}\n",
                t + 2
            );
            t += 3;
        }
        journal += &upload((g - 1) % 2 + 1 + u64::from(probe), (g - 1) / 2 + 1, 24, t);
        t += 2;
        journal += &format!(
            "{{\"schema\":2,\"event\":\"permit\",\"generation\":{g},\"demand_usec\":{t},\"received_usec\":{}}}\n",
            t + 1
        );
        journal += &format!(
            "{{\"schema\":2,\"event\":\"offer\",\"connection_epoch\":1,\"content_grant_epoch\":2,\"output\":3,\"candidate_generation\":{g},\"submitted_monotonic_usec\":{}}}\n",
            t + 2
        );
        host += &native(g, t + 3);
        journal += &format!(
            "{{\"schema\":2,\"event\":\"terminal\",\"connection_epoch\":1,\"content_grant_epoch\":2,\"output\":3,\"candidate_generation\":{g},\"outcome\":\"presented\",\"kind\":2,\"reason\":0,\"observed_monotonic_usec\":{}}}\n",
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

fn arm_capture(arm: &Arm) -> String {
    let (wire, image) = identity(arm.variant);
    let probe = arm.workload == ArmWorkload::Bulk;
    match arm.measurement {
        Measurement::Latency => {
            let text = latency_session(arm.frames as u64, wire, probe);
            if image == "sealed" {
                text.replacen(
                    r#""wire":"files""#,
                    r#""wire":"files","source":"sealed""#,
                    1,
                )
            } else {
                text
            }
        }
        Measurement::CpuOnly => cost_capture(
            arm.frames as u64,
            wire,
            image == "sealed",
            probe,
            "cpu-only",
        ),
        Measurement::RustGlobal => cost_capture(
            arm.frames as u64,
            wire,
            image == "sealed",
            probe,
            "rust-global",
        ),
    }
}

/// The idle condition's 54 arms in frozen order, each capture finished; the
/// arm at `failed` records a launch failure instead.
struct Campaign {
    root: PathBuf,
    directory: PathBuf,
    digest: String,
}

impl Campaign {
    fn new(name: &str, failed: Option<usize>) -> Self {
        let root =
            std::env::temp_dir().join(format!("campaign-import-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let source = signed_source(&root);
        let artifacts = root.join("artifacts");
        std::fs::create_dir(&artifacts).unwrap();
        let mut shared = BTreeMap::new();
        for key in [
            "sophia",
            "peer",
            "wm",
            "factotum",
            "pam-helper",
            "preflight",
            "runner",
        ] {
            let bytes = format!("{key}: hashed only, never executed");
            shared.insert(
                key.to_string(),
                put(&artifacts, key, bytes.as_bytes(), 0o700),
            );
        }
        shared.insert("core".into(), put(&artifacts, "core", b"schema 2\n", 0o600));
        let mut arms = Vec::new();
        for condition in [Condition::Idle, Condition::Load] {
            for (pair, order) in [
                (1, [Variant::C, Variant::B, Variant::A]),
                (2, [Variant::A, Variant::B, Variant::C]),
                (3, [Variant::B, Variant::C, Variant::A]),
            ] {
                for workload in [ArmWorkload::Panel, ArmWorkload::Bulk] {
                    for measurement in [
                        Measurement::Latency,
                        Measurement::CpuOnly,
                        Measurement::RustGlobal,
                    ] {
                        for variant in order {
                            let latency = measurement == Measurement::Latency;
                            arms.push(Arm {
                                id: format!("arm-{}", arms.len()),
                                condition,
                                pair,
                                workload,
                                measurement,
                                variant,
                                plan_sha256: String::new(),
                                peer_config_sha256: String::new(),
                                frames: if latency { 1100 } else { 1000 },
                                warmup: if latency { 100 } else { 0 },
                                runtime_ms: 60_000,
                            });
                        }
                    }
                }
            }
        }
        let mut plans = Vec::new();
        for arm in &mut arms {
            let (plan, peer_config_sha256) = prepared_plan(&artifacts, &shared, &source, arm);
            arm.plan_sha256 = sha256(&plan);
            arm.peer_config_sha256 = peer_config_sha256;
            plans.push(plan);
        }
        let mut m = manifest(arms);
        m.id = "fixture".into();
        m.connector = "DP-test".into();
        (m.width, m.height) = (1920, 1080);
        m.environment = EnvironmentTarget {
            card: "card0".into(),
            output: E2E_OUTPUT,
            scale_milli: 1000,
        };
        m.quiet_window = "Synthetic campaign; never launched".into();
        let digest = m.digest().unwrap();
        let directory = root.join(&m.id);
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec(&m).unwrap(),
        )
        .unwrap();
        for (arm, plan) in m.arms.iter().zip(&plans) {
            std::fs::create_dir(directory.join(&arm.id)).unwrap();
            std::fs::write(directory.join(&arm.id).join("plan.json"), plan).unwrap();
        }
        std::fs::write(directory.join("READY"), &digest).unwrap();
        for (index, (arm, plan)) in m.arms.iter().zip(&plans).enumerate() {
            if arm.condition != Condition::Idle {
                break;
            }
            let attempt = claim_attempt(&directory, &digest, &arm.id).unwrap();
            if failed == Some(index) {
                attempt
                    .finish(AttemptOutcome::Failed {
                        error: "fixture launch failure".into(),
                    })
                    .unwrap();
                continue;
            }
            let run = directory.join(&arm.id);
            std::fs::create_dir(run.join(RUN_DIRECTORY)).unwrap();
            let capture = arm_capture(arm);
            let evidence = retain(
                &run,
                &[
                    ("plan.json", plan),
                    (
                        "result.json",
                        br#"{"schema":1,"status":"captured_only","error":null}"#,
                    ),
                    ("session.log", capture.as_bytes()),
                    ("environment.json", b"{}"),
                    ("environment-receipts.json", b"[]"),
                ],
            );
            attempt
                .finish(AttemptOutcome::Captured { evidence })
                .unwrap();
        }
        Self {
            root,
            directory,
            digest,
        }
    }

    fn import(&self) -> Result<ImportedCondition, String> {
        import_condition(&self.directory, &self.digest, Condition::Idle, &Supplied)
    }
}

impl Drop for Campaign {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

/// Supplied verdict for the e2e campaign: it reads nothing and authenticates
/// nothing. The capture copies the M5 fixture shape for card0, output 3.
struct Supplied;

impl ArmProvenance for Supplied {
    fn environment(&self, _: &ArmFiles<'_>) -> Result<VerifiedEnvironment, String> {
        Ok(VerifiedEnvironment {
            capture: e2e_capture(),
            expect: Expect {
                output: E2E_OUTPUT,
                card: "card0".into(),
                connector: "DP-test".into(),
                width: 1920,
                height: 1080,
                refresh_millihz: 120000,
                scale_milli: 1000,
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
            clock: domain(),
        })
    }

    fn load(&self, _: &ArmFiles<'_>, _: &VerifiedEnvironment) -> Result<VerifiedLoad, String> {
        Err("the idle fixture supplies no load".into())
    }
}

fn e2e_capture() -> Capture {
    use xtask::shell_timing_environment::{SourceKind, SourceText};
    let stat = |pid: u32, comm: &str| {
        let mut fields: Vec<String> = (3..=52).map(|_| "0".to_string()).collect();
        fields[0] = "S".into();
        format!("{pid} ({comm}) {}\n", fields.join(" "))
    };
    let status =
        |pid: u32| format!("Name:\tx\nTgid:\t{pid}\nPid:\t{pid}\nCpus_allowed_list:\t0-3\n");
    let log = "sophia_shell_environment_output schema=1 pid=4100 output=3 card=card0 connector=DP-test width=1920 height=1080 refresh_millihz=120000 timing_refresh_millihz=120000 scale_milli=1000 vrr=off monotonic_usec=1500\nsophia_shell_environment_renderer schema=1 pid=4100 output=3 card=card0 driver=amdgpu vendor=Mesa renderer=fixture version=4.6 monotonic_usec=1600\n";
    let mut sources = vec![
        (
            SourceKind::Osrelease,
            "/proc/sys/kernel/osrelease",
            "6.18.55_1\n".to_string(),
        ),
        (
            SourceKind::ProcVersion,
            "/proc/version",
            "Linux version 6.18.55_1 (fixture) #1 SMP\n".to_string(),
        ),
        (
            SourceKind::CpuOnline,
            "/sys/devices/system/cpu/online",
            "0-3\n".to_string(),
        ),
        (
            SourceKind::ProcStat { pid: 4100 },
            "/proc/4100/stat",
            stat(4100, "sophia"),
        ),
        (
            SourceKind::ProcStatus { pid: 4100 },
            "/proc/4100/status",
            status(4100),
        ),
        (
            SourceKind::ProcStat { pid: 4200 },
            "/proc/4200/stat",
            stat(4200, "peer"),
        ),
        (
            SourceKind::ProcStatus { pid: 4200 },
            "/proc/4200/status",
            status(4200),
        ),
        (
            SourceKind::DrmDriver {
                card: "card0".into(),
            },
            "/sys/class/drm/card0/device/driver",
            "amdgpu\n".to_string(),
        ),
        (SourceKind::HostLog, "host.log", log.to_string()),
    ];
    let governors: Vec<_> = (0..4)
        .map(|cpu| {
            (
                SourceKind::Governor { cpu },
                format!("/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor"),
            )
        })
        .collect();
    let mut texts: Vec<SourceText> = sources
        .drain(..)
        .map(|(kind, origin, text)| SourceText {
            kind,
            origin: origin.into(),
            captured_usec: 1_200,
            text,
        })
        .collect();
    texts.extend(governors.into_iter().map(|(kind, origin)| SourceText {
        kind,
        origin,
        captured_usec: 1_200,
        text: "performance\n".into(),
    }));
    Capture { sources: texts }
}

#[test]
fn a_finished_idle_condition_imports_and_any_bad_arm_refuses_it() {
    let campaign = Campaign::new("complete", None);
    let imported = campaign.import().unwrap();
    assert_eq!(imported.condition, Condition::Idle);
    assert_eq!(imported.arms.len(), 54);
    let shape: Vec<_> = imported
        .pairs
        .iter()
        .map(|p| (p.index, p.order.len(), p.latency.len(), p.cost.len()))
        .collect();
    assert_eq!(shape, [(1, 18, 6, 12), (2, 18, 6, 12), (3, 18, 6, 12)]);
    for arm in &imported.arms {
        assert_eq!(arm.evidence.len(), 5);
        assert!(arm.load.is_none());
    }

    // One retained capture changed after its attempt finished.
    let log = campaign.directory.join("arm-40/run/session.log");
    let original = std::fs::read(&log).unwrap();
    let mut changed = original.clone();
    changed.extend_from_slice(b"appended\n");
    std::fs::write(&log, &changed).unwrap();
    assert_eq!(
        campaign.import().err().as_deref(),
        Some("arm arm-40: session.log differs from its recorded digest")
    );
    std::fs::write(&log, &original).unwrap();
    assert!(campaign.import().is_ok());

    // A failed attempt is history, never a gap the condition can skip.
    let failed = Campaign::new("failed", Some(17));
    assert_eq!(
        failed.import().err().as_deref(),
        Some("arm arm-17: attempt failed: fixture launch failure")
    );
}
