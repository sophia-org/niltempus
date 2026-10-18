//! Qualify a retained campaign load from the runner's own `/proc` reads.
//!
//! The root runner starts the workers through `LoadController`, observes
//! each worker once after start and once after the measurement ends, stops
//! them, and writes [`LoadRecord`] as `load.json`. That record is
//! runner-written evidence bound by the attempt digest; it is trusted-runner
//! evidence, not an attestation against an adversarial operator.
//!
//! Qualification reads the retained raw texts, never the observer's
//! conclusions. Every observation is bracketed by `CLOCK_MONOTONIC` reads
//! taken before its first and after its last read. A worker is proven alive
//! from the closing timestamp of its start observation to the opening
//! timestamp of its alive observation: the same `(pid, starttime)` at both
//! ends cannot be two processes. Scheduling, affinity, threads and
//! descendants are checked in both observations; nothing claims they held
//! between them. Controller times are opaque and used only for cleanup
//! facts. Nothing here launches a process.
use crate::shell_timing_campaign_import::VerifiedLoad;
use crate::shell_timing_cpu_worker::MAX_DURATION_MS;
use crate::shell_timing_environment::{Interval, affinity, cpu_list, scheduling};
use crate::shell_timing_environment_capture::{CaptureRoots, known_state, monotonic_usec};
use crate::shell_timing_load::{EndReason, LoadEvidence};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Bound on stat, status, children and online reads.
pub const MAX_READ_BYTES: u64 = 65_536;
/// Bound on one worker's command line.
pub const MAX_CMDLINE_BYTES: u64 = 65_536;
/// Bound on task entries listed for one process.
pub const MAX_TASKS: usize = 64;
/// SIGTERM. Workers must end by the controller's TERM, not KILL.
const SIGTERM: i32 = 15;
const WORKER_COMMAND: [&str; 2] = ["shell-timing", "cpu-worker"];

/// One bracketed observation of a load worker: raw texts as read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerObservation {
    pub pid: u32,
    /// `CLOCK_MONOTONIC` microseconds before the first read.
    pub opened_usec: u64,
    pub stat: String,
    pub status: String,
    /// `/proc/<pid>/cmdline`, NUL-terminated arguments.
    pub cmdline: String,
    /// `/proc/<pid>/exe` link text.
    pub exe_link: String,
    /// Device and inode of the file `/proc/<pid>/exe` resolves to.
    pub exe_dev: u64,
    pub exe_ino: u64,
    /// `/proc/<pid>/ns/time` link text.
    pub time_namespace: String,
    /// `/proc/<pid>/task` entries, ascending.
    pub tasks: Vec<u32>,
    /// `/proc/<pid>/task/<tid>/children` for each task, in task order.
    pub children: Vec<String>,
    /// The second stat read, after all others.
    pub closing_stat: String,
    /// `CLOCK_MONOTONIC` microseconds after the last read.
    pub closed_usec: u64,
}

/// The runner's own identity and time namespace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerObservation {
    pub pid: u32,
    pub opened_usec: u64,
    pub stat: String,
    pub time_namespace: String,
    pub closing_stat: String,
    pub closed_usec: u64,
}

/// `/sys/devices/system/cpu/online` as read at load start.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnlineObservation {
    pub opened_usec: u64,
    pub text: String,
    pub closed_usec: u64,
}

/// `load.json`: everything the runner retained for one load arm.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadRecord {
    pub schema: u8,
    pub runner: RunnerObservation,
    /// The prepared worker executable and its identity at launch.
    pub program: String,
    pub program_dev: u64,
    pub program_ino: u64,
    /// The exact spawn argv, the same for every worker.
    pub argv: Vec<String>,
    pub online: OnlineObservation,
    /// One observation per worker after start, in worker index order.
    pub started: Vec<WorkerObservation>,
    /// One observation per worker after the measurement, before stop.
    pub alive: Vec<WorkerObservation>,
    /// `LoadController::stop` evidence.
    pub stop: LoadEvidence,
    /// `CLOCK_MONOTONIC` microseconds when stop returned.
    pub stopped_usec: u64,
}

