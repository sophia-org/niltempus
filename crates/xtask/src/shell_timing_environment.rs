//! Check that a supplied environment capture states every fact a timing
//! campaign needs, each with its source. This validates supplied text only:
//! it does not open devices, read `/proc` or `/sys`, prove that the capture
//! is authentic, or accept a campaign.
//!
//! # Host records
//!
//! Output and renderer facts come only from records the measured Sophia host
//! writes to its log. Configuration and profiles are never read. Each record
//! is one line: the record name, then space-separated `key=value` fields in
//! any order. Every listed key must appear exactly once and no other key may
//! appear. Numbers are unsigned decimal without sign or leading zeros.
//!
//! ```text
//! sophia_shell_environment_output schema=1 pid=<host pid> output=<id> \
//!     card=<cardN> connector=<name> width=<px> height=<px> \
//!     refresh_millihz=<mHz> scale_milli=<scale x 1000> vrr=<on|off> \
//!     monotonic_usec=<CLOCK_MONOTONIC>
//! sophia_shell_environment_renderer schema=1 pid=<host pid> output=<id> \
//!     card=<cardN> driver=<kernel driver> vendor=<pct> renderer=<pct> \
//!     version=<pct> monotonic_usec=<CLOCK_MONOTONIC>
//! ```
//!
//! The output record states the mode, scale and VRR state the host observed
//! on that output, not the configured ones. The renderer record states the
//! active GL context the host process uses for that output and card: vendor,
//! renderer and version are its `GL_VENDOR`, `GL_RENDERER` and `GL_VERSION`
//! strings. `EGL_VERSION` names only the EGL API and is not accepted as an
//! implementation version. `<pct>` values
//! are percent-encoded: bytes outside `A-Z a-z 0-9 - . _ ~` are written as
//! `%XX` with uppercase hex, unreserved bytes are never encoded, and the
//! decoded value is non-empty UTF-8. Other `card`, `connector` and `driver`
//! values use only `A-Z a-z 0-9 - _`. Lines with other names are ignored,
//! except that any other `sophia_shell_environment_` name is refused.
//!
//! A host with several heads may write records for each. Every environment
//! record is validated. Exactly one output record and one renderer record
//! must carry the campaign's `pid`, `output` and `card`; a second record for
//! that identity fails. Records for other identities cannot satisfy the
//! campaign and are otherwise ignored.
//!
//! All `captured_usec` and `monotonic_usec` values are CLOCK_MONOTONIC
//! microseconds on the measured machine.
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: u64 = 1;
const OUTPUT_RECORD: &str = "sophia_shell_environment_output";
const RENDERER_RECORD: &str = "sophia_shell_environment_renderer";
const RECORD_PREFIX: &str = "sophia_shell_environment_";
const MAX_SOURCES: usize = 4096;
const MAX_TEXT_BYTES: usize = 1 << 20;
/// Highest CPU number accepted in a list, one below the kernel's NR_CPUS
/// ceiling of 8192.
const MAX_CPU: u32 = 8191;
const MAX_COMM_BYTES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceKind {
    /// `/proc/sys/kernel/osrelease`.
    Osrelease,
    /// `/proc/version`.
    ProcVersion,
    /// `/sys/devices/system/cpu/online`.
    CpuOnline,
    /// `/sys/devices/system/cpu/cpu<cpu>/cpufreq/scaling_governor`.
    Governor { cpu: u32 },
    /// `/proc/<pid>/stat`.
    ProcStat { pid: u32 },
    /// `/proc/<pid>/status`.
    ProcStatus { pid: u32 },
    /// Basename of the `/sys/class/drm/<card>/device/driver` link.
    DrmDriver { card: String },
    /// The measured host's log, holding its environment records.
    HostLog,
    /// Free text about the quiet window. Carried through, never parsed, and
    /// never used to satisfy a fact.
    Note,
}

