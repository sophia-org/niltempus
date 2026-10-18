//! Trusted-runner provenance for retained campaign arms.
//!
//! This provider vouches only for evidence produced by the trusted
//! owned runner (`CampaignObserver` + `EnvironmentMonitor`) and
//! retained under the attempt's recorded digests. Deserializing
//! arbitrary bytes is never authentication, and an adversarial
//! operator who can fabricate runner evidence is out of scope. What
//! is checked here is internal consistency against the frozen
//! manifest and the frozen per-arm plan: identities, executable
//! paths, brackets, receipt coverage and clock domain are all
//! re-derived from the retained raw texts (stat, status, namespace
//! links, `timens_offsets`, fdinfo), never from a summary's say-so,
//! and never from a session log.
use crate::{
    shell_timing_campaign::Manifest,
    shell_timing_campaign_import::{
        ArmFiles, ArmProvenance, MonotonicDomain, VerifiedEnvironment, VerifiedLoad,
    },
    shell_timing_environment::{Capture, Interval, SourceKind, SourceText, Vrr},
    shell_timing_environment_monitor::{
        BindingRecord, MonitorEvidence, MonitorOutcome, NamespaceRecord, RecheckRecord,
        SourceKindRecord, SourceRecord,
    },
    shell_timing_environment_socket::EnvironmentReceipt,
    shell_timing_load_qualify::{LoadExpect, LoadRecord, qualify_load},
    shell_timing_native::Request,
};
use serde::Deserialize;
use std::{collections::BTreeMap, path::Path};

/// The frozen plan's relevant shape. Only the existing request/argv
/// form is tolerated; parsing here performs no I/O.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanView {
    schema: u8,
    request: Request,
    argv: Vec<String>,
}

fn plan_artifacts(plan_bytes: &[u8]) -> Result<(String, String, String), String> {
    let plan: PlanView = serde_json::from_slice(plan_bytes).map_err(|e| e.to_string())?;
    if plan.schema != 1 || plan.argv.is_empty() || plan.argv.iter().any(|arg| arg.is_empty()) {
        return Err("frozen native plan has an unknown schema or argv".into());
    }
    if plan
        .argv
        .iter()
        .any(|arg| arg == "--bubblewrap" || arg.starts_with("--bubblewrap="))
    {
        return Err("environment binding requires the default bubblewrap executable".into());
    }
    let artifact = |name: &str| -> Result<String, String> {
        plan.request
            .artifacts
            .get(name)
            .ok_or_else(|| format!("frozen native plan lacks the {name} artifact"))
            .and_then(|artifact| {
                artifact
                    .path
                    .to_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{name} artifact path is not UTF-8"))
            })
    };
    Ok((artifact("sophia")?, artifact("peer")?, artifact("runner")?))
}

/// Provider bound to one frozen campaign manifest.
pub struct TrustedRunnerProvenance {
    manifest: Manifest,
}

impl TrustedRunnerProvenance {
    pub fn new(manifest: Manifest) -> Result<Self, String> {
        manifest.validate()?;
        Ok(Self { manifest })
    }

    fn plan(&self, files: &ArmFiles<'_>) -> Result<(String, String, String), String> {
        plan_artifacts(files.plan)
    }
}

/// The environment target a retained arm must have been measured
/// against. The campaign provider derives it from the frozen
/// manifest; calibration callers supply it directly.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceTarget {
    pub card: String,
    pub output: u64,
    pub scale_milli: u32,
    pub connector: String,
    pub width: u32,
    pub height: u32,
    pub mode_millihz: u64,
    pub vrr: bool,
}

impl ProvenanceTarget {
    pub fn validate(&self) -> Result<(), String> {
        let card = self.card.strip_prefix("card");
        if !card.is_some_and(|digits| {
            digits
                .parse::<u32>()
                .is_ok_and(|id| id.to_string() == digits)
        }) || self.output == 0
            || self.scale_milli == 0
            || !self.scale_milli.is_multiple_of(1000)
            || self.connector.is_empty()
            || self.connector.len() > 96
            || !self
                .connector
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
            || self.width != 1920
            || self.height != 1080
            || self.mode_millihz != 120_000
            || self.vrr
        {
            return Err(
                "target requires a canonical card, nonzero output, whole scale, \
                 a valid connector and 1920x1080@120 Hz with VRR off"
                    .into(),
            );
        }
        Ok(())
    }

