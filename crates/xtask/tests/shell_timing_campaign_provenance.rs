//! Trusted-runner provenance over supplied retained evidence. No
//! files are read by the provider, no process is launched, no
//! hardware is touched.
use std::collections::BTreeMap;
use xtask::shell_timing_campaign::{
    Arm, Condition, EnvironmentTarget, Manifest, Measurement, Variant, Workload,
};
use xtask::shell_timing_campaign_import::{
    ArmFiles, ArmProvenance, MonotonicDomain, VerifiedEnvironment,
};
use xtask::shell_timing_campaign_provenance::TrustedRunnerProvenance;
use xtask::shell_timing_environment::{Capture, Interval, SourceKind, SourceText, Vrr};
use xtask::shell_timing_environment_monitor::{
    BatchEvidence, BindingRecord, ChainRecord, EndEvidence, ExecutableRecord, IdentityRecord,
    IntervalRecord, MonitorEvidence, MonitorOutcome, NamespaceRecord, ProvenAlive, RecheckRecord,
    SourceKindRecord, SourceRecord, StartEvidence, TimeRecord,
};
use xtask::shell_timing_environment_socket::EnvironmentReceipt;
use xtask::shell_timing_load::{EndReason, LoadEvidence, WorkerEvidence};
use xtask::shell_timing_load_qualify::{
    LoadRecord, OnlineObservation, RunnerObservation, WorkerObservation,
};

const NS: u64 = 4_026_531_840;
const HOST: u32 = 4100;
const PEER: u32 = 4200;
const HOST_START: u64 = 4242;
const PEER_START: u64 = 3000;
const RUNNER: &str = "/fixture/runner";

fn manifest() -> Manifest {
    let mut arms = Vec::new();
    for condition in [Condition::Idle, Condition::Load] {
        for pair in 1..=3u8 {
            for workload in [Workload::Panel, Workload::Bulk] {
                for measurement in [
                    Measurement::Latency,
                    Measurement::CpuOnly,
                    Measurement::RustGlobal,
                ] {
                    let order = match pair {
                        1 => [Variant::C, Variant::B, Variant::A],
                        2 => [Variant::A, Variant::B, Variant::C],
                        _ => [Variant::B, Variant::C, Variant::A],
                    };
                    for variant in order {
                        let index = arms.len();
                        let (frames, warmup) = if measurement == Measurement::Latency {
                            (1100, 100)
                        } else {
                            (1000, 0)
                        };
                        arms.push(Arm {
                            id: format!("arm-{index}"),
                            condition,
                            pair,
                            workload,
                            measurement,
                            variant,
                            plan_sha256: "a".repeat(64),
                            peer_config_sha256: "b".repeat(64),
                            frames,
                            warmup,
                            runtime_ms: 60_000,
                        });
                    }
                }
            }
        }
    }
    Manifest {
        schema: 2,
        id: "campaign".into(),
        connector: "DP-2".into(),
        width: 1920,
        height: 1080,
        mode_millihz: 120_000,
        vrr: false,
        environment: EnvironmentTarget {
            card: "card1".into(),
            output: 3,
            scale_milli: 1000,
        },
        load_workers: 16,
        quiet_window: "declared quiet window".into(),
        arms,
    }
}

fn plan_bytes() -> Vec<u8> {
    let artifact = |path: &str| serde_json::json!({"path": path, "sha256": "c".repeat(64)});
    let mut artifacts = BTreeMap::new();
    artifacts.insert("sophia".to_string(), artifact("/fixture/sophia"));
    artifacts.insert("peer".to_string(), artifact("/fixture/peer"));
    artifacts.insert("runner".to_string(), artifact(RUNNER));
    serde_json::to_vec(&serde_json::json!({
        "schema": 1,
        "request": {
            "sophia_source": {"path": "/fixture/sophia-source", "commit": "d".repeat(40)},
            "integration_source": {"path": "/fixture/integration", "commit": "e".repeat(40)},
            "artifacts": artifacts,
            "tty": "tty3",
            "display": 1,
            "input_seat": "seat0",
            "runtime_ms": 60000,
            "wire": "files",
            "output": 3,
            "frames": 1100,
            "mode_millihz": 120000,
            "instrumentation": "none",
            "environment_readback": true
        },
        "argv": ["/fixture/sophia", "session"]
    }))
    .unwrap()
}

