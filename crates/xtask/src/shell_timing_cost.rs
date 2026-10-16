//! Conservative process-counter bounds, not a B9 acceptance decision.
use crate::shell_timing::Offer;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug)]
struct Counter {
    cpu_only: bool,
    profiling: bool,
    pid: u64,
    stage: String,
    sequence: u64,
    before: u64,
    after: u64,
    cpu: u64,
    allocations: u64,
    reallocations: u64,
    deallocations: u64,
    bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct Bounds {
    pub lower: u64,
    pub upper: u64,
}

#[derive(Debug, Serialize)]
pub struct Cost {
    pub allocation_operations: Option<Bounds>,
    pub process_cpu_ns: Bounds,
    pub outer_start_usec: u64,
    pub outer_end_usec: u64,
}

#[derive(Debug, Serialize)]
pub struct Frame {
    pub generation: u64,
    pub cost: Cost,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub instrumentation: &'static str,
    pub scope: &'static str,
    pub cpu_scope: &'static str,
    pub allocation_scope: &'static str,
    pub frames: Vec<Frame>,
    pub whole_workload: Cost,
    pub normalized: Option<Normalized>,
}

#[derive(Debug, Serialize)]
pub struct Normalized {
    pub frame_count: u64,
    pub uploaded_bytes: u64,
    pub presentation_bytes: u64,
    pub transfer_probe_bytes: u64,
    pub cpu_ns_per_uploaded_mib: Bounds,
    pub allocation_operations_per_frame: Option<Bounds>,
}

// Round outward, so normalization cannot turn an inconclusive bound into a pass.
fn normalized(bounds: &Bounds, multiplier: u64, denominator: u64) -> Result<Bounds, String> {
    if denominator == 0 {
        return Err("zero normalization denominator".into());
    }
    let scale = |value: u64, ceil: bool| -> Result<u64, String> {
        let numerator = u128::from(value) * u128::from(multiplier);
        let divisor = u128::from(denominator);
        u64::try_from(if ceil {
            numerator.div_ceil(divisor)
        } else {
            numerator / divisor
        })
        .map_err(|_| "normalized count overflow".into())
    };
    Ok(Bounds {
        lower: scale(bounds.lower, false)?,
        upper: scale(bounds.upper, true)?,
    })
}

/// Byte normalization requires complete upload evidence, not inferred image sizes.
pub fn measure_journal(
    host_log: &str,
    peer_log: &str,
    journal: &str,
    expected: usize,
) -> Result<Report, String> {
    let metrics = crate::shell_timing::collect_metrics(journal, expected)?;
    if metrics.schema != 2 {
        return Err("byte-normalized costs require schema 2 upload evidence".into());
    }
    let mut report = measure(host_log, peer_log, &metrics.offers)?;
    let peer = counters(peer_log, "peer")?;
    let mut presentation_bytes = 0u64;
    let mut transfer_probe_bytes = 0u64;
    for (index, permit) in metrics.permits.iter().enumerate() {
        if permit.demand_usec < peer[index * 2].after
            || metrics.terminal_usec[index] > peer[index * 2 + 1].before
        {
            return Err("permit or terminal lies outside its frame counter interval".into());
        }
    }
    for accepted in &metrics.uploads {
        let upload = &accepted.upload;
        let index = (upload.candidate_generation - 1) as usize;
        let start = &peer[index * 2];
        let end = &peer[index * 2 + 1];
        if upload.begin_usec < start.after
            || accepted.accepted_usec > end.before
            || accepted.released_usec.is_some_and(|time| time > end.before)
        {
            return Err("upload lies outside its frame counter interval".into());
        }
        let total = if accepted.purpose == "transfer_probe" {
            &mut transfer_probe_bytes
        } else {
            &mut presentation_bytes
        };
        *total = total
            .checked_add(upload.total_bytes)
            .ok_or("uploaded byte count overflow")?;
    }
    let uploaded_bytes = presentation_bytes
        .checked_add(transfer_probe_bytes)
        .ok_or("uploaded byte count overflow")?;
    report.normalized = Some(Normalized {
        frame_count: expected as u64,
        uploaded_bytes,
        presentation_bytes,
        transfer_probe_bytes,
        cpu_ns_per_uploaded_mib: normalized(
            &report.whole_workload.process_cpu_ns,
            1048576,
            uploaded_bytes,
        )?,
        allocation_operations_per_frame: report
            .whole_workload
            .allocation_operations
            .as_ref()
            .map(|bounds| normalized(bounds, 1, expected as u64))
            .transpose()?,
    });
    Ok(report)
}

pub fn for_mode(mode: &str, capture: &str, offers: &[Offer]) -> Result<Option<Report>, String> {
    match mode {
        "rust-global" | "cpu-only" | "profile" => {
            let report = measure(capture, capture, offers)?;
            if report.instrumentation != mode {
                return Err("counter mode differs from prepared instrumentation".into());
            }
            Ok(Some(report))
        }
        "none" if !capture.contains("sophia_comparator_counters ") => Ok(None),
        "none" => Err("instrumented capture cannot supply an uninstrumented timing run".into()),
        _ => Err("unknown instrumentation mode".into()),
    }
}

fn counters(log: &str, process: &str) -> Result<Vec<Counter>, String> {
    if log.len() > 64 * 1024 * 1024 || !log.ends_with('\n') {
        return Err("counter log oversized or truncated".into());
    }
    let mut result = Vec::<Counter>::new();
    let required = [
        "schema",
        "process",
        "pid",
        "stage",
        "sequence",
        "before_usec",
        "after_usec",
        "cpu_ns",
        "allocations",
        "reallocations",
        "deallocations",
        "bytes_allocated",
    ];
    for line in log.lines() {
        let Some(at) = line.find("sophia_comparator_counters ") else {
            continue;
        };
        let mut f = BTreeMap::new();
        for word in line[at..].split_whitespace().skip(1) {
            let (key, value) = word.split_once('=').ok_or("malformed counter record")?;
            if (!required.contains(&key) && key != "mode") || f.insert(key, value).is_some() {
                return Err("unknown or repeated counter field".into());
            }
        }
        let cpu_only = f.get("schema") == Some(&"2") && f.get("mode") == Some(&"cpu-only");
        let profiling = f.get("schema") == Some(&"2") && f.get("mode") == Some(&"profile");
        if !required.iter().all(|key| f.contains_key(key))
            || f.len() != required.len() + usize::from(cpu_only || profiling)
            || (!cpu_only && !profiling && f.get("schema") != Some(&"1"))
            || !matches!(f["process"], "session" | "peer")
        {
            return Err("invalid counter schema or process".into());
        }
        if f["process"] != process {
            continue;
        }
        let n = |key: &str| -> Result<u64, String> {
            let text = f[key];
            if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("invalid {key}"));
            }
            text.parse().map_err(|_| format!("overflowing {key}"))
        };
        let row = Counter {
            cpu_only,
            profiling,
            pid: n("pid")?,
            stage: f["stage"].into(),
            sequence: n("sequence")?,
            before: n("before_usec")?,
            after: n("after_usec")?,
            cpu: n("cpu_ns")?,
            allocations: n("allocations")?,
            reallocations: n("reallocations")?,
            deallocations: n("deallocations")?,
            bytes: n("bytes_allocated")?,
        };
        if cpu_only
            && (row.allocations != 0
                || row.reallocations != 0
                || row.deallocations != 0
                || row.bytes != 0)
        {
            return Err("CPU-only record cannot claim allocation counts".into());
        }
        if row.pid == 0 || row.before == 0 || row.after < row.before {
            return Err("invalid counter identity or clock bounds".into());
        }
        if let Some(old) = result.last()
            && (old.cpu_only != row.cpu_only
                || old.profiling != row.profiling
                || old.pid != row.pid
                || old.after > row.before
                || old.cpu > row.cpu
                || old.allocations > row.allocations
                || old.reallocations > row.reallocations
                || old.deallocations > row.deallocations
                || old.bytes > row.bytes)
        {
            return Err("counter process changed or counters/clock moved backward".into());
        }
        result.push(row);
        if result.len() > 60_002 {
            return Err("counter record bound exceeded".into());
        }
    }
    Ok(result)
}