/// What the load must match, from the frozen arm and the authenticated
/// environment.
pub struct LoadExpect<'a> {
    pub workers: usize,
    /// The plan's runner artifact path; the worker is that executable.
    pub program: &'a str,
    /// The arm's declared lifecycle; each worker must be asked to run at
    /// least this long.
    pub min_duration_ms: u64,
    pub measurement: Interval,
    /// The environment capture's CPU online text.
    pub online: &'a str,
    /// The proven capturer time-namespace inode.
    pub time_namespace_inode: u64,
}

fn read(path: &Path, limit: u64) -> Result<String, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{}: exceeds {limit} byte bound", path.display()));
    }
    String::from_utf8(bytes).map_err(|_| format!("{}: not UTF-8", path.display()))
}

fn link(path: &Path) -> Result<String, String> {
    std::fs::read_link(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .into_os_string()
        .into_string()
        .map_err(|_| format!("{}: not UTF-8", path.display()))
}

/// Observe one worker under `roots`. Reads only; judges nothing.
pub fn observe_worker(roots: &CaptureRoots, pid: u32) -> Result<WorkerObservation, String> {
    let base = roots.proc_root.join(pid.to_string());
    let opened_usec = monotonic_usec()?;
    let stat = read(&base.join("stat"), MAX_READ_BYTES)?;
    let status = read(&base.join("status"), MAX_READ_BYTES)?;
    let cmdline = read(&base.join("cmdline"), MAX_CMDLINE_BYTES)?;
    let exe_link = link(&base.join("exe"))?;
    let exe = std::fs::metadata(base.join("exe"))
        .map_err(|e| format!("{}: {e}", base.join("exe").display()))?;
    let time_namespace = link(&base.join("ns/time"))?;
    let mut tasks = Vec::new();
    for entry in
        std::fs::read_dir(base.join("task")).map_err(|e| format!("{}: {e}", base.display()))?
    {
        let name = entry.map_err(|e| e.to_string())?.file_name();
        let tid = name
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
            .ok_or("task entry is not a thread id")?;
        tasks.push(tid);
        if tasks.len() > MAX_TASKS {
            return Err(format!("pid {pid} lists more than {MAX_TASKS} tasks"));
        }
    }
    tasks.sort_unstable();
    let children = tasks
        .iter()
        .map(|tid| read(&base.join(format!("task/{tid}/children")), MAX_READ_BYTES))
        .collect::<Result<_, _>>()?;
    let closing_stat = read(&base.join("stat"), MAX_READ_BYTES)?;
    Ok(WorkerObservation {
        pid,
        opened_usec,
        stat,
        status,
        cmdline,
        exe_link,
        exe_dev: exe.dev(),
        exe_ino: exe.ino(),
        time_namespace,
        tasks,
        children,
        closing_stat,
        closed_usec: monotonic_usec()?,
    })
}

/// Observe the runner's identity and time namespace.
pub fn observe_runner(roots: &CaptureRoots, pid: u32) -> Result<RunnerObservation, String> {
    let base = roots.proc_root.join(pid.to_string());
    let opened_usec = monotonic_usec()?;
    let stat = read(&base.join("stat"), MAX_READ_BYTES)?;
    let time_namespace = link(&base.join("ns/time"))?;
    let closing_stat = read(&base.join("stat"), MAX_READ_BYTES)?;
    Ok(RunnerObservation {
        pid,
        opened_usec,
        stat,
        time_namespace,
        closing_stat,
        closed_usec: monotonic_usec()?,
    })
}

/// Observe the CPU online set.
pub fn observe_online(roots: &CaptureRoots) -> Result<OnlineObservation, String> {
    let opened_usec = monotonic_usec()?;
    let text = read(
        &roots.sys_root.join("devices/system/cpu/online"),
        MAX_READ_BYTES,
    )?;
    Ok(OnlineObservation {
        opened_usec,
        text,
        closed_usec: monotonic_usec()?,
    })
}

/// Stat facts beyond scheduling: fields counted after the last `)`.
struct StatFacts {
    state: char,
    ppid: u32,
    cpu_ticks: u64,
    starttime: u64,
}

fn stat_facts(text: &str, pid: u32) -> Result<StatFacts, String> {
    let line = text.strip_suffix('\n').unwrap_or(text);
    let open = line.find(" (").ok_or("stat: no command name")?;
    let close = line.rfind(") ").ok_or("stat: no command name")?;
    if line.contains('\n') || close < open || line[..open] != pid.to_string() {
        return Err(format!("stat: not the record for pid {pid}"));
    }
    let fields: Vec<&str> = line[close + 2..].split(' ').collect();
    let number = |n: usize| -> Result<u64, String> {
        let text = fields
            .get(n - 3)
            .ok_or_else(|| format!("stat: missing field {n}"))?;
        if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("stat: bad field {n}"));
        }
        text.parse().map_err(|_| format!("stat: bad field {n}"))
    };
    let state = match fields.first().map(|s| s.as_bytes()) {
        Some([state]) => char::from(*state),
        _ => return Err("stat: bad state".into()),
    };
    Ok(StatFacts {
        state,
        ppid: u32::try_from(number(4)?).map_err(|_| "stat: bad ppid")?,
        cpu_ticks: number(14)?
            .checked_add(number(15)?)
            .ok_or("stat: cpu time overflow")?,
        starttime: number(22)?,
    })
}