fn stat(pid: u32, ppid: u32, start: u64) -> String {
    let mut fields: Vec<String> = (3..=52).map(|_| "0".to_string()).collect();
    fields[0] = "S".into();
    fields[1] = ppid.to_string();
    fields[22 - 3] = start.to_string();
    format!("{pid} (proc-{pid}) {}\n", fields.join(" "))
}

fn identity(pid: u32, start: u64) -> IdentityRecord {
    IdentityRecord {
        pid,
        start_time_ticks: start,
    }
}

fn namespace(pid: u32, id: IdentityRecord, captured: u64) -> NamespaceRecord {
    NamespaceRecord {
        pid,
        identity: id,
        time_namespace_inode: NS,
        origin: format!("/proc/{pid}/ns/time"),
        observed_target: format!("time:[{NS}]"),
        captured_usec: captured,
    }
}

fn source(kind: SourceKindRecord, text: &str, captured: u64) -> SourceRecord {
    SourceRecord {
        kind,
        origin: "fixture".into(),
        captured_usec: captured,
        text: text.into(),
    }
}

fn statics(captured: u64) -> Vec<SourceRecord> {
    vec![
        source(SourceKindRecord::Osrelease, "6.18.55_1\n", captured),
        source(
            SourceKindRecord::ProcVersion,
            "Linux version 6.18.55_1 (builder) #1\n",
            captured,
        ),
        source(SourceKindRecord::CpuOnline, "0-1\n", captured),
        source(
            SourceKindRecord::Governor { cpu: 0 },
            "performance\n",
            captured,
        ),
        source(
            SourceKindRecord::Governor { cpu: 1 },
            "performance\n",
            captured,
        ),
        source(
            SourceKindRecord::DrmDriver {
                card: "card1".into(),
            },
            "amdgpu\n",
            captured,
        ),
    ]
}

