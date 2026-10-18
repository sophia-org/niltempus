//! Capture the environment facts a timing campaign needs from supplied
//! filesystem roots, as [`SourceText`] values the supplied-capture checker
//! in [`crate::shell_timing_environment`] can validate.
//!
//! The default roots are `/proc` and `/sys`; tests supply fixture roots.
//! Nothing here opens a device, launches a process, or reads a live host
//! unless the caller constructs the system roots and calls a capture.
//! Readers are bounded: a source larger than its limit is refused, never
//! truncated into an accepted value.
//!
//! Kernel release, CPU online set, governors and process affinity can all
//! change during a run. `capture_static` is therefore callable at the
//! start and at the end of a run and the runner compares the two samples;
//! the interval between samples is unobserved and no stability is claimed
//! from any one sample. Each source is stamped with a direct
//! `CLOCK_MONOTONIC` read taken when its read completes.
//!
//! Process identity is `(pid, start_time_ticks)` from `/proc/<pid>/stat`
//! field 22, never the command name. `capture_process` brackets the status
//! read with two stat reads and requires the same identity from both, so a
//! pid reused during the capture cannot qualify. The runner supplies the
//! pids of the supervised host and peer; this helper binds a pid to the
//! process that held it at capture time and makes no ancestry claim.
//! `recheck` distinguishes a pid that is gone from a read that is
//! malformed or denied.
use crate::shell_timing_environment::{Capture, SourceKind, SourceText, cpu_list};
use serde::Serialize;
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

/// Limit for kernel, version, online and governor one-line sources.
pub const MAX_SMALL_BYTES: u64 = 4096;
/// Limit for `/proc/<pid>/stat` and `/proc/<pid>/status`.
pub const MAX_PROCESS_BYTES: u64 = 65536;

/// Filesystem roots to read from. `CaptureRoots::system()` is `/proc` and
/// `/sys`; tests build fixture trees elsewhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureRoots {
    pub proc_root: PathBuf,
    pub sys_root: PathBuf,
}

impl CaptureRoots {
    pub fn new(proc_root: PathBuf, sys_root: PathBuf) -> Self {
        Self {
            proc_root,
            sys_root,
        }
    }

    pub fn system() -> Self {
        Self::new(PathBuf::from("/proc"), PathBuf::from("/sys"))
    }
}

/// One captured process, pinned to the identity that held its pid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ProcessIdentity {
    pub pid: u32,
    /// `/proc/<pid>/stat` field 22: process start time in clock ticks
    /// since boot. With the pid this distinguishes a reused pid.
    pub start_time_ticks: u64,
}

/// Stat facts this helper uses: identity plus the field 3 state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StatFacts {
    identity: ProcessIdentity,
    state: char,
}

fn ended(state: char) -> bool {
    matches!(state, 'Z' | 'X' | 'x')
}

/// The state letters Linux writes in stat field 3. Anything else is a
/// malformed record, never a live process.
pub(crate) fn known_state(state: char) -> bool {
    matches!(
        state,
        'R' | 'S' | 'D' | 'T' | 't' | 'Z' | 'X' | 'x' | 'K' | 'W' | 'P' | 'I'
    )
}

/// The stat and status sources for one process, plus the identity both
/// stat reads agreed on. `closing_stat` is the second, bracketing stat
/// read: it is evidence for the bracket and stays here as a sidecar, so
/// source lists built from `stat`/`status` keep one stat per pid.
#[derive(Clone, Debug)]
pub struct ProcessCapture {
    pub identity: ProcessIdentity,
    pub stat: SourceText,
    pub status: SourceText,
    pub closing_stat: SourceText,
}

/// Result of re-reading a previously captured identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessRecheck {
    /// The same `(pid, start_time_ticks)` is still there and not ended.
    Alive(ProcessIdentity),
    /// The pid is gone, or the same identity is in an ended state
    /// (zombie `Z` or dead `X`/`x`): an unreaped zombie is ended, not
    /// alive.
    Exited,
}

/// One measurement capture: static host facts plus the host and peer
/// process bindings, ready to extend with a host-log source and hand to
/// `environment_record`. `closing_stats` holds the bracketing second stat
/// reads (host then peer) as sidecar evidence; they are not in `sources`,
/// so each pid keeps exactly one stat source there.
#[derive(Clone, Debug)]
pub struct MeasurementCapture {
    pub sources: Vec<SourceText>,
    pub host: ProcessIdentity,
    pub peer: ProcessIdentity,
    pub closing_stats: Vec<SourceText>,
}

impl MeasurementCapture {
    pub fn into_capture(self) -> Capture {
        Capture {
            sources: self.sources,
        }
    }
}

/// A capturer over fixed filesystem roots. Cheap to construct; each
/// capture performs its own bounded reads and clock reads.
#[derive(Clone, Debug)]
pub struct Capturer {
    roots: CaptureRoots,
}