fn namespace_inode(text: &str) -> Result<u64, String> {
    text.strip_prefix("time:[")
        .and_then(|rest| rest.strip_suffix(']'))
        .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|digits| digits.parse().ok())
        .filter(|&inode| inode != 0)
        .ok_or_else(|| format!("time namespace link {text:?} is not time:[inode]"))
}

fn threads(status: &str) -> Result<u64, String> {
    let mut found = None;
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("Threads:")
            && found.replace(value.trim()).is_some()
        {
            return Err("status: duplicate Threads".into());
        }
    }
    found
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| "status: no Threads".into())
}

/// The canonical duration argument, bounded by the worker's own limit.
fn duration_ms(text: &str) -> Option<u64> {
    (!text.is_empty() && !text.starts_with('0') && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
        .filter(|&ms| ms <= MAX_DURATION_MS)
}

/// Checks shared by the start and alive observations. Returns the
/// worker's starttime and CPU ticks.
fn check_observation(
    record: &LoadRecord,
    observation: &WorkerObservation,
    runner_starttime: u64,
    online: &crate::shell_timing_environment::CpuList,
    namespace: u64,
) -> Result<(u64, u64), String> {
    let pid = observation.pid;
    if observation.opened_usec > observation.closed_usec {
        return Err(format!("worker {pid}: observation clock moved backward"));
    }
    let first = stat_facts(&observation.stat, pid)?;
    let closing = stat_facts(&observation.closing_stat, pid)?;
    if first.starttime != closing.starttime || first.ppid != closing.ppid {
        return Err(format!("worker {pid}: identity changed during observation"));
    }
    if first.ppid != record.runner.pid || first.starttime < runner_starttime {
        return Err(format!("worker {pid}: not a child of the runner"));
    }
    for state in [first.state, closing.state] {
        if !known_state(state) || matches!(state, 'Z' | 'X' | 'x' | 'T' | 't') {
            return Err(format!("worker {pid}: state {state} is not running"));
        }
    }
    let schedule = scheduling(&observation.stat, pid)?;
    if (schedule.policy, schedule.nice, schedule.rt_priority) != ("normal", 0, 0) {
        return Err(format!("worker {pid}: scheduling is not normal, nice 0"));
    }
    if &affinity(&observation.status, pid)? != online {
        return Err(format!("worker {pid}: affinity is not every online CPU"));
    }
    if threads(&observation.status)? != 1 || observation.tasks != [pid] {
        return Err(format!("worker {pid}: worker is not single-threaded"));
    }
    if observation.children.len() != 1 || !observation.children[0].trim().is_empty() {
        return Err(format!("worker {pid}: worker has descendants"));
    }
    let argv: Vec<&str> = observation
        .cmdline
        .strip_suffix('\0')
        .map(|text| text.split('\0').collect())
        .unwrap_or_default();
    if argv != record.argv {
        return Err(format!(
            "worker {pid}: command line differs from the spawn argv"
        ));
    }
    if observation.exe_link.ends_with(" (deleted)")
        || (observation.exe_dev, observation.exe_ino) != (record.program_dev, record.program_ino)
    {
        return Err(format!("worker {pid}: executable is not the prepared file"));
    }
    if namespace_inode(&observation.time_namespace)? != namespace {
        return Err(format!("worker {pid}: time namespace differs"));
    }
    Ok((first.starttime, first.cpu_ticks))
}

/// Qualify a retained load against its arm and environment.
pub fn qualify_load(record: &LoadRecord, expect: &LoadExpect<'_>) -> Result<VerifiedLoad, String> {
    if record.schema != 1 {
        return Err("unknown load record schema".into());
    }
    if expect.measurement.start_usec >= expect.measurement.end_usec {
        return Err("load measurement interval is empty or inverted".into());
    }
    let [program, command, subcommand, duration] = record.argv.as_slice() else {
        return Err("load argv is not program shell-timing cpu-worker DURATION".into());
    };
    if program != &record.program
        || record.program != expect.program
        || [command.as_str(), subcommand.as_str()] != WORKER_COMMAND
    {
        return Err("load argv is not the prepared cpu-worker".into());
    }
    if duration_ms(duration).is_none_or(|ms| ms < expect.min_duration_ms) {
        return Err("load worker duration does not cover the arm lifecycle".into());
    }

    let runner = &record.runner;
    let runner_first = stat_facts(&runner.stat, runner.pid)?;
    let runner_closing = stat_facts(&runner.closing_stat, runner.pid)?;
    if runner.opened_usec > runner.closed_usec || runner_first.starttime != runner_closing.starttime
    {
        return Err("runner identity changed during observation".into());
    }
    let namespace = namespace_inode(&runner.time_namespace)?;
    if namespace != expect.time_namespace_inode {
        return Err("runner time namespace is not the proven capturer namespace".into());
    }

    if record.online.opened_usec > record.online.closed_usec
        || record.online.text.trim_end() != expect.online.trim_end()
    {
        return Err("load CPU online set differs from the environment".into());
    }
    let online = cpu_list(record.online.text.trim_end())?;

    let stop = &record.stop;
    if stop.schema != 1 {
        return Err("unknown load stop evidence schema".into());
    }
    if expect.workers == 0
        || stop.requested_workers != expect.workers
        || stop.workers.len() != expect.workers
        || record.started.len() != expect.workers
        || record.alive.len() != expect.workers
    {
        return Err("load does not have the declared worker count".into());
    }
    let mut pids = BTreeSet::new();
    let mut latest_start = 0;
    let mut earliest_alive = u64::MAX;
    let mut cpu_ticks = Vec::with_capacity(expect.workers);
    for (index, worker) in stop.workers.iter().enumerate() {
        let pid = worker.pid.ok_or("load worker never started")?;
        if worker.index != index
            || worker.program != *program
            || worker.args != record.argv[1..]
            || !pids.insert(pid)
        {
            return Err(format!("load worker {index} differs from its spawn record"));
        }
        if worker.end_reason != EndReason::StopRequested
            || !worker.reaped
            || worker.signal != Some(SIGTERM)
            || worker.exit_code.is_some()
            || worker.error.is_some()
        {
            return Err(format!("load worker {index} did not end by requested TERM"));
        }
        let (started, alive) = (&record.started[index], &record.alive[index]);
        if started.pid != pid || alive.pid != pid {
            return Err(format!("load worker {index} observations name another pid"));
        }
        let (start_identity, start_ticks) =
            check_observation(record, started, runner_first.starttime, &online, namespace)?;
        let (alive_identity, alive_ticks) =
            check_observation(record, alive, runner_first.starttime, &online, namespace)?;
        if start_identity != alive_identity {
            return Err(format!("load worker {index} pid was reused"));
        }
        let delta = alive_ticks
            .checked_sub(start_ticks)
            .filter(|&ticks| ticks > 0)
            .ok_or_else(|| format!("load worker {index} used no CPU time"))?;
        cpu_ticks.push(delta);
        if started.closed_usec > alive.opened_usec || alive.closed_usec > record.stopped_usec {
            return Err(format!("load worker {index} observations are out of order"));
        }
        latest_start = latest_start.max(started.closed_usec);
        earliest_alive = earliest_alive.min(alive.opened_usec);
    }
    let interval = Interval {
        start_usec: latest_start,
        end_usec: earliest_alive,
    };
    if interval.start_usec > expect.measurement.start_usec
        || interval.end_usec < expect.measurement.end_usec
    {
        return Err("proven load life does not bracket the measurement".into());
    }
    Ok(VerifiedLoad {
        workers: u8::try_from(expect.workers).map_err(|_| "worker count overflow")?,
        interval,
        measurement: expect.measurement,
        worker_cpu_ticks: cpu_ticks,
    })
}