fn base_evidence() -> MonitorEvidence {
    let mut start_sources = statics(1100);
    start_sources.push(source(
        SourceKindRecord::ProcStat { pid: HOST },
        &stat(HOST, 110, HOST_START),
        1200,
    ));
    start_sources.push(source(
        SourceKindRecord::ProcStatus { pid: HOST },
        "Name:\thost\nPid:\t4100\nUid:\t1000 1000 1000 1000\nGid:\t1000 1000 1000 1000\nCpus_allowed_list:\t0-1\n",
        1200,
    ));
    start_sources.push(source(
        SourceKindRecord::ProcStat { pid: PEER },
        &stat(PEER, HOST, PEER_START),
        1200,
    ));
    start_sources.push(source(
        SourceKindRecord::ProcStatus { pid: PEER },
        "Name:\tpeer\nPid:\t4200\nUid:\t1000 1000 1000 1000\nGid:\t1000 1000 1000 1000\nCpus_allowed_list:\t0-1\n",
        1200,
    ));
    MonitorEvidence {
        schema: 1,
        outcome: MonitorOutcome::Bound,
        reason: None,
        capture_window: Some(IntervalRecord {
            start_usec: 100,
            end_usec: 10_000,
        }),
        finish_error: None,
        card: "card1".into(),
        environment_socket: "/run/environment.sock".into(),
        binding: Some(BindingRecord {
            wrapper: identity(100, 1000),
            host: identity(HOST, HOST_START),
            peer: identity(PEER, PEER_START),
            chain: vec![
                ChainRecord {
                    identity: identity(100, 1000),
                    parent_pid: 900,
                },
                ChainRecord {
                    identity: identity(110, 1100),
                    parent_pid: 100,
                },
                ChainRecord {
                    identity: identity(HOST, HOST_START),
                    parent_pid: 110,
                },
                ChainRecord {
                    identity: identity(PEER, PEER_START),
                    parent_pid: HOST,
                },
            ],
            host_executable: ExecutableRecord {
                prepared_path: "/fixture/sophia".into(),
                observed_link: "/fixture/sophia".into(),
                dev: 5,
                ino: 6,
            },
            peer_executable: ExecutableRecord {
                prepared_path: "/fixture/peer".into(),
                observed_link: "/fixture/peer".into(),
                dev: 5,
                ino: 7,
            },
            time: TimeRecord {
                reference: namespace(1, identity(1, 10), 150),
                host: namespace(HOST, identity(HOST, HOST_START), 150),
                peer: namespace(PEER, identity(PEER, PEER_START), 150),
                capturer: namespace(900, identity(900, 9000), 150),
                capturer_timens_offsets: "monotonic 0 0\nboottime 0 0\n".into(),
                capturer_timens_offsets_origin: "/proc/900/timens_offsets".into(),
                monotonic_offset_seconds: 0,
                monotonic_offset_nanoseconds: 0,
            },
        }),
        host_uid: Some(1000),
        host_gid: Some(1000),
        last_proven_alive: Some(ProvenAlive {
            interval: IntervalRecord {
                start_usec: 9000,
                end_usec: 9100,
            },
            host_start_time_ticks: HOST_START,
            peer_start_time_ticks: PEER_START,
        }),
        start: Some(StartEvidence {
            window: IntervalRecord {
                start_usec: 1000,
                end_usec: 2000,
            },
            sources: start_sources,
            closing_stats: vec![
                source(
                    SourceKindRecord::ProcStat { pid: HOST },
                    &stat(HOST, 110, HOST_START),
                    1300,
                ),
                source(
                    SourceKindRecord::ProcStat { pid: PEER },
                    &stat(PEER, HOST, PEER_START),
                    1400,
                ),
            ],
            host: identity(HOST, HOST_START),
            peer: identity(PEER, PEER_START),
        }),
        end: EndEvidence {
            window: Some(IntervalRecord {
                start_usec: 9500,
                end_usec: 9900,
            }),
            static_sources: Ok(statics(9600)),
            host_recheck: Some(RecheckRecord::Alive(identity(HOST, HOST_START))),
            peer_recheck: Some(RecheckRecord::Alive(identity(PEER, PEER_START))),
        },
        batches: vec![BatchEvidence {
            interval: IntervalRecord {
                start_usec: 3000,
                end_usec: 3100,
            },
            host_start_time_ticks: HOST_START,
            peer_start_time_ticks: PEER_START,
            first_receipt: 0,
            receipt_count: 2,
        }],
        receipts: vec![
            EnvironmentReceipt {
                pid: HOST,
                uid: 1000,
                gid: 1000,
                received_usec: 3050,
                record: format!(
                    "sophia_shell_environment_output schema=1 pid={HOST} output=3 card=card1 \
                     connector=DP-2 width=1920 height=1080 refresh_millihz=120000 \
                     timing_refresh_millihz=119980 scale_milli=1000 vrr=off monotonic_usec=3050"
                ),
                sender_fdinfo: "pos:\t0\nPid:\t4100\nNSpid:\t4100\n".into(),
            },
            EnvironmentReceipt {
                pid: HOST,
                uid: 1000,
                gid: 1000,
                received_usec: 3060,
                record: format!(
                    "sophia_shell_environment_renderer schema=1 pid={HOST} output=3 card=card1 \
                     driver=amdgpu vendor=Mesa renderer=AMD%20Radeon version=4.6 monotonic_usec=3060"
                ),
                sender_fdinfo: "pos:\t0\nPid:\t4100\nNSpid:\t4100\n".into(),
            },
        ],
    }
}

/// Serialized bytes kept alive beside the borrowing `ArmFiles`.
struct Case {
    plan: Vec<u8>,
    environment: Vec<u8>,
    receipts: Vec<u8>,
}

