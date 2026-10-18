//! Live observation owned by the attended campaign runner.
//!
//! Construction does no I/O. `prepare` is invoked only by run-native after
//! its attended-console, source, artifact and executable checks. No worker is
//! started by an importer or a device-free fixture. Cleanup evidence is written
//! even when preparation or the wrapper fails; it is never a passing verdict.
use crate::shell_timing_campaign::Condition;
use crate::shell_timing_environment_capture::{CaptureRoots, Capturer, monotonic_usec};
use crate::shell_timing_environment_monitor::{EnvironmentMonitor, MonitorConfig, MonitorOutcome};
use crate::shell_timing_load::{EndReason, LoadController, LoadSpec};
use crate::shell_timing_load_qualify::{
    LoadRecord, OnlineObservation, RunnerObservation, WorkerObservation, observe_online,
    observe_runner, observe_worker,
};
use crate::shell_timing_native::{CampaignObserver, Request};
use crate::shell_timing_process_binding::Binder;
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const WORKER_START_LIMIT: Duration = Duration::from_secs(5);
const WORKER_START_POLL: Duration = Duration::from_millis(10);
const TERM_GRACE_MS: u64 = 2_000;
const KILL_GRACE_MS: u64 = 2_000;

/// The wrapper has runtime + 90 seconds to restore the console. Workers get
/// another 30 seconds for setup, the wrapper's 10-second Drop grace and final
/// observations. They remain independently bounded if the runner stops making
/// progress; exhausting that margin invalidates the arm.
pub fn load_lifetime_ms(runtime_ms: u64) -> Result<u64, String> {
    runtime_ms
        .checked_add(120_000)
        .filter(|&ms| ms <= crate::shell_timing_cpu_worker::MAX_DURATION_MS)
        .ok_or_else(|| "campaign runtime leaves no bounded load cleanup window".into())
}

fn write_new<T: Serialize>(stage: &Path, name: &str, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let path = stage.join(name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("{}: {e}", path.display()))
}

struct RunningLoad {
    controller: LoadController,
    runner: RunnerObservation,
    online: OnlineObservation,
    program: String,
    dev: u64,
    ino: u64,
    argv: Vec<String>,
    started: Vec<WorkerObservation>,
}

/// Used only by the attended campaign entry point. The receiver is bound
/// before spawning the wrapper; load observations precede that binding so the
/// load can cover the entire monitor capture window.
pub struct LiveCampaignObserver {
    condition: Condition,
    card: String,
    roots: CaptureRoots,
    monitor: Option<EnvironmentMonitor>,
    load: Option<RunningLoad>,
    load_failure: Option<serde_json::Value>,
    prepared: bool,
    finished: bool,
}

impl LiveCampaignObserver {
    pub fn new(condition: Condition, card: String) -> Self {
        Self {
            condition,
            card,
            roots: CaptureRoots::system(),
            monitor: None,
            load: None,
            load_failure: None,
            prepared: false,
            finished: false,
        }
    }

    fn start_load(&mut self, request: &Request) -> Result<(), String> {
        let lifetime = load_lifetime_ms(request.runtime_ms)?;
        let program_path = &request.artifacts["runner"].path;
        let program = program_path
            .to_str()
            .ok_or("runner path is not UTF-8")?
            .to_owned();
        let metadata = fs::metadata(program_path).map_err(|e| e.to_string())?;
        let running_image = fs::metadata("/proc/self/exe").map_err(|e| e.to_string())?;
        if (metadata.dev(), metadata.ino()) != (running_image.dev(), running_image.ino()) {
            return Err("load executable differs from the verified running image".into());
        }
        let runner = observe_runner(&self.roots, std::process::id())?;
        let online = observe_online(&self.roots)?;
        let argv = vec![
            program.clone(),
            "shell-timing".into(),
            "cpu-worker".into(),
            lifetime.to_string(),
        ];
        let spec = LoadSpec::campaign(argv.clone(), lifetime, TERM_GRACE_MS, KILL_GRACE_MS)?;
        let controller = match LoadController::start(spec) {
            Ok(controller) => controller,
            Err(error) => {
                self.load_failure = Some(serde_json::json!({
                    "schema": 1, "error": error.message, "stop": error.evidence,
                }));
                return Err(format!("campaign load start failed: {error}"));
            }
        };
        // Retain the controller before any fallible observation. finish must
        // always stop these children, including a partially observed start.
        self.load = Some(RunningLoad {
            controller,
            runner,
            online,
            program,
            dev: running_image.dev(),
            ino: running_image.ino(),
            argv,
            started: Vec::new(),
        });
        let load = self.load.as_mut().unwrap();
        let deadline = Instant::now() + WORKER_START_LIMIT;
        let expected_cmdline = format!("{}\0", load.argv.join("\0"));
        for pid in load.controller.worker_pids() {
            let pid = pid.ok_or("load child did not start")?;
            loop {
                let observed = observe_worker(&self.roots, pid)?;
                // fork/exec shares the runner executable: exe identity alone
                // is insufficient to prove the worker command has started.
                if observed.cmdline == expected_cmdline
                    && (observed.exe_dev, observed.exe_ino) == (load.dev, load.ino)
                    && !observed.exe_link.ends_with(" (deleted)")
                {
                    load.started.push(observed);
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "load worker {pid} did not exec the prepared command"
                    ));
                }
                std::thread::sleep(WORKER_START_POLL);
            }
        }
        Ok(())
    }

    fn finish_load(&mut self, stage: &Path) -> Result<(), String> {
        if let Some(failure) = self.load_failure.take() {
            write_new(stage, "load.json", &failure)?;
            return Err("load failed to start; cleanup evidence retained".into());
        }
        let Some(load) = self.load.take() else {
            return if self.condition == Condition::Load {
                Err("load was not prepared".into())
            } else {
                Ok(())
            };
        };
        // Never let an observation error bypass stop. The raw partial data
        // and cleanup report survive even if the record cannot qualify.
        let alive = load
            .controller
            .worker_pids()
            .into_iter()
            .map(|pid| observe_worker(&self.roots, pid.ok_or("load child missing")?))
            .collect::<Result<Vec<_>, String>>();
        let stop = load.controller.stop();
        let stopped_usec = monotonic_usec();
        let cleanup_ok = stop.cleanup_complete()
            && stop.workers.iter().all(|worker| {
                worker.end_reason == EndReason::StopRequested
                    && worker.signal == Some(15)
                    && worker.exit_code.is_none()
                    && worker.error.is_none()
            });
        match (alive, stopped_usec) {
            (Ok(alive), Ok(stopped_usec)) => {
                write_new(
                    stage,
                    "load.json",
                    &LoadRecord {
                        schema: 1,
                        runner: load.runner,
                        program: load.program,
                        program_dev: load.dev,
                        program_ino: load.ino,
                        argv: load.argv,
                        online: load.online,
                        started: load.started,
                        alive,
                        stop,
                        stopped_usec,
                    },
                )?;
                if !cleanup_ok {
                    return Err("load workers did not all end by requested TERM".into());
                }
                Ok(())
            }
            (alive, stopped) => {
                let error = format!(
                    "load final observation failed: alive={:?}, clock={:?}",
                    alive.as_ref().err(),
                    stopped.as_ref().err()
                );
                write_new(
                    stage,
                    "load.json",
                    &serde_json::json!({
                        "schema": 1, "error": error, "stop": stop,
                        "started": load.started, "alive": alive, "stopped_usec": stopped,
                    }),
                )?;
                Err(error)
            }
        }
    }
}

