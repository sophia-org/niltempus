//! Load qualification over supplied observations and fixture proc trees.
//! Nothing is launched; no live `/proc` or `/sys` is read.
use std::path::PathBuf;
use xtask::shell_timing_environment::Interval;
use xtask::shell_timing_environment_capture::CaptureRoots;
use xtask::shell_timing_load::{EndReason, LoadEvidence, WorkerEvidence};
use xtask::shell_timing_load_qualify::*;

const RUNNER: u32 = 100;
const PROGRAM: &str = "/fixture/xtask";
const NAMESPACE: &str = "time:[4026531834]";
const WORKERS: usize = 16;

struct Stat {
    state: char,
    ppid: u32,
    utime: u64,
    nice: i64,
    starttime: u64,
    rt_priority: u64,
    policy: u64,
}

impl Stat {
    fn worker(utime: u64) -> Self {
        Self {
            state: 'R',
            ppid: RUNNER,
            utime,
            nice: 0,
            starttime: 60,
            rt_priority: 0,
            policy: 0,
        }
    }

    /// A 52-field `/proc/<pid>/stat` line.
    fn line(&self, pid: u32) -> String {
        let mut fields: Vec<String> = (3..=52).map(|_| "0".to_string()).collect();
        fields[0] = self.state.to_string();
        fields[4 - 3] = self.ppid.to_string();
        fields[14 - 3] = self.utime.to_string();
        fields[19 - 3] = self.nice.to_string();
        fields[22 - 3] = self.starttime.to_string();
        fields[40 - 3] = self.rt_priority.to_string();
        fields[41 - 3] = self.policy.to_string();
        format!("{pid} (xtask) {}\n", fields.join(" "))
    }
}

fn argv() -> Vec<String> {
    [PROGRAM, "shell-timing", "cpu-worker", "120000"]
        .map(String::from)
        .to_vec()
}

fn status(pid: u32, cpus: &str, threads: u32) -> String {
    format!(
        "Name:\txtask\nUmask:\t0022\nState:\tR (running)\nPid:\t{pid}\nPPid:\t{RUNNER}\nThreads:\t{threads}\nCpus_allowed_list:\t{cpus}\n"
    )
}

fn observation(pid: u32, opened_usec: u64, utime: u64) -> WorkerObservation {
    let stat = Stat::worker(utime).line(pid);
    WorkerObservation {
        pid,
        opened_usec,
        stat: stat.clone(),
        status: status(pid, "0-3", 1),
        cmdline: argv().join("\0") + "\0",
        exe_link: PROGRAM.into(),
        exe_dev: 5,
        exe_ino: 6,
        time_namespace: NAMESPACE.into(),
        tasks: vec![pid],
        children: vec![String::new()],
        closing_stat: stat,
        closed_usec: opened_usec + 5,
    }
}

fn worker_evidence(index: usize) -> WorkerEvidence {
    WorkerEvidence {
        index,
        program: PROGRAM.into(),
        args: argv()[1..].to_vec(),
        pid: Some(200 + index as u32),
        started_unix_ms: Some(1),
        started_offset_ms: Some(0),
        ended_unix_ms: Some(2),
        ended_offset_ms: Some(1),
        exit_code: None,
        signal: Some(15),
        end_reason: EndReason::StopRequested,
        reaped: true,
        error: None,
    }
}

/// A complete record for 16 workers. Supplied observations, not a run.
fn record() -> LoadRecord {
    let runner = Stat {
        starttime: 50,
        ppid: 1,
        ..Stat::worker(0)
    }
    .line(RUNNER);
    LoadRecord {
        schema: 1,
        runner: RunnerObservation {
            pid: RUNNER,
            opened_usec: 900,
            stat: runner.clone(),
            time_namespace: NAMESPACE.into(),
            closing_stat: runner,
            closed_usec: 901,
        },
        program: PROGRAM.into(),
        program_dev: 5,
        program_ino: 6,
        argv: argv(),
        online: OnlineObservation {
            opened_usec: 950,
            text: "0-3\n".into(),
            closed_usec: 951,
        },
        started: (0..WORKERS)
            .map(|i| observation(200 + i as u32, 1_000 + 10 * i as u64, 1))
            .collect(),
        alive: (0..WORKERS)
            .map(|i| observation(200 + i as u32, 9_000_000 + i as u64, 500))
            .collect(),
        stop: LoadEvidence {
            schema: 1,
            requested_workers: WORKERS,
            started_unix_ms: 1,
            ended_unix_ms: Some(3),
            workers: (0..WORKERS).map(worker_evidence).collect(),
        },
        stopped_usec: 9_100_000,
    }
}