    fn from_manifest(manifest: &Manifest) -> Self {
        Self {
            card: manifest.environment.card.clone(),
            output: manifest.environment.output,
            scale_milli: manifest.environment.scale_milli,
            connector: manifest.connector.clone(),
            width: manifest.width,
            height: manifest.height,
            mode_millihz: manifest.mode_millihz,
            vrr: manifest.vrr,
        }
    }
}

fn parse_stat(text: &str, expected_pid: u32) -> Result<(u32, u64, char), String> {
    let line = text.strip_suffix('\n').unwrap_or(text);
    if line.contains('\n') {
        return Err("stat source is not one line".into());
    }
    let open = line.find(" (").ok_or("stat source has no command name")?;
    let close = line.rfind(") ").ok_or("stat source has no command name")?;
    let pid: u32 = line[..open]
        .parse()
        .map_err(|_| "stat source has a bad pid".to_string())?;
    if pid != expected_pid {
        return Err("stat source names another pid".into());
    }
    let fields: Vec<&str> = line[close + 2..].split(' ').collect();
    let state = fields
        .first()
        .copied()
        .filter(|state| state.len() == 1)
        .and_then(|state| state.chars().next())
        .filter(|state| {
            matches!(
                state,
                'R' | 'S' | 'D' | 'T' | 't' | 'Z' | 'X' | 'x' | 'K' | 'W' | 'P' | 'I'
            )
        })
        .ok_or("stat source has an unknown state")?;
    if matches!(state, 'Z' | 'X' | 'x') {
        return Err("stat source is in an ended state".into());
    }
    let parent: u32 = fields
        .get(1)
        .copied()
        .ok_or("stat source lacks ppid")?
        .parse()
        .map_err(|_| "stat source has a bad ppid".to_string())?;
    let start: u64 = fields
        .get(22 - 3)
        .copied()
        .ok_or("stat source lacks start_time")?
        .parse()
        .map_err(|_| "stat source has a bad start_time".to_string())?;
    Ok((parent, start, state))
}

fn parse_status_id(status: &str, name: &str) -> Result<u32, String> {
    let line = status
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{name}:")))
        .ok_or_else(|| format!("status source has no {name} line"))?;
    let values: Vec<&str> = line.split_whitespace().collect();
    if values.len() != 4 {
        return Err(format!("status {name} line must carry four ids"));
    }
    let parsed: Vec<u32> = values
        .iter()
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|_| format!("status {name} id is not numeric"))
        })
        .collect::<Result<_, _>>()?;
    Ok(parsed[0])
}

fn parse_namespace(record: &NamespaceRecord) -> Result<u64, String> {
    let inode: u64 = record
        .observed_target
        .strip_prefix("time:[")
        .and_then(|rest| rest.strip_suffix(']'))
        .and_then(|digits| digits.parse().ok())
        .ok_or("namespace evidence target is not a time namespace link")?;
    if inode == 0 || inode != record.time_namespace_inode {
        return Err("namespace inode summary differs from its raw link target".into());
    }
    Ok(inode)
}

fn parse_offsets(text: &str) -> Result<(i64, i64), String> {
    let mut monotonic: Option<(i64, i64)> = None;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        if parts.next() == Some("monotonic") {
            if monotonic.is_some() {
                return Err("timens_offsets has a duplicate monotonic line".into());
            }
            let seconds: i64 = parts
                .next()
                .and_then(|value| value.parse().ok())
                .ok_or("timens_offsets has bad monotonic seconds")?;
            let nanos: i64 = parts
                .next()
                .and_then(|value| value.parse().ok())
                .ok_or("timens_offsets has bad monotonic nanoseconds")?;
            if parts.next().is_some() {
                return Err("timens_offsets monotonic line has trailing fields".into());
            }
            monotonic = Some((seconds, nanos));
        }
    }
    monotonic.ok_or_else(|| "timens_offsets has no monotonic line".into())
}

