//! Device-free load-controller controls. Two workers only, `/bin`
//! helper commands only: never the 16-worker campaign form, never a
//! native session, never a desktop.

use std::{
    path::Path,
    time::{Duration, Instant},
};
use xtask::shell_timing_load::{CAMPAIGN_WORKERS, EndReason, LoadController, LoadSpec};

fn spec(commands: Vec<Vec<String>>) -> LoadSpec {
    LoadSpec {
        commands,
        max_run_ms: 60_000,
        term_grace_ms: 500,
        kill_grace_ms: 500,
    }
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(ToString::to_string).collect()
}

fn sleepers() -> LoadSpec {
    spec(vec![
        argv(&["/bin/sleep", "30"]),
        argv(&["/bin/sleep", "30"]),
    ])
}

fn pid_gone(pid: u32) -> bool {
    !Path::new(&format!("/proc/{pid}")).exists()
}

#[test]
fn campaign_shape_is_sixteen_without_spawning() {
    let spec = LoadSpec::campaign(argv(&["/bin/true"]), 60_000, 500, 500).unwrap();
    assert_eq!(spec.commands.len(), CAMPAIGN_WORKERS);
    assert!(spec.validate().is_ok());
}

#[test]
fn spec_rejects_relative_programs_and_bad_bounds() {
    assert!(spec(vec![argv(&["true"])]).validate().is_err());
    assert!(spec(Vec::new()).validate().is_err());
    let mut bad = spec(vec![argv(&["/bin/true"])]);
    bad.max_run_ms = 0;
    assert!(bad.validate().is_err());
    let mut bad = spec(vec![argv(&["/bin/true"])]);
    bad.kill_grace_ms = 0;
    assert!(bad.validate().is_err());
}