impl CampaignObserver for LiveCampaignObserver {
    fn prepare(
        &mut self,
        request: &Request,
        host_argv: &[String],
        stage: &Path,
    ) -> Result<PathBuf, String> {
        if self.prepared || self.finished {
            return Err("campaign observer was already used".into());
        }
        self.prepared = true;
        if !request.environment_readback {
            return Err("campaign requires environment observation".into());
        }
        if self.condition == Condition::Load {
            self.start_load(request)?;
        }
        let socket = stage.join("environment.sock");
        self.monitor = Some(EnvironmentMonitor::new(
            MonitorConfig {
                host_executable: request.artifacts["sophia"].path.clone(),
                host_argv: host_argv.to_vec(),
                // Prepared native argv has no bubblewrap override; Sophia uses
                // this default. Binder retains and rechecks its device/inode.
                supervisor_executable: PathBuf::from("/usr/bin/bwrap"),
                peer_executable: request.artifacts["peer"].path.clone(),
                card: self.card.clone(),
                environment_socket: socket.clone(),
            },
            Binder::new(&self.roots, std::process::id()),
            Capturer::new(self.roots.clone()),
        )?);
        Ok(socket)
    }

    fn observe(&mut self, wrapper_pid: u32, _logs: &Path) -> Result<(), String> {
        // Observe both owners even if one fails; the caller preserves the first
        // error while continuing the restoration loop.
        let environment = self
            .monitor
            .as_mut()
            .ok_or("environment monitor not prepared")?
            .observe(wrapper_pid)
            .map(|_| ());
        let load = match self.load.as_mut() {
            Some(load) => {
                let snapshot = load.controller.snapshot();
                if snapshot
                    .workers
                    .iter()
                    .all(|worker| worker.end_reason == EndReason::Running)
                {
                    Ok(())
                } else {
                    Err("campaign load ended before the wrapper completed".into())
                }
            }
            None => Ok(()),
        };
        environment.and(load)
    }

    fn finish(&mut self, stage: &Path) -> Result<(), String> {
        if self.finished {
            return Err("campaign observer was already finished".into());
        }
        self.finished = true;
        // Stop load before any serialization or disk error can escape.
        let load = self.finish_load(stage);
        let environment = match self.monitor.take() {
            Some(monitor) => {
                let evidence = monitor.finish();
                let valid = matches!(
                    evidence.outcome,
                    MonitorOutcome::Bound | MonitorOutcome::EndedAfterEvidence
                ) && evidence.finish_error.is_none()
                    && evidence.end.static_sources.is_ok();
                let receipts = write_new(stage, "environment-receipts.json", &evidence.receipts);
                let record = write_new(stage, "environment.json", &evidence);
                receipts.and(record).and_then(|()| {
                    if valid {
                        Ok(())
                    } else {
                        Err("environment monitoring did not retain a complete bound capture".into())
                    }
                })
            }
            None => {
                let error = "environment monitor was not prepared";
                let receipts = write_new(stage, "environment-receipts.json", &Vec::<String>::new());
                let record = write_new(
                    stage,
                    "environment.json",
                    &serde_json::json!({"schema":1,"error":error}),
                );
                receipts.and(record).and(Err(error.into()))
            }
        };
        match (load, environment) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(a), Err(b)) => Err(format!("{a}; {b}")),
        }
    }
}