fn make_case(evidence: &MonitorEvidence, receipts: &[EnvironmentReceipt]) -> Case {
    Case {
        plan: plan_bytes(),
        environment: serde_json::to_vec(evidence).unwrap(),
        receipts: serde_json::to_vec(receipts).unwrap(),
    }
}

impl Case {
    fn files<'a>(&'a self, arm: &'a Arm, load: Option<&'a [u8]>) -> ArmFiles<'a> {
        ArmFiles {
            arm,
            plan: &self.plan,
            environment: &self.environment,
            environment_receipts: &self.receipts,
            load,
        }
    }
}

fn run(mutate: impl FnOnce(&mut MonitorEvidence, &mut Vec<EnvironmentReceipt>)) -> String {
    let manifest = manifest();
    let arm = manifest.arms[0].clone();
    let provider = TrustedRunnerProvenance::new(manifest).unwrap();
    let mut evidence = base_evidence();
    let mut receipts = evidence.receipts.clone();
    mutate(&mut evidence, &mut receipts);
    let case = make_case(&evidence, &receipts);
    provider
        .environment(&case.files(&arm, None))
        .err()
        .expect("mutation was accepted")
}

#[test]
fn bound_and_ended_after_evidence_qualify() {
    let manifest = manifest();
    let arm = manifest.arms[0].clone();
    let provider = TrustedRunnerProvenance::new(manifest).unwrap();
    let evidence = base_evidence();
    let receipts = evidence.receipts.clone();
    let case = make_case(&evidence, &receipts);
    let verified = provider.environment(&case.files(&arm, None)).unwrap();
    assert_eq!(verified.expect.host_pid, HOST);
    assert_eq!(verified.expect.peer_pid, PEER);
    assert_eq!(
        (
            verified.expect.measurement.start_usec,
            verified.expect.measurement.end_usec
        ),
        (100, 9000)
    );
    assert_eq!(verified.host_starttime, HOST_START);
    let log = verified
        .capture
        .sources
        .iter()
        .find(|source| source.kind == SourceKind::HostLog)
        .unwrap();
    assert!(
        log.text
            .starts_with("sophia_shell_environment_output schema=1 pid=4100 output=3")
    );
    assert!(
        log.text
            .contains("sophia_shell_environment_renderer schema=1 pid=4100 output=3")
    );
    assert!(log.text.ends_with('\n'));
    // Normal lifecycle: processes ended after evidence.
    let mut ended = base_evidence();
    ended.outcome = MonitorOutcome::EndedAfterEvidence;
    ended.end.host_recheck = Some(RecheckRecord::Exited);
    ended.end.peer_recheck = Some(RecheckRecord::Exited);
    let receipts = ended.receipts.clone();
    let case = make_case(&ended, &receipts);
    provider.environment(&case.files(&arm, None)).unwrap();
}

#[test]
fn produced_capture_satisfies_the_real_m5_record() {
    use xtask::shell_timing_campaign_import::check_measured_interval;
    use xtask::shell_timing_environment::environment_record;
    // Supplied trusted-runner fixture evidence only: this proves the
    // produced capture satisfies the M5 grammar, not any kernel or
    // authentication fact about a live host.
    for ended in [false, true] {
        let manifest = manifest();
        let arm = manifest.arms[0].clone();
        let provider = TrustedRunnerProvenance::new(manifest).unwrap();
        let mut evidence = base_evidence();
        if ended {
            evidence.outcome = MonitorOutcome::EndedAfterEvidence;
            evidence.end.host_recheck = Some(RecheckRecord::Exited);
            evidence.end.peer_recheck = Some(RecheckRecord::Exited);
        }
        let receipts = evidence.receipts.clone();
        let case = make_case(&evidence, &receipts);
        let verified = provider.environment(&case.files(&arm, None)).unwrap();
        let record = environment_record(&verified.capture, &verified.expect).unwrap();
        assert_eq!(record.verdict, "environment_complete");
        assert_eq!(record.drm_driver.value, "amdgpu");
        // Raw run spans inside the declared measurement pass; spans
        // escaping either bound refuse.
        let inside = Interval {
            start_usec: 2000,
            end_usec: 8000,
        };
        check_measured_interval(&verified, inside).unwrap();
        for outside in [
            Interval {
                start_usec: 50,
                end_usec: 8000,
            },
            Interval {
                start_usec: 2000,
                end_usec: 9500,
            },
        ] {
            assert!(check_measured_interval(&verified, outside).is_err());
        }
    }
}