fn fdinfo_pid(fdinfo: &str) -> Result<u32, String> {
    let ids: Vec<&str> = fdinfo
        .lines()
        .filter_map(|line| line.strip_prefix("Pid:"))
        .collect();
    if ids.len() != 1 {
        return Err("sender fdinfo must carry exactly one Pid line".into());
    }
    ids[0]
        .trim()
        .parse::<u32>()
        .map_err(|_| "sender fdinfo Pid is not a live pid".into())
}

fn is_static(kind: &SourceKindRecord) -> bool {
    matches!(
        kind,
        SourceKindRecord::Osrelease
            | SourceKindRecord::ProcVersion
            | SourceKindRecord::CpuOnline
            | SourceKindRecord::Governor { .. }
            | SourceKindRecord::DrmDriver { .. }
    )
}

fn source_kind(record: &SourceKindRecord) -> SourceKind {
    match record {
        SourceKindRecord::Osrelease => SourceKind::Osrelease,
        SourceKindRecord::ProcVersion => SourceKind::ProcVersion,
        SourceKindRecord::CpuOnline => SourceKind::CpuOnline,
        SourceKindRecord::Governor { cpu } => SourceKind::Governor { cpu: *cpu },
        SourceKindRecord::ProcStat { pid } => SourceKind::ProcStat { pid: *pid },
        SourceKindRecord::ProcStatus { pid } => SourceKind::ProcStatus { pid: *pid },
        SourceKindRecord::DrmDriver { card } => SourceKind::DrmDriver { card: card.clone() },
        SourceKindRecord::HostLog => SourceKind::HostLog,
        SourceKindRecord::Note => SourceKind::Note,
    }
}

fn static_map(sources: &[SourceRecord]) -> BTreeMap<String, String> {
    sources
        .iter()
        .filter(|source| is_static(&source.kind))
        .map(|source| (format!("{:?}", source.kind), source.text.clone()))
        .collect()
}