fn expect() -> LoadExpect<'static> {
    LoadExpect {
        workers: WORKERS,
        program: PROGRAM,
        min_duration_ms: 60_000,
        measurement: Interval {
            start_usec: 2_000,
            end_usec: 8_000_000,
        },
        online: "0-3\n",
        time_namespace_inode: 4026531834,
    }
}

#[test]
fn a_complete_load_is_proven_from_last_start_to_first_alive() {
    let load = qualify_load(&record(), &expect()).unwrap();
    assert_eq!(load.workers, 16);
    // The latest start observation closes at 1_000 + 150 + 5; the earliest
    // alive observation opens at 9_000_000.
    assert_eq!(
        (load.interval.start_usec, load.interval.end_usec),
        (1_155, 9_000_000)
    );
    assert_eq!(load.measurement, expect().measurement);
    assert_eq!(load.worker_cpu_ticks, vec![499; 16]);
}

fn refusal(mutate: impl FnOnce(&mut LoadRecord)) -> String {
    let mut r = record();
    mutate(&mut r);
    qualify_load(&r, &expect())
        .err()
        .expect("mutation was accepted")
}

fn pick(r: &mut LoadRecord, alive: bool, index: usize) -> &mut WorkerObservation {
    if alive {
        &mut r.alive[index]
    } else {
        &mut r.started[index]
    }
}

/// Replace one stat line, opening and closing, in an observation.
fn restat(o: &mut WorkerObservation, stat: Stat) {
    o.stat = stat.line(o.pid);
    o.closing_stat = o.stat.clone();
}

#[test]
fn the_argv_is_exactly_the_prepared_worker_with_a_covering_duration() {
    let shape = "load argv is not program shell-timing cpu-worker DURATION";
    assert_eq!(refusal(|r| r.argv.push("--extra".into())), shape);
    assert_eq!(refusal(|r| r.argv.truncate(3)), shape);
    assert_eq!(
        refusal(|r| r.argv[2] = "load".into()),
        "load argv is not the prepared cpu-worker"
    );
    assert_eq!(
        refusal(|r| {
            r.program = "/other".into();
            r.argv[0] = "/other".into();
        }),
        "load argv is not the prepared cpu-worker"
    );
    for duration in ["59999", "0120000", "600001", "", "+60000"] {
        assert_eq!(
            refusal(|r| r.argv[3] = duration.into()),
            "load worker duration does not cover the arm lifecycle",
            "{duration:?}"
        );
    }
}

#[test]
fn runner_namespace_and_online_set_come_from_the_environment() {
    assert_eq!(
        refusal(|r| r.runner.time_namespace = "time:[7]".into()),
        "runner time namespace is not the proven capturer namespace"
    );
    assert!(
        refusal(|r| r.runner.time_namespace = "mnt:[4026531834]".into()).contains("time:[inode]")
    );
    assert_eq!(
        refusal(|r| r.runner.closing_stat = r.runner.closing_stat.replace(" 50 ", " 51 ")),
        "runner identity changed during observation"
    );
    assert_eq!(
        refusal(|r| r.online.text = "0-7\n".into()),
        "load CPU online set differs from the environment"
    );
}