#[test]
fn disqualifying_outcomes_and_receipt_mismatches_refuse() {
    for outcome in [
        MonitorOutcome::NeverBound,
        MonitorOutcome::NoReceipts,
        MonitorOutcome::EndedBeforeAccept,
        MonitorOutcome::Failed,
    ] {
        let error = run(|evidence, _| evidence.outcome = outcome);
        assert!(error.contains("cannot qualify"), "{error}");
    }
    assert!(
        run(|_, receipts| receipts[0].record.push_str(" forged"))
            .contains("differs from the monitor evidence")
    );
    assert!(
        run(|evidence, receipts| {
            evidence.receipts[0].uid = 999;
            receipts[0].uid = 999;
        })
        .contains("not the supervised host")
    );
    assert!(
        run(|evidence, receipts| {
            evidence.receipts[0].received_usec = 9999;
            receipts[0].received_usec = 9999;
        })
        .contains("outside its batch interval")
    );
    assert!(run(|evidence, _| evidence.batches[0].receipt_count = 0).contains("contiguously"));
}

#[test]
fn binding_static_clock_and_end_mismatches_refuse() {
    assert!(
        run(|evidence, _| {
            evidence
                .binding
                .as_mut()
                .unwrap()
                .host_executable
                .prepared_path = "/fixture/other".into();
        })
        .contains("differs from the frozen plan")
    );
    assert!(
        run(|evidence, _| {
            if let Ok(sources) = &mut evidence.end.static_sources {
                sources[3].text = "powersave\n".into();
            }
        })
        .contains("static environment changed")
    );
    assert!(
        run(|evidence, _| {
            let time = &mut evidence.binding.as_mut().unwrap().time;
            time.capturer_timens_offsets = "monotonic 1 0\nboottime 0 0\n".into();
            time.monotonic_offset_seconds = 1;
        })
        .contains("not exactly zero")
    );
    assert!(
        run(|evidence, _| {
            let peer = &mut evidence.binding.as_mut().unwrap().time.peer;
            peer.observed_target = format!("time:[{}]", NS + 1);
            peer.time_namespace_inode = NS + 1;
        })
        .contains("inodes differ")
    );
    assert!(
        run(|evidence, _| {
            evidence.end.host_recheck = Some(RecheckRecord::Error("denied".into()));
        })
        .contains("end host recheck")
    );
    assert!(run(|evidence, _| evidence.end.peer_recheck = None).contains("end peer recheck"));
    assert!(
        run(|evidence, _| {
            evidence
                .last_proven_alive
                .as_mut()
                .unwrap()
                .host_start_time_ticks = 1;
        })
        .contains("proven-alive")
    );
    assert!(
        run(|evidence, _| {
            evidence.start.as_mut().unwrap().sources[6].text = stat(HOST, 110, HOST_START + 1);
        })
        .contains("start stat")
    );
}

fn stat_with_state(pid: u32, ppid: u32, start: u64, state: &str) -> String {
    let mut fields: Vec<String> = (3..=52).map(|_| "0".to_string()).collect();
    fields[0] = state.into();
    fields[1] = ppid.to_string();
    fields[22 - 3] = start.to_string();
    format!("{pid} (proc-{pid}) {}\n", fields.join(" "))
}

