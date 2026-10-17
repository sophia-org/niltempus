//! Diagnostic counter validation. Counts are process totals, not CPU costs.
use serde::Serialize;
use std::collections::BTreeMap;

pub const NAMES: [&str; 30] = [
    "owner_pass",
    "component_pass",
    "server_poll_turn",
    "server_flush",
    "handoff_accepted",
    "handoff_fallback",
    "read_bytes",
    "written_bytes",
    "replies_encoded",
    "pending_read_attempt",
    "peer_content_poll",
    "peer_wait",
    "relay_dispatch",
    "relay_idle",
    "relay_expiry",
    "request_walk",
    "request_open",
    "request_read",
    "request_write",
    "request_clunk",
    "owner_completed",
    "owner_setup",
    "owner_record",
    "owner_submit",
    "owner_pixel",
    "owner_cadence",
    "owner_flush_only",
    "owner_other",
    "owner_unattributed",
    "owner_mixed",
];

#[derive(Clone, Serialize)]
pub struct Sample {
    pub process: String,
    pub pid: u64,
    pub sequence: u64,
    pub before_usec: u64,
    pub after_usec: u64,
    pub counts: Vec<u64>,
}

pub fn collect(text: &str) -> Result<Vec<Sample>, String> {
    let mut samples = Vec::new();
    let mut previous: BTreeMap<String, Sample> = BTreeMap::new();
    let mut schema = None;
    for line in text.lines() {
        if line.contains("sophia_comparator_counters ")
            && !line.split_whitespace().any(|s| s == "mode=progress")
        {
            return Err("mixed counter instrumentation in progress capture".into());
        }
        let Some(at) = line.find("sophia_comparator_progress ") else {
            continue;
        };
        let mut fields = BTreeMap::new();
        for token in line[at..].split_whitespace().skip(1) {
            let (key, value) = token.split_once('=').ok_or("malformed progress field")?;
            if fields.insert(key, value).is_some() {
                return Err("duplicate progress field".into());
            }
        }
        let count = match fields.get("schema") {
            Some(&"1") => 12,
            Some(&"2") => 20,
            Some(&"3") => NAMES.len(),
            _ => return Err("unsupported progress schema".into()),
        };
        if fields.len() != count + 7 || fields.get("mode") != Some(&"progress") {
            return Err("unsupported progress schema or fields".into());
        }
        if schema.is_some_and(|old| old != count) {
            return Err("mixed progress schemas".into());
        }
        schema = Some(count);
        let number = |key: &str| {
            fields
                .get(key)
                .ok_or("missing progress field")?
                .parse::<u64>()
                .map_err(|_| "invalid progress number")
        };
        let process = *fields.get("process").ok_or("missing process")?;
        if !matches!(process, "session" | "peer") {
            return Err("unknown progress process".into());
        }
        let mut counts = vec![0; count];
        for (i, name) in NAMES[..count].iter().enumerate() {
            counts[i] = number(name)?;
        }
        let sample = Sample {
            process: process.into(),
            pid: number("pid")?,
            sequence: number("sequence")?,
            before_usec: number("before_usec")?,
            after_usec: number("after_usec")?,
            counts,
        };
        if sample.pid == 0 || sample.before_usec > sample.after_usec {
            return Err("invalid progress identity or interval".into());
        }
        if let Some(old) = previous.get(process)
            && (old.pid != sample.pid
                || old.sequence > sample.sequence
                || old.after_usec > sample.before_usec
                || old.counts.iter().zip(&sample.counts).any(|(a, b)| a > b))
        {
            return Err("progress counters reset, wrap or go backwards".into());
        }
        previous.insert(process.into(), sample.clone());
        samples.push(sample);
        if samples.len() > 20_000 {
            return Err("too many progress samples".into());
        }
    }
    if previous.len() != 2 {
        return Err("missing session or peer progress".into());
    }
    Ok(samples)
}
