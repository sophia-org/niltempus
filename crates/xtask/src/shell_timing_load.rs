//! External CPU-load worker controller for shell timing campaigns.
//!
//! The controller spawns a caller-supplied command once per worker — in
//! the campaign, the future xtask CPU-worker subcommand at exactly
//! [`CAMPAIGN_WORKERS`] workers — and produces start/end/exit evidence
//! for every worker. It synthesizes no workload of its own.
//!
//! Bounds, stated honestly: `max_run_ms` is a caller-polled expiry.
//! Nothing here fires when it passes; expiry is enforced when the
//! caller next polls [`LoadController::snapshot`] (or calls
//! [`LoadController::stop`]). It is a hard lifetime bound only when
//! each worker command also bounds its own lifetime.
//!
//! Intended worker commands do not fork: the campaign command is
//! the xtask CPU-worker, one bounded process per worker. Cleanup is
//! proven for direct children; group termination is best effort
//! beyond that and does not prove arbitrary descendant cleanup.
//!
//! The controller makes no priority, affinity or cgroup adjustment:
//! workers inherit the runner's scheduling policy and allowed CPUs.
//! Whether that inheritance held is for parent/worker capture at the
//! runner to verify; this module asserts nothing about it.

use rustix::process::{Pid, Signal, kill_process_group};
use serde::{Deserialize, Serialize};
use std::{
    os::unix::process::{CommandExt as _, ExitStatusExt as _},
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Worker count of a campaign load arm.
pub const CAMPAIGN_WORKERS: usize = 16;

const MAX_WORKERS: usize = 16;
const MAX_RUN_MS: u64 = 600_000;
const MAX_GRACE_MS: u64 = 30_000;
const POLL_INTERVAL: Duration = Duration::from_millis(5);

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// One external command per worker, with controller timing bounds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadSpec {
    pub commands: Vec<Vec<String>>,
    pub max_run_ms: u64,
    pub term_grace_ms: u64,
    pub kill_grace_ms: u64,
}

impl LoadSpec {
    /// Exactly [`CAMPAIGN_WORKERS`] workers running the same command.
    pub fn campaign(
        argv: Vec<String>,
        max_run_ms: u64,
        term_grace_ms: u64,
        kill_grace_ms: u64,
    ) -> Result<Self, String> {
        let spec = Self {
            commands: vec![argv; CAMPAIGN_WORKERS],
            max_run_ms,
            term_grace_ms,
            kill_grace_ms,
        };
        spec.validate()?;
        Ok(spec)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.commands.is_empty() || self.commands.len() > MAX_WORKERS {
            return Err(format!("load requires 1..={MAX_WORKERS} workers"));
        }
        for argv in &self.commands {
            if argv.is_empty() || !Path::new(&argv[0]).is_absolute() {
                return Err("load worker programs must be absolute paths".into());
            }
        }
        if !(1..=MAX_RUN_MS).contains(&self.max_run_ms) {
            return Err(format!("load max_run_ms must be in 1..={MAX_RUN_MS}"));
        }
        for (name, grace) in [
            ("term_grace_ms", self.term_grace_ms),
            ("kill_grace_ms", self.kill_grace_ms),
        ] {
            if !(1..=MAX_GRACE_MS).contains(&grace) {
                return Err(format!("load {name} must be in 1..={MAX_GRACE_MS}"));
            }
        }
        Ok(())
    }
}

/// How a worker's record ended. `Running` and `SpawnFailed` are not
/// exit statuses; nothing here certifies that a load was valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    Running,
    NaturalExit,
    StopRequested,
    MaxRunTimeout,
    SpawnFailed,
    CleanupFailed,
}

/// Start, end and exit evidence for one worker. End timestamps are
/// when the controller observed and reaped the exit. `program` and
/// `args` are the exact command the worker was spawned with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerEvidence {
    pub index: usize,
    pub program: String,
    pub args: Vec<String>,
    pub pid: Option<u32>,
    pub started_unix_ms: Option<u64>,
    /// Monotonic offset from controller start.
    pub started_offset_ms: Option<u64>,
    pub ended_unix_ms: Option<u64>,
    /// Monotonic offset from controller start.
    pub ended_offset_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub end_reason: EndReason,
    pub reaped: bool,
    pub error: Option<String>,
}

/// Evidence for one load session. No predicate here means the load
/// was valid: cleanup completeness and a maintained load are separate
/// facts, and early or nonzero worker exits invalidate a load arm at
/// the runner even when cleanup is complete.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadEvidence {
    pub schema: u8,
    pub requested_workers: usize,
    pub started_unix_ms: u64,
    pub ended_unix_ms: Option<u64>,
    pub workers: Vec<WorkerEvidence>,
}

impl LoadEvidence {
    /// Every spawned worker was reaped and none is left running.
    /// A failed start can be cleanup-complete without ever having
    /// been a complete load.
    pub fn cleanup_complete(&self) -> bool {
        self.workers.iter().all(|w| {
            w.end_reason != EndReason::Running
                && w.end_reason != EndReason::CleanupFailed
                && (w.pid.is_none() || w.reaped)
        })
    }