#[test]
fn natural_exits_are_recorded_with_exact_codes() {
    let mut controller =
        LoadController::start(spec(vec![argv(&["/bin/true"]), argv(&["/bin/true"])])).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let report = loop {
        let report = controller.snapshot();
        if report.workers.iter().all(|w| w.reaped) {
            break report;
        }
        assert!(Instant::now() < deadline, "workers never exited");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(report.workers.len(), 2);
    assert!(report.workers.iter().all(|w| w.pid.is_some()));
    assert!(
        report
            .workers
            .iter()
            .all(|w| w.exit_code == Some(0) && w.end_reason == EndReason::NaturalExit)
    );
    assert!(!report.maintained_through_stop());
    let report = controller.stop();
    assert!(report.cleanup_complete());
}

#[test]
fn stop_terminates_and_reaps_sleeping_workers() {
    let controller = LoadController::start(sleepers()).unwrap();
    let pids: Vec<u32> = controller
        .worker_pids()
        .into_iter()
        .map(Option::unwrap)
        .collect();
    let began = Instant::now();
    let report = controller.stop();
    assert!(began.elapsed() < Duration::from_secs(10));
    assert!(report.cleanup_complete());
    assert!(report.maintained_through_stop());
    assert!(
        report
            .workers
            .iter()
            .all(|w| w.end_reason == EndReason::StopRequested && w.reaped)
    );
    assert!(
        report
            .workers
            .iter()
            .all(|w| w.program == "/bin/sleep" && w.args == ["30"])
    );
    assert!(pids.iter().all(|pid| pid_gone(*pid)));
}

#[test]
fn spawn_failure_rolls_back_started_workers() {
    let error = match LoadController::start(spec(vec![
        argv(&["/bin/sleep", "30"]),
        argv(&["/nonexistent/load-worker"]),
    ])) {
        Ok(_) => panic!("second worker must fail to spawn"),
        Err(error) => error,
    };
    let report = &error.evidence;
    assert_eq!(report.workers.len(), 2);
    let started = report.workers[0].pid.expect("first worker started");
    assert!(report.workers[0].reaped);
    assert_eq!(report.workers[1].end_reason, EndReason::SpawnFailed);
    assert!(report.cleanup_complete());
    assert!(!report.start_complete());
    assert!(pid_gone(started));
}

#[test]
fn drop_terminates_and_reaps_sleeping_workers() {
    let controller = LoadController::start(sleepers()).unwrap();
    let pids: Vec<u32> = controller
        .worker_pids()
        .into_iter()
        .map(Option::unwrap)
        .collect();
    drop(controller);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !pids.iter().all(|pid| pid_gone(*pid)) {
        assert!(Instant::now() < deadline, "dropped workers survived");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn early_exit_is_visible_and_stop_still_cleans_up() {
    let mut controller = LoadController::start(spec(vec![
        argv(&["/bin/true"]),
        argv(&["/bin/sleep", "30"]),
    ]))
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let report = controller.snapshot();
        if report.workers[0].reaped {
            assert!(!report.maintained_through_stop());
            break;
        }
        assert!(Instant::now() < deadline, "first worker never exited");
        std::thread::sleep(Duration::from_millis(10));
    }
    let report = controller.stop();
    assert!(report.cleanup_complete());
    assert!(!report.maintained_through_stop());
}

#[test]
fn max_run_expiry_is_caller_polled() {
    let mut limited = sleepers();
    limited.max_run_ms = 150;
    limited.term_grace_ms = 200;
    limited.kill_grace_ms = 200;
    let mut controller = LoadController::start(limited).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let report = controller.snapshot();
        if report.cleanup_complete() {
            assert!(
                report
                    .workers
                    .iter()
                    .all(|w| w.end_reason == EndReason::MaxRunTimeout)
            );
            break;
        }
        assert!(Instant::now() < deadline, "polled expiry never fired");
        std::thread::sleep(Duration::from_millis(10));
    }
    let report = controller.stop();
    assert!(report.cleanup_complete());
    assert!(
        report
            .workers
            .iter()
            .all(|w| w.end_reason == EndReason::MaxRunTimeout)
    );
}

#[test]
fn exited_but_unobserved_before_stop_keeps_natural_exit() {
    // Both workers exit on their own before stop is ever called.
    // Their exited state is observed externally with waitid
    // WNOWAIT, which does not reap them; the controller itself is
    // never polled in between. The first controller observation
    // happens inside stop, and must not relabel them StopRequested.
    use rustix::process::{Pid, WaitId, WaitIdOptions};
    let controller =
        LoadController::start(spec(vec![argv(&["/bin/true"]), argv(&["/bin/true"])])).unwrap();
    let pids: Vec<u32> = controller
        .worker_pids()
        .into_iter()
        .map(Option::unwrap)
        .collect();
    let deadline = Instant::now() + Duration::from_secs(5);
    for pid in &pids {
        let raw = Pid::from_raw(*pid as i32).unwrap();
        loop {
            let status = rustix::process::waitid(
                WaitId::Pid(raw),
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
            )
            .unwrap();
            if status.is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "worker {pid} never exited");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let report = controller.stop();
    assert!(
        report
            .workers
            .iter()
            .all(|w| w.end_reason == EndReason::NaturalExit && w.exit_code == Some(0))
    );
    assert!(!report.maintained_through_stop());
    assert!(report.cleanup_complete());
}

#[test]
fn evidence_serializes_with_command_and_both_clocks() {
    let controller =
        LoadController::start(spec(vec![argv(&["/bin/true"]), argv(&["/bin/true"])])).unwrap();
    let report = controller.stop();
    for worker in &report.workers {
        assert_eq!(worker.program, "/bin/true");
        assert!(worker.started_unix_ms.is_some());
        assert!(worker.started_offset_ms.is_some());
        assert!(worker.ended_unix_ms.is_some());
        assert!(worker.ended_offset_ms.is_some());
    }
    let decoded: xtask::shell_timing_load::LoadEvidence =
        serde_json::from_str(&serde_json::to_string(&report).unwrap()).unwrap();
    assert_eq!(decoded, report);
    assert!(decoded.cleanup_complete());
}