#[test]
fn every_declared_worker_must_end_by_requested_term() {
    assert_eq!(
        refusal(|r| {
            r.started.pop();
        }),
        "load does not have the declared worker count"
    );
    assert_eq!(
        refusal(|r| r.stop.requested_workers = 15),
        "load does not have the declared worker count"
    );
    let term = "load worker 3 did not end by requested TERM";
    assert_eq!(refusal(|r| r.stop.workers[3].signal = Some(9)), term);
    assert_eq!(
        refusal(|r| r.stop.workers[3].end_reason = EndReason::MaxRunTimeout),
        term
    );
    assert_eq!(
        refusal(|r| {
            let w = &mut r.stop.workers[3];
            w.end_reason = EndReason::NaturalExit;
            w.signal = None;
            w.exit_code = Some(1);
        }),
        term
    );
    assert_eq!(refusal(|r| r.stop.workers[3].reaped = false), term);
    assert_eq!(
        refusal(|r| r.stop.workers[3].error = Some("signal failed".into())),
        term
    );
    assert_eq!(
        refusal(|r| r.stop.workers[3].pid = None),
        "load worker never started"
    );
    assert_eq!(
        refusal(|r| r.stop.workers[4].pid = Some(203)),
        "load worker 4 differs from its spawn record"
    );
    assert_eq!(
        refusal(|r| r.stop.workers[4].args.push("x".into())),
        "load worker 4 differs from its spawn record"
    );
    assert_eq!(
        refusal(|r| r.alive[4].pid = 299),
        "load worker 4 observations name another pid"
    );
}

#[test]
fn identity_lineage_and_state_are_checked_in_both_observations() {
    assert_eq!(
        refusal(|r| restat(
            &mut r.started[2],
            Stat {
                ppid: 1,
                ..Stat::worker(1)
            }
        )),
        "worker 202: not a child of the runner"
    );
    assert_eq!(
        refusal(|r| restat(
            &mut r.started[2],
            Stat {
                starttime: 40,
                ..Stat::worker(1)
            }
        )),
        "worker 202: not a child of the runner"
    );
    assert_eq!(
        refusal(|r| r.started[2].closing_stat = Stat {
            starttime: 61,
            ..Stat::worker(1)
        }
        .line(202)),
        "worker 202: identity changed during observation"
    );
    assert_eq!(
        refusal(|r| restat(
            &mut r.alive[2],
            Stat {
                starttime: 61,
                ..Stat::worker(500)
            }
        )),
        "load worker 2 pid was reused"
    );
    for (state, alive) in [
        ('Z', true),
        ('X', true),
        ('x', false),
        ('T', false),
        ('t', true),
        ('Q', false),
        ('?', true),
        ('r', true),
    ] {
        let message = refusal(|r| {
            let utime = if alive { 500 } else { 1 };
            restat(
                pick(r, alive, 2),
                Stat {
                    state,
                    ..Stat::worker(utime)
                },
            );
        });
        assert_eq!(message, format!("worker 202: state {state} is not running"));
    }
    assert_eq!(
        refusal(|r| r.alive[2].opened_usec = r.alive[2].closed_usec + 1),
        "worker 202: observation clock moved backward"
    );
}

#[test]
fn scheduling_and_affinity_are_checked_at_start_and_alive() {
    let scheduling = "worker 205: scheduling is not normal, nice 0";
    for alive in [false, true] {
        let utime = if alive { 500 } else { 1 };
        for stat in [
            Stat {
                nice: 5,
                ..Stat::worker(utime)
            },
            Stat {
                nice: -1,
                ..Stat::worker(utime)
            },
            Stat {
                policy: 3,
                ..Stat::worker(utime)
            },
            Stat {
                policy: 1,
                rt_priority: 10,
                ..Stat::worker(utime)
            },
            Stat {
                rt_priority: 1,
                ..Stat::worker(utime)
            },
        ] {
            let message = refusal(|r| restat(pick(r, alive, 5), stat));
            assert_eq!(message, scheduling, "alive={alive}");
        }
        let message = refusal(|r| pick(r, alive, 5).status = status(205, "0-2", 1));
        assert_eq!(message, "worker 205: affinity is not every online CPU");
    }
}