    /// Every requested worker was spawned.
    pub fn start_complete(&self) -> bool {
        self.workers.len() == self.requested_workers && self.workers.iter().all(|w| w.pid.is_some())
    }

    /// Every requested worker started, none left naturally before
    /// stop or polled expiry began, and every worker's record ends
    /// with an explicit stop reason. A mid-run `Running` snapshot
    /// therefore never satisfies this. Recorded exit codes are not
    /// judged here; the runner treats a nonzero or early exit as an
    /// invalid load.
    pub fn maintained_through_stop(&self) -> bool {
        self.start_complete()
            && self.workers.iter().all(|w| {
                matches!(
                    w.end_reason,
                    EndReason::StopRequested | EndReason::MaxRunTimeout
                )
            })
    }
}

/// A failed start, with the partial evidence. Cleanup of workers
/// that did start was bounded best effort under the same
/// TERM/grace/KILL/grace contract as `stop`; the evidence records
/// which of them were actually reaped. Callers must use `stop` and
/// its evidence for cleanup proof, not this error alone.
#[derive(Clone, Debug)]
pub struct LoadStartError {
    pub message: String,
    pub evidence: LoadEvidence,
}

impl std::fmt::Display for LoadStartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LoadStartError {}

struct Worker {
    evidence: WorkerEvidence,
    /// Owned until reaped. A reaped worker's pid/group is never
    /// signaled again: signaling is only ever done through this
    /// still-owned child, so a reused pid cannot be hit.
    child: Option<Child>,
}

pub struct LoadController {
    spec: LoadSpec,
    origin: Instant,
    origin_unix_ms: u64,
    workers: Vec<Worker>,
    /// Set once termination begins; natural exits observed after
    /// this are attributed to the stop that caused them.
    terminating: Option<EndReason>,
    finished: bool,
}

impl LoadController {
    pub fn start(spec: LoadSpec) -> Result<Self, LoadStartError> {
        spec.validate().map_err(|message| LoadStartError {
            message,
            evidence: LoadEvidence {
                schema: 1,
                requested_workers: spec.commands.len(),
                started_unix_ms: unix_ms(),
                ended_unix_ms: None,
                workers: Vec::new(),
            },
        })?;
        let origin = Instant::now();
        let origin_unix_ms = unix_ms();
        let mut controller = Self {
            spec,
            origin,
            origin_unix_ms,
            workers: Vec::new(),
            terminating: None,
            finished: false,
        };
        for (index, argv) in controller.spec.commands.clone().iter().enumerate() {
            let spawned = Command::new(&argv[0])
                .args(&argv[1..])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn();
            match spawned {
                Ok(child) => {
                    let now = Instant::now();
                    controller.workers.push(Worker {
                        evidence: WorkerEvidence {
                            index,
                            program: argv[0].clone(),
                            args: argv[1..].to_vec(),
                            pid: Some(child.id()),
                            started_unix_ms: Some(unix_ms()),
                            started_offset_ms: Some(now.duration_since(origin).as_millis() as u64),
                            ended_unix_ms: None,
                            ended_offset_ms: None,
                            exit_code: None,
                            signal: None,
                            end_reason: EndReason::Running,
                            reaped: false,
                            error: None,
                        },
                        child: Some(child),
                    });
                }
                Err(error) => {
                    // Record this worker and every unattempted one,
                    // then roll back the workers that did start.
                    for rest in index..controller.spec.commands.len() {
                        let argv = &controller.spec.commands[rest];
                        controller.workers.push(Worker {
                            evidence: WorkerEvidence {
                                index: rest,
                                program: argv[0].clone(),
                                args: argv[1..].to_vec(),
                                pid: None,
                                started_unix_ms: None,
                                started_offset_ms: None,
                                ended_unix_ms: Some(unix_ms()),
                                ended_offset_ms: Some(
                                    Instant::now().duration_since(origin).as_millis() as u64,
                                ),
                                exit_code: None,
                                signal: None,
                                end_reason: EndReason::SpawnFailed,
                                reaped: false,
                                error: Some(if rest == index {
                                    error.to_string()
                                } else {
                                    format!("not started: worker {index} failed to spawn")
                                }),
                            },
                            child: None,
                        });
                    }
                    controller.terminate(EndReason::StopRequested);
                    controller.finished = true;
                    let evidence = controller.report();
                    return Err(LoadStartError {
                        message: format!("load worker {index} failed to spawn: {error}"),
                        evidence,
                    });
                }
            }
        }
        Ok(controller)
    }

    /// Worker pids as spawned, without polling or refreshing any
    /// worker state. For runner-side parent/worker capture.
    pub fn worker_pids(&self) -> Vec<Option<u32>> {
        self.workers.iter().map(|w| w.evidence.pid).collect()
    }