#[test]
fn retained_json_bounds_provenance_and_deleted_exe_refuse() {
    assert!(run(|evidence, _| evidence.schema = 2).contains("schema"));
    // Checked coverage arithmetic: no panic on retained extremes.
    assert!(
        run(|evidence, _| evidence.batches[0].receipt_count = usize::MAX).contains("contiguously")
    );
    assert!(
        run(|evidence, receipts| {
            let extra = receipts[0].clone();
            evidence.receipts = (0..65).map(|_| extra.clone()).collect();
            *receipts = evidence.receipts.clone();
        })
        .contains("bound")
    );
    // Batch outside the start/proven and capture brackets.
    assert!(
        run(|evidence, receipts| {
            evidence.batches[0].interval = IntervalRecord {
                start_usec: 500,
                end_usec: 600,
            };
            evidence.receipts[0].received_usec = 550;
            receipts[0].received_usec = 550;
        })
        .contains("contiguously")
    );
    // End statics must be unique, not collapsed by comparison.
    assert!(
        run(|evidence, _| {
            if let Ok(sources) = &mut evidence.end.static_sources {
                let duplicate = sources[3].clone();
                sources.push(duplicate);
            }
        })
        .contains("duplicate")
    );
    // Namespace provenance: stamps, origins and pid/identity agreement.
    assert!(
        run(|evidence, _| {
            evidence
                .binding
                .as_mut()
                .unwrap()
                .time
                .capturer
                .captured_usec = 99_999;
        })
        .contains("namespace")
    );
    assert!(
        run(|evidence, _| {
            evidence
                .binding
                .as_mut()
                .unwrap()
                .time
                .reference
                .origin
                .clear();
        })
        .contains("namespace")
    );
    assert!(
        run(|evidence, _| {
            evidence
                .binding
                .as_mut()
                .unwrap()
                .time
                .reference
                .identity
                .pid = 2;
        })
        .contains("namespace")
    );
    assert!(
        run(|evidence, _| {
            evidence
                .binding
                .as_mut()
                .unwrap()
                .time
                .capturer_timens_offsets_origin
                .clear();
        })
        .contains("capturer")
    );
    // End window must follow the proven-alive bracket.
    assert!(
        run(|evidence, _| {
            evidence.end.window = Some(IntervalRecord {
                start_usec: 8000,
                end_usec: 8500,
            });
            if let Ok(sources) = &mut evidence.end.static_sources {
                for source in sources.iter_mut() {
                    source.captured_usec = 8200;
                }
            }
        })
        .contains("proven-alive")
    );
    // Closing bracket parent and live state agree with the opening.
    assert!(
        run(|evidence, _| {
            evidence.start.as_mut().unwrap().closing_stats[0].text = stat(HOST, 999, HOST_START);
        })
        .contains("closing stat")
    );
    assert!(
        run(|evidence, _| {
            evidence.start.as_mut().unwrap().sources[6].text =
                stat_with_state(HOST, 110, HOST_START, "Z");
        })
        .contains("ended state")
    );
    // A deleted executable target cannot bind.
    assert!(
        run(|evidence, _| {
            evidence
                .binding
                .as_mut()
                .unwrap()
                .host_executable
                .observed_link = "/fixture/sophia (deleted)".into();
        })
        .contains("frozen plan")
    );
}

fn worker_stat(pid: u32, ppid: u32, utime: u64, start: u64) -> String {
    let mut fields: Vec<String> = (3..=52).map(|_| "0".to_string()).collect();
    fields[0] = "R".into();
    fields[1] = ppid.to_string();
    fields[14 - 3] = utime.to_string();
    fields[22 - 3] = start.to_string();
    format!("{pid} (xtask) {}\n", fields.join(" "))
}

