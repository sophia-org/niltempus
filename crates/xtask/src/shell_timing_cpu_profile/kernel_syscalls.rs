//! Explicit kernel+user CPU sampling with syscall-entry counts. Diagnostic
//! only: tracepoints and higher-rate sampling perturb the measured run, and
//! nothing here is a cost or latency verdict.
use super::time_filter;
use crate::shell_timing_environment::Interval;
use std::collections::BTreeMap;
use std::path::Path;

pub const KERNEL_SYSCALLS_SCOPE: &str = "diagnostic kernel+user CPU samples and syscall-entry counts only; no cost or latency gate verdict; sampling and tracepoints perturb execution";

/// The CPU event carries its own DWARF stack request. No global call-graph
/// option exists, so no stack collection reaches the syscall tracepoints.
pub const CPU_EVENT: &str = "cpu-clock/freq=997,call-graph=dwarf,stack-size=8192/";

/// Fixed entry tracepoints. All `syscalls:sys_enter_*` events (366 on the
/// reference kernel) times every CPU exceeds the 4096 descriptor limit.
pub const SYSCALLS: [&str; 26] = [
    "read",
    "write",
    "readv",
    "writev",
    "pread64",
    "pwrite64",
    "recvmsg",
    "sendmsg",
    "recvfrom",
    "sendto",
    "poll",
    "ppoll",
    "epoll_wait",
    "epoll_pwait",
    "mmap",
    "munmap",
    "mprotect",
    "memfd_create",
    "fcntl",
    "ioctl",
    "futex",
    "close",
    "openat",
    "newfstatat",
    "sched_yield",
    "clock_nanosleep",
];

/// Tracepoints only accept `call-graph` and `stack-size` terms; their
/// default period of one is asserted from the recorded attributes instead.
const SYSCALL_TERMS: &str = "/call-graph=no/";

fn syscall_event(name: &str) -> String {
    format!("syscalls:sys_enter_{name}{SYSCALL_TERMS}")
}

/// The syscall a recorded event name denotes, with or without perf echoing
/// the configuration terms back.
fn syscall_of(event: &str) -> Option<&'static str> {
    let event = event.strip_suffix(SYSCALL_TERMS).unwrap_or(event);
    let name = event.strip_prefix("syscalls:sys_enter_")?;
    SYSCALLS.iter().copied().find(|known| *known == name)
}

pub fn kernel_record_arguments(
    data: &Path,
    runner: &Path,
    descriptor: &Path,
    sha: &str,
    stage: &Path,
) -> Vec<String> {
    let mut args = vec!["record".to_string(), format!("--event={CPU_EVENT}")];
    args.extend(
        SYSCALLS
            .iter()
            .map(|name| format!("--event={}", syscall_event(name))),
    );
    args.extend([
        "--clockid=mono".into(),
        "--timestamp".into(),
        "--output".into(),
        data.display().to_string(),
        "--".into(),
        runner.display().to_string(),
        "shell-timing".into(),
        "capture-cpu-profile".into(),
        descriptor.display().to_string(),
        sha.into(),
        stage.display().to_string(),
    ]);
    args
}

pub fn evlist_arguments(data: &Path) -> Vec<String> {
    vec![
        "evlist".into(),
        "-v".into(),
        "-i".into(),
        data.display().to_string(),
    ]
}

/// `perf evlist -v` line attribute, e.g. `sample_type` or `freq`.
fn attribute<'a>(attributes: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}: ");
    let start = if attributes.starts_with(&prefix) {
        0
    } else {
        attributes.find(&format!(", {prefix}"))? + 2
    };
    let value = &attributes[start + prefix.len()..];
    Some(value.split(", ").next().unwrap_or(value))
}