#[derive(Clone, Debug)]
pub struct SourceText {
    pub kind: SourceKind,
    /// Exact path, command line or log name the text was read from.
    pub origin: String,
    pub captured_usec: u64,
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct Capture {
    pub sources: Vec<SourceText>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Interval {
    pub start_usec: u64,
    pub end_usec: u64,
}

impl Interval {
    fn contains(self, usec: u64) -> bool {
        (self.start_usec..=self.end_usec).contains(&usec)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Vrr {
    On,
    Off,
}

/// What the campaign declares. Every observed fact must match it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Expect {
    pub output: u64,
    pub card: String,
    pub connector: String,
    pub width: u32,
    pub height: u32,
    pub refresh_millihz: u64,
    pub scale_milli: u32,
    pub vrr: Vrr,
    pub host_pid: u32,
    pub peer_pid: u32,
    /// Every source must be captured inside this window.
    pub capture: Interval,
    /// The measured run. The capture window must bracket it.
    pub measurement: Interval,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SourceRef {
    #[serde(flatten)]
    pub kind: SourceKind,
    pub origin: String,
    pub captured_usec: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Fact<T> {
    pub value: T,
    pub source: SourceRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Kernel {
    /// From `/proc/sys/kernel/osrelease`.
    pub release: Fact<String>,
    /// From `/proc/version`, which must name the same release.
    pub version: Fact<String>,
}

/// A canonical CPU list: ascending, non-overlapping, non-adjacent ranges.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CpuList {
    pub ranges: Vec<(u32, u32)>,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Governor {
    pub cpu: u32,
    pub governor: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Scheduling {
    /// `/proc/<pid>/stat` describes the process leader thread only. Other
    /// threads may be scheduled differently.
    pub scope: &'static str,
    pub comm: String,
    pub nice: i64,
    pub policy: &'static str,
    pub rt_priority: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Process {
    pub role: &'static str,
    pub pid: u32,
    pub scheduling: Fact<Scheduling>,
    /// `Cpus_allowed_list` from `/proc/<pid>/status`, leader thread only.
    pub affinity: Fact<CpuList>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Output {
    pub pid: u32,
    pub output: u64,
    pub card: String,
    pub connector: String,
    pub width: u32,
    pub height: u32,
    pub refresh_millihz: u64,
    pub scale_milli: u32,
    pub vrr: Vrr,
    pub monotonic_usec: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Renderer {
    pub pid: u32,
    pub output: u64,
    pub card: String,
    pub driver: String,
    pub vendor: String,
    pub renderer: String,
    pub version: String,
    pub monotonic_usec: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Environment {
    pub schema: u64,
    pub verdict: &'static str,
    pub scope: &'static str,
    pub expect: Expect,
    pub kernel: Kernel,
    pub cpus_online: Fact<CpuList>,
    pub governors: Vec<Fact<Governor>>,
    pub processes: Vec<Process>,
    pub drm_driver: Fact<String>,
    pub output: Fact<Output>,
    pub renderer: Fact<Renderer>,
    pub notes: Vec<Fact<String>>,
}

fn source_ref(source: &SourceText) -> SourceRef {
    SourceRef {
        kind: source.kind.clone(),
        origin: source.origin.clone(),
        captured_usec: source.captured_usec,
    }
}

fn fact<T>(value: T, source: &SourceText) -> Fact<T> {
    Fact {
        value,
        source: source_ref(source),
    }
}

/// Check that `capture` states every required fact and that each matches
/// `expect`. Any missing, conflicting, unparsable or mismatched fact fails.
pub fn environment_record(capture: &Capture, expect: &Expect) -> Result<Environment, String> {
    check_expect(expect)?;
    let sources = index(capture, expect)?;
    let one = |kind: &SourceKind| {
        sources
            .get(kind)
            .ok_or_else(|| format!("missing source {kind:?}"))
    };

    let release_source = one(&SourceKind::Osrelease)?;
    let release = single_line(&release_source.text, "osrelease")?;
    if release.is_empty() || release.contains(char::is_whitespace) {
        return Err("osrelease is not one release string".into());
    }
    let version_source = one(&SourceKind::ProcVersion)?;
    let version = single_line(&version_source.text, "/proc/version")?;
    if !version.starts_with(&format!("Linux version {release} ")) {
        return Err("/proc/version does not name the osrelease kernel".into());
    }
    let kernel = Kernel {
        release: fact(release.into(), release_source),
        version: fact(version.into(), version_source),
    };

    let online_source = one(&SourceKind::CpuOnline)?;
    let online = cpu_list(single_line(&online_source.text, "cpu online")?)?;
    let online_cpus: BTreeSet<u32> = online.ranges.iter().flat_map(|&(a, b)| a..=b).collect();
    let governed: BTreeSet<u32> = sources
        .keys()
        .filter_map(|kind| match kind {
            SourceKind::Governor { cpu } => Some(*cpu),
            _ => None,
        })
        .collect();
    if governed != online_cpus {
        return Err("governor sources must cover exactly the online cpus".into());
    }
    let governors = online_cpus
        .iter()
        .map(|&cpu| {
            let source = one(&SourceKind::Governor { cpu })?;
            let governor = single_line(&source.text, "governor")?;
            if governor.is_empty()
                || !governor
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            {
                return Err(format!("cpu {cpu}: governor is not one name"));
            }
            Ok(fact(
                Governor {
                    cpu,
                    governor: governor.into(),
                },
                source,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let cpus_online = fact(online, online_source);

    let processes = [("host", expect.host_pid), ("peer", expect.peer_pid)]
        .into_iter()
        .map(|(role, pid)| {
            let stat = one(&SourceKind::ProcStat { pid })?;
            let status = one(&SourceKind::ProcStatus { pid })?;
            Ok(Process {
                role,
                pid,
                scheduling: fact(scheduling(&stat.text, pid)?, stat),
                affinity: fact(affinity(&status.text, pid)?, status),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if sources.keys().any(|kind| {
        matches!(kind, SourceKind::ProcStat { pid } | SourceKind::ProcStatus { pid }
            if *pid != expect.host_pid && *pid != expect.peer_pid)
    }) {
        return Err("process source for an undeclared pid".into());
    }

    let driver_source = one(&SourceKind::DrmDriver {
        card: expect.card.clone(),
    })?;
    let driver = single_line(&driver_source.text, "drm driver")?;
    if !is_name(driver) {
        return Err("drm driver is not one name".into());
    }
    if sources
        .keys()
        .any(|kind| matches!(kind, SourceKind::DrmDriver { card } if *card != expect.card))
    {
        return Err("drm driver source for an undeclared card".into());
    }
    let drm_driver = fact(driver.to_string(), driver_source);

    let host_log = one(&SourceKind::HostLog)?;
    let (outputs, renderers) = host_records(&host_log.text)?;
    let output = measured(
        outputs,
        expect,
        |o| (o.pid, o.output, &o.card),
        OUTPUT_RECORD,
    )?;
    let renderer = measured(
        renderers,
        expect,
        |r| (r.pid, r.output, &r.card),
        RENDERER_RECORD,
    )?;
    check_output(&output, expect)?;
    check_renderer(&renderer, expect, driver)?;
    for usec in [output.monotonic_usec, renderer.monotonic_usec] {
        if !expect.capture.contains(usec) {
            return Err("host record outside the capture window".into());
        }
    }
    let output = fact(output, host_log);
    let renderer = fact(renderer, host_log);

    let notes = capture
        .sources
        .iter()
        .filter(|source| source.kind == SourceKind::Note)
        .map(|source| fact(source.text.clone(), source))
        .collect();

    Ok(Environment {
        schema: SCHEMA,
        verdict: "environment_complete",
        scope: "supplied facts are complete and consistent; capture authenticity and campaign acceptance are not checked",
        expect: expect.clone(),
        kernel,
        cpus_online,
        governors,
        processes,
        drm_driver,
        output,
        renderer,
        notes,
    })
}

fn check_expect(expect: &Expect) -> Result<(), String> {
    let (capture, measured) = (expect.capture, expect.measurement);
    if measured.start_usec >= measured.end_usec
        || capture.start_usec > measured.start_usec
        || capture.end_usec < measured.end_usec
    {
        return Err("the capture window must bracket a non-empty measurement".into());
    }
    if expect.host_pid == 0 || expect.peer_pid == 0 || expect.host_pid == expect.peer_pid {
        return Err("host and peer pids must be distinct and nonzero".into());
    }
    if !is_name(&expect.card) || !is_name(&expect.connector) {
        return Err("expected card and connector must be names".into());
    }
    if expect.width == 0 || expect.height == 0 || expect.refresh_millihz == 0 {
        return Err("expected mode must be nonzero".into());
    }
    if expect.scale_milli == 0 {
        return Err("expected scale must be nonzero".into());
    }
    Ok(())
}

/// Index sources by kind. Notes may repeat; nothing else may.
fn index<'a>(
    capture: &'a Capture,
    expect: &Expect,
) -> Result<BTreeMap<SourceKind, &'a SourceText>, String> {
    if capture.sources.len() > MAX_SOURCES {
        return Err("too many sources".into());
    }
    let mut sources = BTreeMap::new();
    for source in &capture.sources {
        if source.text.len() > MAX_TEXT_BYTES {
            return Err(format!("{}: source text too large", source.origin));
        }
        if source.origin.is_empty() {
            return Err(format!("{:?}: source has no origin", source.kind));
        }
        if !expect.capture.contains(source.captured_usec) {
            return Err(format!("{}: captured outside the window", source.origin));
        }
        if source.kind == SourceKind::Note {
            continue;
        }
        if sources.insert(source.kind.clone(), source).is_some() {
            return Err(format!("duplicate source {:?}", source.kind));
        }
    }
    Ok(sources)
}

/// The text with at most one trailing newline, which must be one line.
fn single_line<'a>(text: &'a str, what: &str) -> Result<&'a str, String> {
    let line = text.strip_suffix('\n').unwrap_or(text);
    if line.contains(['\n', '\r']) {
        return Err(format!("{what}: expected one line"));
    }
    Ok(line)
}

fn is_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn decimal(value: &str) -> Option<u64> {
    let canonical = value == "0" || (!value.starts_with('0') && !value.is_empty());
    (canonical && value.bytes().all(|b| b.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

/// Parse a kernel CPU list such as `0-3,8,10-11`. Ranges must be ascending,
/// non-overlapping and non-adjacent, a single CPU is written alone rather
/// than as `n-n`, and no CPU may exceed `MAX_CPU`.
pub fn cpu_list(text: &str) -> Result<CpuList, String> {
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    let mut count = 0u32;
    for part in text.split(',') {
        let (low, high, range) = match part.split_once('-') {
            Some((low, high)) => (low, high, true),
            None => (part, part, false),
        };
        let parse = |n: &str| {
            decimal(n)
                .and_then(|n| u32::try_from(n).ok())
                .filter(|&n| n <= MAX_CPU)
                .ok_or_else(|| format!("cpu list {text:?}: bad cpu {n:?}"))
        };
        let (low, high) = (parse(low)?, parse(high)?);
        if low > high || (range && low == high) {
            return Err(format!("cpu list {text:?}: descending or single-cpu range"));
        }
        if let Some(&(_, last)) = ranges.last()
            && low <= last + 1
        {
            return Err(format!(
                "cpu list {text:?}: ranges must ascend without overlap or adjacency"
            ));
        }
        ranges.push((low, high));
        count += high - low + 1;
    }
    Ok(CpuList { ranges, count })
}

fn policy_name(policy: u64) -> Option<&'static str> {
    Some(match policy {
        0 => "normal",
        1 => "fifo",
        2 => "rr",
        3 => "batch",
        5 => "idle",
        6 => "deadline",
        7 => "ext",
        _ => return None,
    })
}

/// Leader-thread scheduling from `/proc/<pid>/stat`. The command name may
/// contain spaces and parentheses, so fields are counted from the last `)`.
fn scheduling(text: &str, pid: u32) -> Result<Scheduling, String> {
    let line = single_line(text, "stat")?;
    let open = line.find(" (").ok_or("stat: no command name")?;
    let close = line.rfind(") ").ok_or("stat: no command name")?;
    if close < open || decimal(&line[..open]) != Some(u64::from(pid)) {
        return Err(format!("stat: not the record for pid {pid}"));
    }
    let comm = &line[open + 2..close];
    if comm.is_empty() || comm.len() > MAX_COMM_BYTES {
        return Err("stat: bad command name".into());
    }
    // Field 3 (state) is index 0 after the command name.
    let fields: Vec<&str> = line[close + 2..].split(' ').collect();
    let field = |n: usize| {
        fields
            .get(n - 3)
            .copied()
            .ok_or_else(|| format!("stat: missing field {n}"))
    };
    let nice: i64 = field(19)?
        .parse()
        .ok()
        .filter(|n| (-20..=19).contains(n))
        .ok_or("stat: bad nice")?;
    let rt_priority = decimal(field(40)?).ok_or("stat: bad rt_priority")?;
    let policy = decimal(field(41)?)
        .and_then(policy_name)
        .ok_or("stat: unknown scheduling policy")?;
    Ok(Scheduling {
        scope: "process_leader_thread",
        comm: comm.into(),
        nice,
        policy,
        rt_priority,
    })
}

/// `Cpus_allowed_list` from `/proc/<pid>/status`, after checking `Pid`.
fn affinity(text: &str, pid: u32) -> Result<CpuList, String> {
    let mut found_pid = None;
    let mut list = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let slot = match key {
            "Pid" => &mut found_pid,
            "Cpus_allowed_list" => &mut list,
            _ => continue,
        };
        if slot.replace(value.trim()).is_some() {
            return Err(format!("status: duplicate {key}"));
        }
    }
    if found_pid.and_then(decimal) != Some(u64::from(pid)) {
        return Err(format!("status: not the record for pid {pid}"));
    }
    cpu_list(list.ok_or("status: no Cpus_allowed_list")?)
}

fn fields<'a>(
    rest: &'a str,
    keys: &[&str],
    record: &str,
) -> Result<BTreeMap<&'a str, &'a str>, String> {
    let mut found = BTreeMap::new();
    for token in rest.split(' ') {
        let (key, value) = token
            .split_once('=')
            .filter(|(_, v)| !v.is_empty())
            .ok_or_else(|| format!("{record}: bad field {token:?}"))?;
        if !keys.contains(&key) || found.insert(key, value).is_some() {
            return Err(format!("{record}: unknown or repeated field {key}"));
        }
    }
    if found.len() != keys.len() {
        return Err(format!("{record}: missing fields"));
    }
    if found["schema"] != "1" {
        return Err(format!("{record}: unsupported schema"));
    }
    Ok(found)
}

fn number<T: TryFrom<u64>>(fields: &BTreeMap<&str, &str>, key: &str) -> Result<T, String> {
    decimal(fields[key])
        .and_then(|n| T::try_from(n).ok())
        .ok_or_else(|| format!("bad number {key}={}", fields[key]))
}

fn name(fields: &BTreeMap<&str, &str>, key: &str) -> Result<String, String> {
    let value = fields[key];
    is_name(value)
        .then(|| value.to_string())
        .ok_or_else(|| format!("bad name {key}={value}"))
}

/// Decode a canonical percent-encoded value.
pub fn percent_decode(value: &str) -> Result<String, String> {
    let unreserved = |b: u8| b.is_ascii_alphanumeric() || b"-._~".contains(&b);
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if unreserved(b) {
            out.push(b);
            i += 1;
            continue;
        }
        let hex = |h: u8| match h {
            b'0'..=b'9' => Some(h - b'0'),
            b'A'..=b'F' => Some(h - b'A' + 10),
            _ => None,
        };
        let decoded = (b == b'%')
            .then(|| Some(hex(*bytes.get(i + 1)?)? << 4 | hex(*bytes.get(i + 2)?)?))
            .flatten()
            .filter(|&d| !unreserved(d))
            .ok_or_else(|| format!("{value:?} is not canonical percent encoding"))?;
        out.push(decoded);
        i += 3;
    }
    match String::from_utf8(out) {
        Ok(s) if !s.is_empty() => Ok(s),
        _ => Err(format!("{value:?} does not decode to non-empty UTF-8")),
    }
}

type HostRecords = (Vec<Output>, Vec<Renderer>);

/// The one record for the campaign's host pid, output and card.
fn measured<T>(
    records: Vec<T>,
    expect: &Expect,
    identity: impl Fn(&T) -> (u32, u64, &String),
    record: &str,
) -> Result<T, String> {
    let wanted = (expect.host_pid, expect.output, &expect.card);
    let mut matching = records.into_iter().filter(|r| identity(r) == wanted);
    match (matching.next(), matching.next()) {
        (Some(found), None) => Ok(found),
        (None, _) => Err(format!(
            "host log has no {record} for the measured pid, output and card"
        )),
        (Some(_), Some(_)) => Err(format!(
            "host log has more than one {record} for the measured pid, output and card"
        )),
    }
}

fn host_records(text: &str) -> Result<HostRecords, String> {
    let (mut outputs, mut renderers) = (Vec::new(), Vec::new());
    for line in text.lines() {
        let (record, rest) = line.split_once(' ').unwrap_or((line, ""));
        if record == OUTPUT_RECORD {
            let f = fields(
                rest,
                &[
                    "schema",
                    "pid",
                    "output",
                    "card",
                    "connector",
                    "width",
                    "height",
                    "refresh_millihz",
                    "scale_milli",
                    "vrr",
                    "monotonic_usec",
                ],
                record,
            )?;
            let vrr = match f["vrr"] {
                "on" => Vrr::On,
                "off" => Vrr::Off,
                other => return Err(format!("{record}: bad vrr {other}")),
            };
            let parsed = Output {
                pid: number(&f, "pid")?,
                output: number(&f, "output")?,
                card: name(&f, "card")?,
                connector: name(&f, "connector")?,
                width: number(&f, "width")?,
                height: number(&f, "height")?,
                refresh_millihz: number(&f, "refresh_millihz")?,
                scale_milli: number(&f, "scale_milli")?,
                vrr,
                monotonic_usec: number(&f, "monotonic_usec")?,
            };
            outputs.push(parsed);
        } else if record == RENDERER_RECORD {
            let f = fields(
                rest,
                &[
                    "schema",
                    "pid",
                    "output",
                    "card",
                    "driver",
                    "vendor",
                    "renderer",
                    "version",
                    "monotonic_usec",
                ],
                record,
            )?;
            let parsed = Renderer {
                pid: number(&f, "pid")?,
                output: number(&f, "output")?,
                card: name(&f, "card")?,
                driver: name(&f, "driver")?,
                vendor: percent_decode(f["vendor"])?,
                renderer: percent_decode(f["renderer"])?,
                version: percent_decode(f["version"])?,
                monotonic_usec: number(&f, "monotonic_usec")?,
            };
            renderers.push(parsed);
        } else if record.starts_with(RECORD_PREFIX) {
            return Err(format!("unknown host record {record}"));
        }
    }
    Ok((outputs, renderers))
}

fn check_output(output: &Output, expect: &Expect) -> Result<(), String> {
    let mismatches = [
        ("pid", output.pid != expect.host_pid),
        ("output", output.output != expect.output),
        ("card", output.card != expect.card),
        ("connector", output.connector != expect.connector),
        ("width", output.width != expect.width),
        ("height", output.height != expect.height),
        (
            "refresh_millihz",
            output.refresh_millihz != expect.refresh_millihz,
        ),
        ("scale_milli", output.scale_milli != expect.scale_milli),
        ("vrr", output.vrr != expect.vrr),
    ];
    match mismatches.iter().find(|(_, differs)| *differs) {
        Some((field, _)) => Err(format!("output record {field} differs from the campaign")),
        None => Ok(()),
    }
}

fn check_renderer(renderer: &Renderer, expect: &Expect, driver: &str) -> Result<(), String> {
    if renderer.pid != expect.host_pid {
        return Err("renderer record is not from the measured host".into());
    }
    if renderer.output != expect.output || renderer.card != expect.card {
        return Err("renderer record is for another output or card".into());
    }
    if renderer.driver != driver {
        return Err("renderer driver differs from the card's kernel driver".into());
    }
    Ok(())
}