#[test]
fn workers_are_single_threaded_without_descendants_running_the_prepared_file() {
    let threaded = "worker 207: worker is not single-threaded";
    assert_eq!(
        refusal(|r| r.alive[7].status = status(207, "0-3", 2)),
        threaded
    );
    assert_eq!(
        refusal(|r| {
            r.started[7].tasks.push(1207);
            r.started[7].children.push(String::new());
        }),
        threaded
    );
    let descendants = "worker 207: worker has descendants";
    assert_eq!(
        refusal(|r| r.alive[7].children = vec!["1301 ".into()]),
        descendants
    );
    assert_eq!(refusal(|r| r.alive[7].children.clear()), descendants);
    let command = "worker 207: command line differs from the spawn argv";
    assert_eq!(
        refusal(|r| r.started[7].cmdline = r.started[7].cmdline.replace("120000", "60000")),
        command
    );
    assert_eq!(
        refusal(|r| {
            r.started[7].cmdline.pop();
        }),
        command
    );
    assert_eq!(
        refusal(|r| r.alive[7].cmdline.push_str("--extra\0")),
        command
    );
    let executable = "worker 207: executable is not the prepared file";
    assert_eq!(
        refusal(|r| r.alive[7].exe_link.push_str(" (deleted)")),
        executable
    );
    assert_eq!(refusal(|r| r.started[7].exe_ino = 7), executable);
    assert_eq!(refusal(|r| r.started[7].exe_dev = 4), executable);
    assert_eq!(
        refusal(|r| r.alive[7].time_namespace = "time:[4026531835]".into()),
        "worker 207: time namespace differs"
    );
}

#[test]
fn proven_life_must_bracket_the_measurement_and_use_cpu() {
    assert_eq!(
        refusal(|r| restat(&mut r.alive[9], Stat::worker(1))),
        "load worker 9 used no CPU time"
    );
    let bracket = "proven load life does not bracket the measurement";
    // The opening timestamp of the alive read bounds the end, so a read
    // that closes after the measurement but opened inside it is refused.
    assert_eq!(
        refusal(|r| {
            r.alive[9].opened_usec = 7_999_999;
            r.alive[9].closed_usec = 8_000_010;
        }),
        bracket
    );
    // The closing timestamp of the start read bounds the beginning.
    assert_eq!(
        refusal(|r| {
            r.started[9].opened_usec = 1_990;
            r.started[9].closed_usec = 2_001;
        }),
        bracket
    );
    assert_eq!(
        refusal(|r| r.stopped_usec = 9_000_010),
        "load worker 6 observations are out of order"
    );
    assert_eq!(
        refusal(|r| {
            r.alive[9].opened_usec = 1_000;
            r.alive[9].closed_usec = 1_001;
        }),
        "load worker 9 observations are out of order"
    );
}

#[test]
fn stop_evidence_must_use_the_known_schema() {
    assert_eq!(
        refusal(|r| r.stop.schema = 2),
        "unknown load stop evidence schema"
    );
}

#[test]
fn an_empty_or_inverted_measurement_is_refused() {
    for (start_usec, end_usec) in [(8_000_000, 2_000), (5_000, 5_000)] {
        let e = LoadExpect {
            measurement: Interval {
                start_usec,
                end_usec,
            },
            ..expect()
        };
        assert_eq!(
            qualify_load(&record(), &e).err().as_deref(),
            Some("load measurement interval is empty or inverted"),
            "{start_usec}..{end_usec}"
        );
    }
}

#[test]
fn the_retained_record_round_trips_and_refuses_unknown_fields() {
    let bytes = serde_json::to_vec(&record()).unwrap();
    let parsed: LoadRecord = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed, record());
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["started"][0]["extra"] = serde_json::json!(1);
    assert!(serde_json::from_value::<LoadRecord>(value).is_err());
    assert_eq!(refusal(|r| r.schema = 2), "unknown load record schema");
}