fn allocation_operations(c: &Counter) -> Result<u64, String> {
    c.allocations
        .checked_add(c.reallocations)
        .ok_or_else(|| "allocation count overflow".into())
}

fn cost(host: &[Counter], start: &Counter, end: &Counter) -> Result<Cost, String> {
    let outer_start = host
        .partition_point(|s| s.after <= start.before)
        .checked_sub(1)
        .ok_or("no host snapshot wholly before client window")?;
    let outer_end = host.partition_point(|s| s.before < end.after);
    if outer_end == host.len() {
        return Err("no host snapshot wholly after client window".into());
    }
    let inner_start = host.partition_point(|s| s.before < start.after);
    let inner_end = host
        .partition_point(|s| s.after <= end.before)
        .checked_sub(1);
    let range = |value: fn(&Counter) -> Result<u64, String>| -> Result<Bounds, String> {
        let peer = value(end)?
            .checked_sub(value(start)?)
            .ok_or("backward peer count")?;
        let outer = value(&host[outer_end])?
            .checked_sub(value(&host[outer_start])?)
            .ok_or("backward host count")?;
        let inner = match inner_end {
            Some(last) if inner_start < last => value(&host[last])?
                .checked_sub(value(&host[inner_start])?)
                .ok_or("backward inner count")?,
            _ => 0,
        };
        Ok(Bounds {
            lower: peer.checked_add(inner).ok_or("lower count overflow")?,
            upper: peer.checked_add(outer).ok_or("upper count overflow")?,
        })
    };
    Ok(Cost {
        allocation_operations: if start.cpu_only {
            None
        } else {
            Some(range(allocation_operations)?)
        },
        process_cpu_ns: range(|c| Ok(c.cpu))?,
        outer_start_usec: host[outer_start].before,
        outer_end_usec: host[outer_end].after,
    })
}