fn load_record() -> LoadRecord {
    let argv: Vec<String> = [RUNNER, "shell-timing", "cpu-worker", "120000"]
        .map(String::from)
        .to_vec();
    let observation = |pid: u32, opened: u64, utime: u64| WorkerObservation {
        pid,
        opened_usec: opened,
        stat: worker_stat(pid, 900, utime, 9060),
        status: format!("Name:\txtask\nPid:\t{pid}\nThreads:\t1\nCpus_allowed_list:\t0-3\n"),
        cmdline: argv.join("\0") + "\0",
        exe_link: RUNNER.into(),
        exe_dev: 5,
        exe_ino: 6,
        time_namespace: format!("time:[{NS}]"),
        tasks: vec![pid],
        children: vec![String::new()],
        closing_stat: worker_stat(pid, 900, utime, 9060),
        closed_usec: opened + 5,
    };
    LoadRecord {
        schema: 1,
        runner: RunnerObservation {
            pid: 900,
            opened_usec: 900,
            stat: worker_stat(900, 1, 0, 9000),
            time_namespace: format!("time:[{NS}]"),
            closing_stat: worker_stat(900, 1, 0, 9000),
            closed_usec: 901,
        },
        program: RUNNER.into(),
        program_dev: 5,
        program_ino: 6,
        argv: argv.clone(),
        online: OnlineObservation {
            opened_usec: 950,
            text: "0-3\n".into(),
            closed_usec: 951,
        },
        started: (0..16)
            .map(|i| observation(200 + i, 1000 + 10 * u64::from(i), 1))
            .collect(),
        alive: (0..16)
            .map(|i| observation(200 + i, 9_000_000 + u64::from(i), 500))
            .collect(),
        stop: LoadEvidence {
            schema: 1,
            requested_workers: 16,
            started_unix_ms: 1,
            ended_unix_ms: Some(3),
            workers: (0..16)
                .map(|i| WorkerEvidence {
                    index: i as usize,
                    program: RUNNER.into(),
                    args: argv[1..].to_vec(),
                    pid: Some(200 + i),
                    started_unix_ms: Some(1),
                    started_offset_ms: Some(0),
                    ended_unix_ms: Some(2),
                    ended_offset_ms: Some(1),
                    exit_code: None,
                    signal: Some(15),
                    end_reason: EndReason::StopRequested,
                    reaped: true,
                    error: None,
                })
                .collect(),
        },
        stopped_usec: 9_100_000,
    }
}

fn verified_environment() -> VerifiedEnvironment {
    VerifiedEnvironment {
        capture: Capture {
            sources: vec![SourceText {
                kind: SourceKind::CpuOnline,
                origin: "fixture".into(),
                captured_usec: 1000,
                text: "0-3\n".into(),
            }],
        },
        expect: xtask::shell_timing_environment::Expect {
            output: 3,
            card: "card1".into(),
            connector: "DP-2".into(),
            width: 1920,
            height: 1080,
            refresh_millihz: 120_000,
            scale_milli: 1000,
            vrr: Vrr::Off,
            host_pid: HOST,
            peer_pid: PEER,
            capture: Interval {
                start_usec: 100,
                end_usec: 10_000,
            },
            measurement: Interval {
                start_usec: 2000,
                end_usec: 8_000_000,
            },
        },
        host_starttime: HOST_START,
        peer_starttime: PEER_START,
        clock: MonotonicDomain {
            reference: NS,
            host: NS,
            peer: NS,
            capturer: NS,
            monotonic_offset_seconds: 0,
            monotonic_offset_nanoseconds: 0,
        },
    }
}

#[test]
fn load_uses_the_supplied_verified_environment() {
    let manifest = manifest();
    let arm = manifest.arms[0].clone();
    let provider = TrustedRunnerProvenance::new(manifest).unwrap();
    let evidence = base_evidence();
    let receipts = evidence.receipts.clone();
    let case = make_case(&evidence, &receipts);
    let verified = verified_environment();
    let record = serde_json::to_vec(&load_record()).unwrap();
    let load = provider
        .load(&case.files(&arm, Some(&record)), &verified)
        .unwrap();
    assert_eq!(load.workers, 16);
    assert_eq!(load.measurement, verified.expect.measurement);
    // Missing and mismatched load records refuse.
    assert!(provider.load(&case.files(&arm, None), &verified).is_err());
    let mut wrong = load_record();
    wrong.program = "/fixture/other".into();
    wrong.argv[0] = "/fixture/other".into();
    let record = serde_json::to_vec(&wrong).unwrap();
    assert!(
        provider
            .load(&case.files(&arm, Some(&record)), &verified)
            .is_err()
    );
}