/// The recorded attributes, not the requested arguments, decide whether
/// the capture is what this mode claims: the CPU event samples kernel and
/// user at 997 Hz with an 8192-byte user stack, and every syscall event
/// samples each entry with no callchain, registers or stack.
pub fn check_evlist(text: &str) -> Result<(), String> {
    let mut cpu = 0;
    let mut seen = BTreeMap::new();
    // perf appends a `# Tip:` comment line.
    for line in text
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
    {
        let (event, attributes) = line
            .split_once(": type: ")
            .ok_or("unrecognised perf evlist line")?;
        let attributes = format!("type: {attributes}");
        let get = |key| attribute(&attributes, key);
        let excluded = |key| get(key).is_some_and(|value| value != "0");
        if excluded("exclude_kernel") || excluded("exclude_user") || get("inherit") != Some("1") {
            return Err(format!("{event} excludes a domain or is not inherited"));
        }
        let sample_type = get("sample_type")
            .unwrap_or_default()
            .split('|')
            .collect::<Vec<_>>();
        if event == CPU_EVENT {
            if get("type") != Some("1 (PERF_TYPE_SOFTWARE)")
                || get("config") != Some("0 (PERF_COUNT_SW_CPU_CLOCK)")
                || get("{ sample_period, sample_freq }") != Some("997")
                || get("freq") != Some("1")
                || get("sample_stack_user") != Some("8192")
                || !["CALLCHAIN", "REGS_USER", "STACK_USER"]
                    .iter()
                    .all(|bit| sample_type.contains(bit))
            {
                return Err("CPU event attributes differ from the kernel-syscalls mode".into());
            }
            cpu += 1;
        } else if let Some(name) = syscall_of(event) {
            if get("type") != Some("2 (PERF_TYPE_TRACEPOINT)")
                || get("{ sample_period, sample_freq }") != Some("1")
                || excluded("freq")
                || get("sample_stack_user").is_some()
                || ["CALLCHAIN", "REGS_USER", "STACK_USER"]
                    .iter()
                    .any(|bit| sample_type.contains(bit))
            {
                return Err(format!("{event} is not a period-one stackless tracepoint"));
            }
            *seen.entry(name).or_insert(0) += 1;
        } else {
            return Err(format!("unexpected recorded event {event}"));
        }
    }
    if cpu != 1 || seen.len() != SYSCALLS.len() || seen.values().any(|count| *count != 1) {
        return Err("recorded events differ from the kernel-syscalls set".into());
    }
    Ok(())
}

/// The CPU event's histogram section. Syscall sections follow it in the same
/// report and are counted from raw samples instead. A `--time` filter makes
/// perf append ` (time slices: ...)` to each section header.
pub fn cpu_report_section(text: &str) -> Result<&str, String> {
    let mut sections = text.split("\n# Samples: ").skip(1).filter(|section| {
        section
            .lines()
            .next()
            .and_then(|line| line.split_once(" of event '"))
            .and_then(|(_, event)| event.split_once('\''))
            .is_some_and(|(event, _)| event == CPU_EVENT)
    });
    let section = sections.next().ok_or("missing CPU event report section")?;
    if sections.next().is_some() {
        return Err("duplicated CPU event report section".into());
    }
    Ok(section)
}

/// perf warns on stderr when ring-buffer chunks were lost; the header's lost
/// sample count does not include them.
pub fn check_lost_chunks(stderr: &str) -> Result<(), String> {
    if stderr.to_ascii_lowercase().contains("lost") {
        return Err("perf reported lost events".into());
    }
    Ok(())
}

pub fn kernel_script_arguments(
    data: &Path,
    pid: u32,
    interval: &Interval,
) -> Result<Vec<String>, String> {
    if pid == 0 {
        return Err("invalid profile process".into());
    }
    Ok(vec![
        "script".into(),
        "--no-inline".into(),
        "-G".into(),
        "--ns".into(),
        "--fields=pid,time,event,ip,sym,dso".into(),
        format!("--pid={pid}"),
        format!("--time={}", time_filter(interval)?),
        "--input".into(),
        data.display().to_string(),
    ])
}

#[derive(Debug, PartialEq, Eq)]
pub struct KernelSampleCounts {
    pub cpu_samples: usize,
    pub unknown_leaf_samples: usize,
    /// Every listed syscall, including those never entered.
    pub syscall_entries: BTreeMap<&'static str, usize>,
}

/// Count one outer TGID's samples inside the exact monotonic interval,
/// independently of perf's filters. CPU samples and each syscall's entries
/// are kept apart; any other event refuses the input.
pub fn kernel_sample_counts(
    text: &str,
    pid: u32,
    interval: &Interval,
) -> Result<KernelSampleCounts, String> {
    time_filter(interval)?;
    if pid == 0 || text.len() > 256 * super::CAP as usize {
        return Err("invalid profile sample input".into());
    }
    let mut counts = KernelSampleCounts {
        cpu_samples: 0,
        unknown_leaf_samples: 0,
        syscall_entries: SYSCALLS.iter().map(|name| (*name, 0)).collect(),
    };
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 3 || fields[0].parse::<u32>().ok() != Some(pid) {
            return Err("perf sample identity differs".into());
        }
        super::check_sample_time(fields[1], interval)?;
        let event = fields[2]
            .strip_suffix(':')
            .ok_or("invalid perf sample event")?;
        if event == CPU_EVENT {
            if fields.len() < 6 {
                return Err("perf CPU sample lacks its leaf".into());
            }
            counts.cpu_samples += 1;
            counts.unknown_leaf_samples += usize::from(line.contains("[unknown]"));
        } else if let Some(name) = syscall_of(event) {
            *counts.syscall_entries.get_mut(name).unwrap() += 1;
        } else {
            return Err("perf sample event differs".into());
        }
    }
    if counts.cpu_samples == 0 {
        return Err("no CPU samples for the bound process in the measured interval".into());
    }
    Ok(counts)
}