/// Verify one retained arm's environment against `target`. This is
/// the campaign provider's environment implementation, reusable for
/// target-bound calibration; every check is identical.
pub fn verify_environment(
    target: &ProvenanceTarget,
    files: &ArmFiles<'_>,
) -> Result<VerifiedEnvironment, String> {
    target.validate()?;
    let (host_path, peer_path, _runner_path) = plan_artifacts(files.plan)?;
    let evidence: MonitorEvidence =
        serde_json::from_slice(files.environment).map_err(|e| e.to_string())?;
    if evidence.schema != 1 {
        return Err("unknown monitor evidence schema".into());
    }
    if evidence.receipts.len() > 64 || evidence.batches.len() > 64 {
        return Err("environment evidence exceeds the receipt or batch bound".into());
    }
    let receipts_file: Vec<EnvironmentReceipt> =
        serde_json::from_slice(files.environment_receipts).map_err(|e| e.to_string())?;
    if evidence.finish_error.is_some() {
        return Err("environment evidence carries a finish error".into());
    }
    if !matches!(
        evidence.outcome,
        MonitorOutcome::Bound | MonitorOutcome::EndedAfterEvidence
    ) {
        return Err("environment outcome cannot qualify".into());
    }
    if receipts_file.len() != evidence.receipts.len()
        || receipts_file
            .iter()
            .zip(&evidence.receipts)
            .any(|(file, retained)| {
                file.pid != retained.pid
                    || file.uid != retained.uid
                    || file.gid != retained.gid
                    || file.received_usec != retained.received_usec
                    || file.record != retained.record
                    || file.sender_fdinfo != retained.sender_fdinfo
            })
    {
        return Err("environment receipts file differs from the monitor evidence".into());
    }
    if evidence.card != target.card {
        return Err("monitor card differs from the frozen campaign".into());
    }
    if !Path::new(&evidence.environment_socket).is_absolute() {
        return Err("environment socket path is not absolute".into());
    }
    let capture_window = evidence
        .capture_window
        .ok_or("environment evidence has no capture window")?;
    if capture_window.start_usec > capture_window.end_usec {
        return Err("capture window is inverted".into());
    }
    let binding: &BindingRecord = evidence.binding.as_ref().ok_or("environment never bound")?;
    let plan: PlanView = serde_json::from_slice(files.plan).map_err(|e| e.to_string())?;
    if binding.host_argv != plan.argv {
        return Err("bound host arguments differ from the frozen plan".into());
    }
    if binding.uniqueness != crate::shell_timing_process_binding::UNIQUENESS_SCOPE {
        return Err("binding uniqueness scope is unknown".into());
    }

    // Binding chain and identities.
    let chain = &binding.chain;
    if chain.len() < 4
        || chain.first().map(|node| node.identity) != Some(binding.wrapper)
        || chain.last().map(|node| node.identity) != Some(binding.peer)
    {
        return Err("binding chain does not run from wrapper to peer".into());
    }
    let host_at = chain
        .iter()
        .position(|node| node.identity == binding.host)
        .ok_or("binding chain lacks the host")?;
    let peer_at = chain
        .iter()
        .position(|node| node.identity == binding.peer)
        .ok_or("binding chain lacks the peer")?;
    if host_at == 0 || peer_at <= host_at {
        return Err("binding host is not below the wrapper and peer below host".into());
    }
    if peer_at != host_at + 2 || chain[host_at + 1].identity != binding.supervisor {
        return Err("binding must contain host, supervisor and peer as direct children".into());
    }
    let mut chain_pids = std::collections::BTreeSet::new();
    if chain
        .iter()
        .any(|node| node.identity.pid == 0 || !chain_pids.insert(node.identity.pid))
    {
        return Err("binding chain repeats a pid or contains zero".into());
    }
    let mut opaque_pids = std::collections::BTreeSet::new();
    if binding.unexamined.len() > crate::shell_timing_process_binding::MAX_NODES
        || binding.unexamined.iter().any(|node| {
            node.identity.pid == 0
                || chain_pids.contains(&node.identity.pid)
                || !opaque_pids.insert(node.identity.pid)
        })
    {
        return Err("unexamined nodes overlap the binding or repeat a pid".into());
    }
    if binding
        .unexamined
        .iter()
        .any(|node| chain_pids.contains(&node.parent_pid) && node.parent_pid != binding.host.pid)
    {
        return Err("unexamined node has a forbidden parent in the binding chain".into());
    }
    for pair in chain.windows(2) {
        if pair[1].parent_pid != pair[0].identity.pid {
            return Err("binding chain parent links are not contiguous".into());
        }
    }
    if binding.wrapper.pid == binding.host.pid
        || binding.host.pid == binding.peer.pid
        || binding.wrapper.pid == binding.peer.pid
    {
        return Err("binding pids are not distinct".into());
    }
    for (record, expected, role) in [
        (&binding.host_executable, &host_path, "host"),
        (
            &binding.supervisor_executable,
            &"/usr/bin/bwrap".to_owned(),
            "supervisor",
        ),
        (&binding.peer_executable, &peer_path, "peer"),
    ] {
        if record.prepared_path != *expected
            || record.dev == 0
            || record.ino == 0
            || !Path::new(&record.observed_link).is_absolute()
            || record.observed_link.ends_with(" (deleted)")
        {
            return Err(format!(
                "bound {role} executable differs from the frozen plan"
            ));
        }
    }

    // Clock domain, reparsed from the raw retained texts.
    let time = &binding.time;
    let inodes = [
        time.reference.inode,
        parse_namespace(&time.host)?,
        parse_namespace(&time.peer)?,
        parse_namespace(&time.capturer)?,
    ];
    if inodes[0] != crate::shell_timing_process_binding::INITIAL_TIME_NAMESPACE_INODE
        || inodes.iter().any(|&inode| inode != inodes[0])
    {
        return Err("time namespace inodes differ".into());
    }
    if (time.host.pid, time.peer.pid, time.capturer.pid)
        != (binding.host.pid, binding.peer.pid, time.capturer.pid)
        || time.host.identity != binding.host
        || time.peer.identity != binding.peer
    {
        return Err("namespace evidence names other processes".into());
    }
    for record in [&time.host, &time.peer, &time.capturer] {
        if record.pid != record.identity.pid
            || record.origin.is_empty()
            || !(capture_window.start_usec..=capture_window.end_usec)
                .contains(&record.captured_usec)
        {
            return Err("namespace evidence provenance is inconsistent".into());
        }
    }
    if time.capturer.identity.pid != time.capturer.pid
        || time.capturer_timens_offsets_origin.is_empty()
    {
        return Err("capturer namespace evidence is unprovenanced".into());
    }
    let offset = parse_offsets(&time.capturer_timens_offsets)?;
    if offset != (0, 0)
        || offset
            != (
                time.monotonic_offset_seconds,
                time.monotonic_offset_nanoseconds,
            )
    {
        return Err("timens_offsets monotonic offset is not exactly zero".into());
    }

    // Proven-alive bound and measurement end.
    let proven = evidence
        .last_proven_alive
        .ok_or("environment has no proven-alive bracket")?;
    if proven.host_start_time_ticks != binding.host.start_time_ticks
        || proven.peer_start_time_ticks != binding.peer.start_time_ticks
        || proven.interval.start_usec > proven.interval.end_usec
        || proven.interval.start_usec < capture_window.start_usec
        || proven.interval.end_usec > capture_window.end_usec
        || proven.interval.start_usec <= capture_window.start_usec
    {
        return Err("proven-alive bracket is inconsistent".into());
    }

    // Start evidence: identities, raw stat/status, windows.
    let start = evidence.start.as_ref().ok_or("environment has no start")?;
    if start.host != binding.host || start.peer != binding.peer {
        return Err("start capture identities differ from the binding".into());
    }
    if start.window.start_usec < capture_window.start_usec
        || start.window.end_usec > capture_window.end_usec
        || start.window.start_usec > start.window.end_usec
    {
        return Err("start window lies outside the capture window".into());
    }
    let mut seen: Vec<&SourceKindRecord> = Vec::new();
    for source in &start.sources {
        if seen.contains(&&source.kind) {
            return Err("start sources duplicate a kind".into());
        }
        seen.push(&source.kind);
        if matches!(
            source.kind,
            SourceKindRecord::HostLog | SourceKindRecord::Note
        ) {
            return Err("start sources carry a raw-log kind".into());
        }
        if !(start.window.start_usec..=start.window.end_usec).contains(&source.captured_usec) {
            return Err("start source stamp lies outside its window".into());
        }
    }
    let one = |kind: &SourceKindRecord| -> Result<&SourceRecord, String> {
        start
            .sources
            .iter()
            .find(|source| &source.kind == kind)
            .ok_or_else(|| format!("start sources lack {kind:?}"))
    };
    let (host_parent, host_start, _) = {
        let stat = one(&SourceKindRecord::ProcStat {
            pid: binding.host.pid,
        })?;
        parse_stat(&stat.text, binding.host.pid)?
    };
    let (peer_parent, peer_start, _) = {
        let stat = one(&SourceKindRecord::ProcStat {
            pid: binding.peer.pid,
        })?;
        parse_stat(&stat.text, binding.peer.pid)?
    };
    if host_start != binding.host.start_time_ticks
        || peer_start != binding.peer.start_time_ticks
        || chain[host_at].parent_pid != host_parent
        || chain[peer_at].parent_pid != peer_parent
    {
        return Err("start stat facts differ from the binding".into());
    }
    let host_status = one(&SourceKindRecord::ProcStatus {
        pid: binding.host.pid,
    })?;
    one(&SourceKindRecord::ProcStatus {
        pid: binding.peer.pid,
    })?;
    let (uid, gid) = (evidence.host_uid, evidence.host_gid);
    let (uid, gid) = match (uid, gid) {
        (Some(uid), Some(gid)) => (uid, gid),
        _ => return Err("environment lacks host uid/gid".into()),
    };
    if parse_status_id(&host_status.text, "Uid")? != uid
        || parse_status_id(&host_status.text, "Gid")? != gid
    {
        return Err("host status real ids differ from the receipt metadata".into());
    }
    if start.closing_stats.len() != 2
        || start.closing_stats[0].kind
            != (SourceKindRecord::ProcStat {
                pid: binding.host.pid,
            })
        || start.closing_stats[1].kind
            != (SourceKindRecord::ProcStat {
                pid: binding.peer.pid,
            })
    {
        return Err("closing stats are not the host and peer brackets".into());
    }
    for (closing, (expected, opening_parent)) in start
        .closing_stats
        .iter()
        .zip([(binding.host, host_parent), (binding.peer, peer_parent)])
    {
        let (parent, start_ticks, _) = parse_stat(&closing.text, expected.pid)?;
        if start_ticks != expected.start_time_ticks
            || parent != opening_parent
            || !(start.window.start_usec..=start.window.end_usec).contains(&closing.captured_usec)
        {
            return Err("closing stat differs from the bound identity".into());
        }
    }

    // End evidence: static equality and honest rechecks.
    let end = &evidence.end;
    let end_window = end.window.ok_or("environment end has no window")?;
    if end_window.start_usec > end_window.end_usec
        || end_window.start_usec < capture_window.start_usec
        || end_window.end_usec > capture_window.end_usec
    {
        return Err("end window lies outside the capture window".into());
    }
    if end_window.start_usec < proven.interval.end_usec {
        return Err("end window does not follow the proven-alive bracket".into());
    }
    let end_sources = end
        .static_sources
        .as_ref()
        .map_err(|error| format!("end static capture failed: {error}"))?;
    let mut end_seen: Vec<&SourceKindRecord> = Vec::new();
    for source in end_sources {
        if end_seen.contains(&&source.kind) {
            return Err("end sources duplicate a kind".into());
        }
        end_seen.push(&source.kind);
        if !is_static(&source.kind)
            || matches!(
                source.kind,
                SourceKindRecord::HostLog | SourceKindRecord::Note
            )
            || !(end_window.start_usec..=end_window.end_usec).contains(&source.captured_usec)
        {
            return Err("end sources are not windowed static facts".into());
        }
    }
    if static_map(&start.sources) != static_map(end_sources) {
        return Err("static environment changed between start and end".into());
    }
    for (recheck, expected, role) in [
        (&end.host_recheck, &binding.host, "host"),
        (&end.peer_recheck, &binding.peer, "peer"),
    ] {
        match recheck {
            Some(RecheckRecord::Alive(identity)) if identity == expected => {}
            Some(RecheckRecord::Exited) => {}
            _ => {
                return Err(format!(
                    "end {role} recheck is missing or does not match the binding"
                ));
            }
        }
    }

    // Batches cover every receipt exactly once, in order.
    if evidence.receipts.is_empty() || evidence.batches.is_empty() {
        return Err("environment accepted no receipts".into());
    }
    let mut covered = 0usize;
    let mut previous_end = 0u64;
    for batch in &evidence.batches {
        let batch_end_receipt = batch
            .first_receipt
            .checked_add(batch.receipt_count)
            .ok_or("accepted batch coverage overflows")?;
        if batch.first_receipt != covered
            || batch.receipt_count == 0
            || batch_end_receipt > evidence.receipts.len()
            || batch.interval.start_usec > batch.interval.end_usec
            || batch.interval.start_usec < previous_end
            || batch.interval.start_usec < start.window.end_usec
            || batch.interval.end_usec > proven.interval.end_usec
            || !(capture_window.start_usec..=capture_window.end_usec)
                .contains(&batch.interval.start_usec)
            || !(capture_window.start_usec..=capture_window.end_usec)
                .contains(&batch.interval.end_usec)
            || batch.host_start_time_ticks != binding.host.start_time_ticks
            || batch.peer_start_time_ticks != binding.peer.start_time_ticks
        {
            return Err("accepted batches do not cover the receipts contiguously".into());
        }
        previous_end = batch.interval.end_usec;
        for receipt in
            &evidence.receipts[batch.first_receipt..batch.first_receipt + batch.receipt_count]
        {
            receipt.require_host(binding.host.pid, uid, gid)?;
            if !(batch.interval.start_usec..=batch.interval.end_usec)
                .contains(&receipt.received_usec)
            {
                return Err("receipt lies outside its batch interval".into());
            }
            if fdinfo_pid(&receipt.sender_fdinfo)? != binding.host.pid {
                return Err("receipt pidfd does not name the bound host".into());
            }
        }
        covered += batch.receipt_count;
    }
    if covered != evidence.receipts.len() {
        return Err("accepted batches do not cover every receipt".into());
    }

    // Canonical capture: start sources plus the receipt-derived
    // host log, and nothing else.
    let mut sources: Vec<SourceText> = start
        .sources
        .iter()
        .map(|source| SourceText {
            kind: source_kind(&source.kind),
            origin: source.origin.clone(),
            captured_usec: source.captured_usec,
            text: source.text.clone(),
        })
        .collect();
    let host_log = evidence
        .receipts
        .iter()
        .map(|receipt| receipt.record.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    sources.push(SourceText {
        kind: SourceKind::HostLog,
        origin: evidence.environment_socket.clone(),
        captured_usec: evidence.batches.last().unwrap().interval.end_usec,
        text: format!("{host_log}\n"),
    });
    Ok(VerifiedEnvironment {
        capture: Capture { sources },
        expect: crate::shell_timing_environment::Expect {
            output: target.output,
            card: target.card.clone(),
            connector: target.connector.clone(),
            width: target.width,
            height: target.height,
            refresh_millihz: target.mode_millihz,
            scale_milli: target.scale_milli,
            vrr: if target.vrr { Vrr::On } else { Vrr::Off },
            host_pid: binding.host.pid,
            peer_pid: binding.peer.pid,
            capture: Interval {
                start_usec: capture_window.start_usec,
                end_usec: capture_window.end_usec,
            },
            measurement: Interval {
                start_usec: capture_window.start_usec,
                end_usec: proven.interval.start_usec,
            },
        },
        host_starttime: binding.host.start_time_ticks,
        peer_starttime: binding.peer.start_time_ticks,
        clock: MonotonicDomain {
            reference: inodes[0],
            host: inodes[1],
            peer: inodes[2],
            capturer: inodes[3],
            monotonic_offset_seconds: offset.0,
            monotonic_offset_nanoseconds: offset.1,
        },
    })
}

impl ArmProvenance for TrustedRunnerProvenance {
    fn environment(&self, files: &ArmFiles<'_>) -> Result<VerifiedEnvironment, String> {
        verify_environment(&ProvenanceTarget::from_manifest(&self.manifest), files)
    }

    fn load(
        &self,
        files: &ArmFiles<'_>,
        environment: &VerifiedEnvironment,
    ) -> Result<VerifiedLoad, String> {
        let bytes = files.load.ok_or("load arm has no load record")?;
        let record: LoadRecord = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        // The load observer is the same process that captured the
        // environment: tie the record's runner identity to the
        // environment binding's capturer from the same checked bytes
        // (qualify_load alone only checks the namespace).
        let evidence: MonitorEvidence =
            serde_json::from_slice(files.environment).map_err(|e| e.to_string())?;
        if evidence.schema != 1 {
            return Err("unknown monitor evidence schema".into());
        }
        let binding = evidence.binding.as_ref().ok_or("environment never bound")?;
        let capturer = binding.time.capturer.identity;
        let (_, runner_open_start, _) = parse_stat(&record.runner.stat, record.runner.pid)?;
        let (_, runner_close_start, _) =
            parse_stat(&record.runner.closing_stat, record.runner.pid)?;
        if record.runner.pid != capturer.pid
            || runner_open_start != capturer.start_time_ticks
            || runner_close_start != capturer.start_time_ticks
        {
            return Err("load runner is not the environment capturer".into());
        }
        let (_host, _peer, runner) = self.plan(files)?;
        let online = environment
            .capture
            .sources
            .iter()
            .find(|source| source.kind == SourceKind::CpuOnline)
            .ok_or("verified capture has no cpu online source")?;
        qualify_load(
            &record,
            &LoadExpect {
                workers: usize::from(self.manifest.load_workers),
                program: &runner,
                min_duration_ms: files.arm.runtime_ms,
                measurement: environment.expect.measurement,
                online: online.text.trim_end_matches(['\n', '\r']),
                time_namespace_inode: environment.clock.capturer,
            },
        )
    }
}