/// Every counter frame must correspond to an offered candidate. Call the native
/// completion join separately; counters cannot attest physical presentation.
pub fn measure(host_log: &str, peer_log: &str, offers: &[Offer]) -> Result<Report, String> {
    if offers.is_empty() || offers.len() > 10_000 {
        return Err("invalid offered frame count".into());
    }
    let host = counters(host_log, "session")?;
    let peer = counters(peer_log, "peer")?;
    if host.len() < 2
        || host.first().is_none_or(|s| s.stage != "start")
        || host.last().is_none_or(|s| s.stage != "end")
        || peer.len() != offers.len() * 2
    {
        return Err("incomplete host or peer counter lifecycle".into());
    }
    for (index, row) in host.iter().enumerate() {
        if row.sequence != index as u64
            || (index > 0 && index + 1 < host.len() && row.stage != "sample")
        {
            return Err("missing, duplicate or exhausted host samples".into());
        }
    }
    if peer[0].pid == host[0].pid {
        return Err("peer and Session must be distinct processes".into());
    }
    if peer[0].cpu_only != host[0].cpu_only || peer[0].profiling != host[0].profiling {
        return Err("Session and peer use different counter modes".into());
    }
    let mut frames = Vec::with_capacity(offers.len());
    for (index, offer) in offers.iter().enumerate() {
        let start = &peer[index * 2];
        let end = &peer[index * 2 + 1];
        let generation = index as u64 + 1;
        if start.stage != "frame_start"
            || end.stage != "frame_end"
            || start.sequence != generation
            || end.sequence != generation
            || offer.candidate_generation != generation
            || offer.outcome != "presented"
            || offer.submitted_monotonic_usec < start.after
            || offer.submitted_monotonic_usec > end.before
        {
            return Err("counter frame differs from complete offered workload".into());
        }
        frames.push(Frame {
            generation,
            cost: cost(&host, start, end)?,
        });
    }
    let whole_workload = cost(&host, &peer[0], peer.last().unwrap())?;
    Ok(Report {
        instrumentation: if host[0].cpu_only {
            "cpu-only"
        } else if host[0].profiling {
            "profile"
        } else {
            "rust-global"
        },
        scope: if host[0].cpu_only {
            "session_and_peer_process_cpu"
        } else {
            "session_and_peer_rust_global_allocator"
        },
        cpu_scope: "all_threads_in_session_and_peer_processes_including_foreign_code_excluding_other_processes",
        allocation_scope: if host[0].cpu_only {
            "not_measured"
        } else {
            "session_and_peer_rust_global_allocations_and_reallocations_excluding_foreign_allocators"
        },
        frames,
        whole_workload,
        normalized: None,
    })
}