    /// Poll worker states and return current evidence. This is the
    /// caller-polled expiry point: if `max_run_ms` has passed,
    /// termination begins here, not on a timer.
    pub fn snapshot(&mut self) -> LoadEvidence {
        self.refresh();
        if self.terminating.is_none()
            && self.origin.elapsed() >= Duration::from_millis(self.spec.max_run_ms)
        {
            self.terminate(EndReason::MaxRunTimeout);
        }
        self.report()
    }

    /// Terminate every worker and return final evidence. Cleanup
    /// failures are recorded per worker, never discarded.
    pub fn stop(mut self) -> LoadEvidence {
        if self.terminating.is_none() {
            self.terminate(EndReason::StopRequested);
        }
        self.finished = true;
        self.report()
    }

    /// Record exits of workers that have already finished.
    fn refresh(&mut self) {
        for worker in &mut self.workers {
            let Some(child) = worker.child.as_mut() else {
                continue;
            };
            match child.try_wait() {
                Ok(Some(status)) => {
                    worker.child = None;
                    let now = Instant::now();
                    worker.evidence.ended_unix_ms = Some(unix_ms());
                    worker.evidence.ended_offset_ms =
                        Some(now.duration_since(self.origin).as_millis() as u64);
                    worker.evidence.exit_code = status.code();
                    worker.evidence.signal = status.signal();
                    worker.evidence.end_reason = match self.terminating {
                        Some(reason) => reason,
                        None => EndReason::NaturalExit,
                    };
                    worker.evidence.reaped = true;
                }
                Ok(None) => {}
                Err(error) => {
                    worker.evidence.end_reason = EndReason::CleanupFailed;
                    worker.evidence.error = Some(format!("wait failed: {error}"));
                }
            }
        }
    }

    /// Signal one still-owned worker's group. Workers whose child
    /// handle is gone (reaped, or never spawned) are never signaled.
    /// ESRCH only means the group already ended; any other failure
    /// is preserved on that worker's record.
    fn signal_live(&mut self, signal: Signal) {
        for worker in &mut self.workers {
            let Some(child) = worker.child.as_ref() else {
                continue;
            };
            if let Some(group) = Pid::from_raw(child.id() as i32)
                && let Err(error) = kill_process_group(group, signal)
                && error != rustix::io::Errno::SRCH
            {
                worker.evidence.error = Some(format!("signal failed: {error}"));
            }
        }
    }

    fn any_live(&self) -> bool {
        self.workers.iter().any(|w| w.child.is_some())
    }

    /// TERM, bounded grace, KILL, bounded reap. Every live worker is
    /// attempted even when another worker's cleanup fails.
    fn terminate(&mut self, reason: EndReason) {
        // Observe first: a worker that already exited before this
        // stop keeps its NaturalExit record; only exits observed
        // after the cause is installed are attributed to it.
        self.refresh();
        if self.terminating.is_none() {
            self.terminating = Some(reason);
        }
        if !self.any_live() {
            return;
        }
        self.signal_live(Signal::TERM);
        let term_end = Instant::now() + Duration::from_millis(self.spec.term_grace_ms);
        while self.any_live() && Instant::now() < term_end {
            std::thread::sleep(POLL_INTERVAL);
            self.refresh();
        }
        if self.any_live() {
            self.signal_live(Signal::KILL);
            let kill_end = Instant::now() + Duration::from_millis(self.spec.kill_grace_ms);
            while self.any_live() && Instant::now() < kill_end {
                std::thread::sleep(POLL_INTERVAL);
                self.refresh();
            }
        }
        // Still owned after SIGKILL grace (e.g. uninterruptible
        // sleep): record it and keep the handle. Nothing waits on
        // it unboundedly; the CleanupFailed record stands.
        for worker in &mut self.workers {
            if worker.child.is_some() {
                worker.evidence.end_reason = EndReason::CleanupFailed;
                worker
                    .evidence
                    .error
                    .get_or_insert_with(|| "worker still running after SIGKILL grace".into());
            }
        }
    }

    fn report(&self) -> LoadEvidence {
        LoadEvidence {
            schema: 1,
            requested_workers: self.spec.commands.len(),
            started_unix_ms: self.origin_unix_ms,
            ended_unix_ms: (!self.any_live()).then(unix_ms),
            workers: self.workers.iter().map(|w| w.evidence.clone()).collect(),
        }
    }
}

impl Drop for LoadController {
    /// Bounded best effort only: the same TERM/grace/KILL/grace as
    /// `stop`. A worker still owned after that (e.g. in
    /// uninterruptible sleep) keeps its CleanupFailed record in the
    /// last evidence the caller took; Drop itself produces no
    /// evidence and drops the handle with the controller, so it is
    /// never cleanup proof. Callers must use `stop` and its evidence
    /// for that. Drop never blocks on `wait` and never signals a
    /// reaped pid.
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if self.any_live() {
            self.terminate(EndReason::StopRequested);
        }
    }
}
