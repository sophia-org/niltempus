//! Diagnostic counter validation. Counts are process totals, not CPU costs.
use serde::Serialize;
use std::collections::BTreeMap;

pub const NAMES: [&str; 12] = [
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
];

#[derive(Clone, Serialize)]
pub struct Sample {
    pub process: String,
    pub pid: u64,
    pub sequence: u64,
    pub before_usec: u64,
    pub after_usec: u64,
    pub counts: [u64; 12],
}

pub fn collect(text: &str) -> Result<Vec<Sample>, String> {
    let mut samples = Vec::new();
    let mut previous: BTreeMap<String, Sample> = BTreeMap::new();
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
        if fields.len() != 19
            || fields.get("schema") != Some(&"1")
            || fields.get("mode") != Some(&"progress")
        {
            return Err("unsupported progress schema or fields".into());
        }
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
        let mut counts = [0; 12];
        for (i, name) in NAMES.iter().enumerate() {
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
                || old.counts.iter().zip(counts).any(|(a, b)| *a > b))
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