/// A fixture proc and sys tree, removed when dropped.
struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("load-qualify-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sys/devices/system/cpu")).unwrap();
        std::fs::write(root.join("sys/devices/system/cpu/online"), "0-3\n").unwrap();
        std::fs::write(root.join("xtask"), "fixture executable, never run").unwrap();
        Self(root)
    }

    fn roots(&self) -> CaptureRoots {
        CaptureRoots::new(self.0.join("proc"), self.0.join("sys"))
    }

    fn process(&self, pid: u32, stat: &str, threads: &[u32]) -> PathBuf {
        let base = self.0.join(format!("proc/{pid}"));
        std::fs::create_dir_all(base.join("ns")).unwrap();
        std::fs::write(base.join("stat"), stat).unwrap();
        std::fs::write(
            base.join("status"),
            status(pid, "0-3", threads.len() as u32),
        )
        .unwrap();
        std::fs::write(base.join("cmdline"), argv().join("\0") + "\0").unwrap();
        std::os::unix::fs::symlink(self.0.join("xtask"), base.join("exe")).unwrap();
        std::os::unix::fs::symlink(NAMESPACE, base.join("ns/time")).unwrap();
        for tid in threads {
            std::fs::create_dir_all(base.join(format!("task/{tid}"))).unwrap();
            std::fs::write(base.join(format!("task/{tid}/children")), "").unwrap();
        }
        base
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn observers_retain_raw_bracketed_reads_from_fixture_roots() {
    use std::os::unix::fs::MetadataExt;
    let tree = Tree::new("observe");
    let roots = tree.roots();
    tree.process(200, &Stat::worker(1).line(200), &[200]);
    let worker = observe_worker(&roots, 200).unwrap();
    let file = std::fs::metadata(tree.0.join("xtask")).unwrap();
    assert_eq!((worker.exe_dev, worker.exe_ino), (file.dev(), file.ino()));
    assert_eq!(worker.exe_link, tree.0.join("xtask").to_str().unwrap());
    assert_eq!(worker.stat, Stat::worker(1).line(200));
    assert_eq!(worker.closing_stat, worker.stat);
    assert_eq!(worker.cmdline, argv().join("\0") + "\0");
    assert_eq!(worker.time_namespace, NAMESPACE);
    assert_eq!(
        (worker.tasks, worker.children),
        (vec![200], vec![String::new()])
    );
    assert!(worker.opened_usec > 0 && worker.opened_usec <= worker.closed_usec);

    let runner = Stat {
        starttime: 50,
        ppid: 1,
        ..Stat::worker(0)
    }
    .line(RUNNER);
    tree.process(RUNNER, &runner, &[RUNNER]);
    let observed = observe_runner(&roots, RUNNER).unwrap();
    assert_eq!(
        (observed.stat, observed.time_namespace),
        (runner, NAMESPACE.into())
    );
    let online = observe_online(&roots).unwrap();
    assert_eq!(online.text, "0-3\n");
    assert!(online.opened_usec <= online.closed_usec);

    // Observations from the tree qualify once stamped into a record.
    let mut r = record();
    r.program = tree.0.join("xtask").to_str().unwrap().into();
    r.argv[0] = r.program.clone();
    (r.program_dev, r.program_ino) = (file.dev(), file.ino());
    for w in &mut r.stop.workers {
        w.program = r.program.clone();
    }
    let cmdline = r.argv.join("\0") + "\0";
    for (index, o) in r.started.iter_mut().chain(r.alive.iter_mut()).enumerate() {
        let pid = o.pid;
        let utime = if index < WORKERS { 1 } else { 500 };
        let base = tree.0.join(format!("proc/{pid}"));
        if !base.exists() {
            tree.process(pid, &Stat::worker(utime).line(pid), &[pid]);
        }
        std::fs::write(base.join("stat"), Stat::worker(utime).line(pid)).unwrap();
        std::fs::write(base.join("cmdline"), &cmdline).unwrap();
        let (opened, closed) = (o.opened_usec, o.closed_usec);
        *o = observe_worker(&roots, pid).unwrap();
        // Supplied clock: the fixture reads happen now, not at run time.
        (o.opened_usec, o.closed_usec) = (opened, closed);
    }
    let program = r.program.clone();
    let expect = LoadExpect {
        program: &program,
        ..expect()
    };
    assert_eq!(
        qualify_load(&r, &expect).unwrap().worker_cpu_ticks,
        vec![499; 16]
    );
}

#[test]
fn observer_reads_are_bounded() {
    let tree = Tree::new("bounds");
    let roots = tree.roots();
    let threads: Vec<u32> = (300..=300 + MAX_TASKS as u32).collect();
    tree.process(300, &Stat::worker(1).line(300), &threads);
    assert!(
        observe_worker(&roots, 300)
            .unwrap_err()
            .contains("more than")
    );
    let base = tree.process(301, &Stat::worker(1).line(301), &[301]);
    std::fs::write(
        base.join("cmdline"),
        "x".repeat(MAX_CMDLINE_BYTES as usize + 1),
    )
    .unwrap();
    assert!(
        observe_worker(&roots, 301)
            .unwrap_err()
            .contains("byte bound")
    );
    assert!(observe_worker(&roots, 999).is_err());
}
