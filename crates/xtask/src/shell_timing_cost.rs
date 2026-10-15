//! Conservative process-counter bounds, not a B9 acceptance decision.
use crate::shell_timing::Offer;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug)]
struct Counter {
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
    pub allocation_operations: Bounds,
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
    pub scope: &'static str,
    pub frames: Vec<Frame>,
    pub whole_workload: Cost,
}

pub fn for_mode(mode: &str, capture: &str, offers: &[Offer]) -> Result<Option<Report>, String> {
    match mode {
        "rust-global" => measure(capture, capture, offers).map(Some),
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
            if !required.contains(&key) || f.insert(key, value).is_some() {
                return Err("unknown or repeated counter field".into());
            }
        }
        if f.len() != required.len()
            || f["schema"] != "1"
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
        if row.pid == 0 || row.before == 0 || row.after < row.before {
            return Err("invalid counter identity or clock bounds".into());
        }
        if let Some(old) = result.last()
            && (old.pid != row.pid
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
        allocation_operations: range(allocation_operations)?,
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
        scope: "session_and_peer_rust_global_allocator",
        frames,
        whole_workload,
    })
}