pub(crate) fn monotonic_usec() -> Result<u64, String> {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    let seconds = u64::try_from(now.tv_sec).map_err(|_| "monotonic clock before boot")?;
    let nanos = u64::try_from(now.tv_nsec).map_err(|_| "monotonic clock nanoseconds")?;
    seconds
        .checked_mul(1_000_000)
        .and_then(|usec| usec.checked_add(nanos / 1000))
        .ok_or_else(|| "monotonic clock overflow".into())
}

/// Read at most `limit` bytes; more is an error, never a truncation.
fn read_bounded(path: &Path, limit: u64) -> Result<String, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{}: exceeds {limit} byte bound", path.display()));
    }
    String::from_utf8(bytes).map_err(|_| format!("{}: not UTF-8", path.display()))
}

fn source(kind: SourceKind, path: &Path, limit: u64) -> Result<SourceText, String> {
    let text = read_bounded(path, limit)?;
    Ok(SourceText {
        kind,
        origin: path.display().to_string(),
        captured_usec: monotonic_usec()?,
        text,
    })
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Identity and state from a stat line. The command name may contain
/// spaces and parentheses, so fields are counted from the last `)`.
/// Field 3 is the state and field 22 (index 19 after the command name)
/// is the start time.
fn stat_facts(text: &str, expected_pid: u32) -> Result<StatFacts, String> {
    let line = text.strip_suffix('\n').unwrap_or(text);
    if line.contains('\n') {
        return Err("stat: expected one line".into());
    }
    let open = line.find(" (").ok_or("stat: no command name")?;
    let close = line.rfind(") ").ok_or("stat: no command name")?;
    if close < open {
        return Err("stat: malformed command name".into());
    }
    let pid: u32 = line[..open]
        .parse()
        .map_err(|_| "stat: bad pid".to_string())?;
    if pid != expected_pid {
        return Err(format!(
            "stat: record is for pid {pid}, not requested pid {expected_pid}"
        ));
    }
    let fields: Vec<&str> = line[close + 2..].split(' ').collect();
    let state = fields
        .first()
        .copied()
        .filter(|state| state.len() == 1)
        .and_then(|state| state.chars().next())
        .filter(|&state| known_state(state))
        .ok_or("stat: unknown state field")?;
    let start_time_ticks: u64 = fields
        .get(22 - 3)
        .copied()
        .ok_or("stat: missing start_time field")?
        .parse()
        .map_err(|_| "stat: bad start_time".to_string())?;
    Ok(StatFacts {
        identity: ProcessIdentity {
            pid,
            start_time_ticks,
        },
        state,
    })
}

impl Capturer {
    pub fn new(roots: CaptureRoots) -> Self {
        Self { roots }
    }

    pub fn system() -> Self {
        Self::new(CaptureRoots::system())
    }

    pub fn roots(&self) -> &CaptureRoots {
        &self.roots
    }

    fn proc_path(&self, relative: &str) -> PathBuf {
        self.roots.proc_root.join(relative)
    }

    fn sys_path(&self, relative: &str) -> PathBuf {
        self.roots.sys_root.join(relative)
    }

    /// Kernel, CPU online set, one governor per online CPU, and the DRM
    /// driver basename for `card`. Callable at run start and end; the
    /// runner compares samples. Governors must exist for exactly the
    /// online CPUs or the capture is refused here.
    pub fn capture_static(&self, card: &str) -> Result<Vec<SourceText>, String> {
        if !valid_name(card) || card.contains('.') {
            return Err(format!("invalid drm card name {card:?}"));
        }
        let mut sources = vec![
            source(
                SourceKind::Osrelease,
                &self.proc_path("sys/kernel/osrelease"),
                MAX_SMALL_BYTES,
            )?,
            source(
                SourceKind::ProcVersion,
                &self.proc_path("version"),
                MAX_SMALL_BYTES,
            )?,
        ];
        let online = source(
            SourceKind::CpuOnline,
            &self.sys_path("devices/system/cpu/online"),
            MAX_SMALL_BYTES,
        )?;
        let online_text = online.text.trim_end_matches(['\n', '\r']);
        let cpus = cpu_list(online_text)?;
        sources.push(online);
        for &(low, high) in &cpus.ranges {
            for cpu in low..=high {
                sources.push(source(
                    SourceKind::Governor { cpu },
                    &self.sys_path(&format!(
                        "devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor"
                    )),
                    MAX_SMALL_BYTES,
                )?);
            }
        }
        let driver_link = self.sys_path(&format!("class/drm/{card}/device/driver"));
        let target =
            fs::read_link(&driver_link).map_err(|e| format!("{}: {e}", driver_link.display()))?;
        let driver = target
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| valid_name(name))
            .ok_or_else(|| format!("{}: driver link has no name", driver_link.display()))?;
        sources.push(SourceText {
            kind: SourceKind::DrmDriver { card: card.into() },
            origin: driver_link.display().to_string(),
            captured_usec: monotonic_usec()?,
            text: format!("{driver}\n"),
        });
        Ok(sources)
    }

    /// Stat and status for `pid`, bracketed by two stat reads that must
    /// agree on `(pid, start_time_ticks)`. The returned stat source is the
    /// first read; the second read is retained as `closing_stat` sidecar
    /// evidence. A process already in an ended state (zombie or dead) is
    /// refused: presence of its stat is not liveness.
    pub fn capture_process(&self, pid: u32) -> Result<ProcessCapture, String> {
        if pid == 0 {
            return Err("pid must be nonzero".into());
        }
        let stat_path = self.proc_path(&format!("{pid}/stat"));
        let status_path = self.proc_path(&format!("{pid}/status"));
        let stat = source(SourceKind::ProcStat { pid }, &stat_path, MAX_PROCESS_BYTES)?;
        let first = stat_facts(&stat.text, pid)?;
        if ended(first.state) {
            return Err(format!(
                "pid {pid} is in ended state {} at capture",
                first.state
            ));
        }
        let identity = first.identity;
        let status = source(
            SourceKind::ProcStatus { pid },
            &status_path,
            MAX_PROCESS_BYTES,
        )?;
        let closing_stat = source(SourceKind::ProcStat { pid }, &stat_path, MAX_PROCESS_BYTES)?;
        let closing = stat_facts(&closing_stat.text, pid)?;
        if closing.identity != identity {
            return Err(format!(
                "pid {pid} identity changed during capture (start_time {} then {}); \
                 the bracketed reads may name different processes",
                identity.start_time_ticks, closing.identity.start_time_ticks
            ));
        }
        if ended(closing.state) {
            return Err(format!(
                "pid {pid} is in ended state {} at capture",
                closing.state
            ));
        }
        Ok(ProcessCapture {
            identity,
            stat,
            status,
            closing_stat,
        })
    }

    /// Re-read the stat for a captured identity. Absence, or the same
    /// identity in an ended state (zombie or dead), is `Exited`; a
    /// different start time, malformed text or a denied read is an error,
    /// so none of them can be reported as a clean exit and a zombie left
    /// unreaped never qualifies as alive.
    pub fn recheck(&self, identity: &ProcessIdentity) -> Result<ProcessRecheck, String> {
        let stat_path = self.proc_path(&format!("{}/stat", identity.pid));
        let file = match fs::File::open(&stat_path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(ProcessRecheck::Exited);
            }
            Err(e) => return Err(format!("{}: {e}", stat_path.display())),
        };
        let mut bytes = Vec::new();
        file.take(MAX_PROCESS_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{}: {e}", stat_path.display()))?;
        if bytes.len() as u64 > MAX_PROCESS_BYTES {
            return Err(format!(
                "{}: exceeds {MAX_PROCESS_BYTES} byte bound",
                stat_path.display()
            ));
        }
        let text =
            String::from_utf8(bytes).map_err(|_| format!("{}: not UTF-8", stat_path.display()))?;
        let current =
            stat_facts(&text, identity.pid).map_err(|e| format!("{}: {e}", stat_path.display()))?;
        if current.identity != *identity {
            return Err(format!(
                "pid {} was reused: start_time {} replaced {}",
                identity.pid, current.identity.start_time_ticks, identity.start_time_ticks
            ));
        }
        if ended(current.state) {
            return Ok(ProcessRecheck::Exited);
        }
        Ok(ProcessRecheck::Alive(current.identity))
    }

    /// Static facts plus host and peer process bindings for one run. The
    /// two pids are supplied by the runner from its supervised processes;
    /// this helper binds each pid to the process that held it at capture
    /// time and makes no ancestry claim.
    pub fn capture_measurement(
        &self,
        card: &str,
        host_pid: u32,
        peer_pid: u32,
    ) -> Result<MeasurementCapture, String> {
        if host_pid == peer_pid {
            return Err("host and peer pids must differ".into());
        }
        let mut sources = self.capture_static(card)?;
        let host = self.capture_process(host_pid)?;
        let peer = self.capture_process(peer_pid)?;
        let (host_identity, peer_identity) = (host.identity, peer.identity);
        let closing_stats = vec![host.closing_stat.clone(), peer.closing_stat.clone()];
        sources.push(host.stat);
        sources.push(host.status);
        sources.push(peer.stat);
        sources.push(peer.status);
        Ok(MeasurementCapture {
            sources,
            host: host_identity,
            peer: peer_identity,
            closing_stats,
        })
    }
}
