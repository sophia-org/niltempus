//! Join every offered candidate to exact kernel-backed display custody.
//! This establishes sample provenance, not the paired B9 performance verdict.
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

use serde::{Deserialize, Serialize};

const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RECORDS: usize = 100_000;
type Candidate = (u64, u64, u64, u64);
type Frame = (u64, u64, u64);

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Offer {
    pub connection_epoch: u64,
    pub content_grant_epoch: u64,
    pub output: u64,
    pub candidate_generation: u64,
    pub submitted_monotonic_usec: u64,
    pub outcome: String,
}

impl Offer {
    fn key(&self) -> Candidate {
        (
            self.connection_epoch,
            self.content_grant_epoch,
            self.output,
            self.candidate_generation,
        )
    }
}

#[derive(Debug, Serialize)]
pub struct Sample {
    pub connection_epoch: u64,
    pub content_grant_epoch: u64,
    pub output: u64,
    pub candidate_generation: u64,
    pub native_owner: u64,
    pub native_frame: u64,
    pub heads: u64,
    pub submitted_monotonic_usec: u64,
    pub completed_monotonic_usec: u64,
    pub latency_usec: u64,
}

#[derive(Debug)]
struct Binding {
    candidate: Candidate,
    frame: Frame,
    head: u64,
    target: u64,
    heads: u64,
    mode: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Completion {
    heads: u64,
    time: u64,
    kernel: bool,
}

fn fields<'a>(line: &'a str, required: &[&str]) -> Result<BTreeMap<&'a str, &'a str>, String> {
    let mut fields = BTreeMap::new();
    for word in line.split_whitespace().skip(1) {
        let (key, value) = word.split_once('=').ok_or("malformed evidence field")?;
        if !required.contains(&key) || fields.insert(key, value).is_some() {
            return Err(format!("unknown or repeated evidence field: {key}"));
        }
    }
    if fields.len() != required.len() || fields.get("schema") != Some(&"1") {
        return Err("missing field or unsupported evidence schema".into());
    }
    Ok(fields)
}

fn number(fields: &BTreeMap<&str, &str>, key: &str) -> Result<u64, String> {
    let text = fields.get(key).ok_or_else(|| format!("missing {key}"))?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("invalid {key}"));
    }
    text.parse::<u64>()
        .ok()
        .filter(|n| *n != 0)
        .ok_or_else(|| format!("invalid {key}"))
}

pub fn join(
    host: &str,
    offers: &[Offer],
    mode: u64,
    expected: usize,
) -> Result<Vec<Sample>, String> {
    if host.len() > MAX_BYTES as usize
        || !(1..=MAX_RECORDS).contains(&expected)
        || offers.len() != expected
        || !matches!(mode, 60_000 | 120_000)
    {
        return Err("invalid log size, expected offer count or exact 60/120 Hz mode".into());
    }
    let mut offered = BTreeSet::new();
    for offer in offers {
        if [
            offer.connection_epoch,
            offer.content_grant_epoch,
            offer.output,
            offer.candidate_generation,
            offer.submitted_monotonic_usec,
        ]
        .contains(&0)
            || offer.outcome != "presented"
            || !offered.insert(offer.key())
        {
            return Err("zero/duplicate offer identity or non-presented terminal outcome".into());
        }
    }
    let mut bindings: BTreeMap<(Candidate, Frame), BTreeMap<u64, Binding>> = BTreeMap::new();
    let mut completions = BTreeMap::new();
    let mut claimed = BTreeMap::new();
    let mut count = 0;
    for line in host.lines() {
        // The bounded diagnostic capture prepends TSV provenance columns.
        let line = line.rsplit('\t').next().unwrap_or(line);
        let name = line.split_whitespace().next().unwrap_or_default();
        if !matches!(
            name,
            "sophia_shell_native_binding" | "sophia_shell_native_completion"
        ) {
            continue;
        }
        count += 1;
        if count > MAX_RECORDS {
            return Err("too many native timing records".into());
        }
        if name == "sophia_shell_native_binding" {
            let f = fields(
                line,
                &[
                    "schema",
                    "connection_epoch",
                    "content_grant_epoch",
                    "output",
                    "candidate_generation",
                    "native_owner",
                    "native_frame",
                    "head",
                    "target_generation",
                    "heads",
                    "mode_refresh_millihz",
                ],
            )?;
            let output = number(&f, "output")?;
            let binding = Binding {
                candidate: (
                    number(&f, "connection_epoch")?,
                    number(&f, "content_grant_epoch")?,
                    output,
                    number(&f, "candidate_generation")?,
                ),
                frame: (
                    number(&f, "native_owner")?,
                    output,
                    number(&f, "native_frame")?,
                ),
                head: number(&f, "head")?,
                target: number(&f, "target_generation")?,
                heads: number(&f, "heads")?,
                mode: number(&f, "mode_refresh_millihz")?,
            };
            if !offered.contains(&binding.candidate) {
                continue;
            }
            let claim = (binding.candidate.0, binding.candidate.1, binding.frame);
            if claimed
                .insert(claim, binding.candidate.3)
                .is_some_and(|old| old != binding.candidate.3)
            {
                return Err("one role claims two candidate generations in one native frame".into());
            }
            if !(1..=4).contains(&binding.heads) || binding.mode != mode {
                return Err("wrong native head count or refresh rate".into());
            }
            let group = bindings
                .entry((binding.candidate, binding.frame))
                .or_default();
            if let Some(old) = group.get(&binding.head) {
                if old.target != binding.target
                    || old.heads != binding.heads
                    || old.mode != binding.mode
                {
                    return Err("contradictory head binding".into());
                }
            } else {
                group.insert(binding.head, binding);
            }
        } else {
            let f = fields(
                line,
                &[
                    "schema",
                    "output",
                    "native_owner",
                    "native_frame",
                    "heads",
                    "monotonic_usec",
                    "timestamp_source",
                    "missing_kernel_timestamp",
                ],
            )?;
            if !matches!(f["timestamp_source"], "kernel" | "observation_fallback")
                || !matches!(f["missing_kernel_timestamp"], "0" | "1")
            {
                return Err("invalid completion clock evidence".into());
            }
            let key = (
                number(&f, "native_owner")?,
                number(&f, "output")?,
                number(&f, "native_frame")?,
            );
            let value = Completion {
                heads: number(&f, "heads")?,
                time: number(&f, "monotonic_usec")?,
                kernel: f["timestamp_source"] == "kernel" && f["missing_kernel_timestamp"] == "0",
            };
            if completions
                .insert(key, value.clone())
                .is_some_and(|old| old != value)
            {
                return Err("contradictory native completion".into());
            }
        }
    }
    let mut topology = BTreeMap::new();
    let mut samples = Vec::with_capacity(offers.len());
    for offer in offers {
        let mut first: Option<(Frame, Completion)> = None;
        for ((_, frame), heads) in
            bindings.range((offer.key(), (0, 0, 0))..=(offer.key(), (u64::MAX, u64::MAX, u64::MAX)))
        {
            let count = heads.values().next().unwrap().heads;
            if heads.len() != count as usize || heads.values().any(|h| h.heads != count) {
                return Err("incomplete native head binding".into());
            }
            let layout: Vec<_> = heads
                .values()
                .map(|h| (h.frame.0, h.head, h.target, h.mode))
                .collect();
            if topology
                .insert(offer.output, layout.clone())
                .is_some_and(|old| old != layout)
            {
                return Err("native owner or topology changed during workload".into());
            }
            if let Some(completion) = completions.get(frame) {
                if completion.heads != count || !completion.kernel {
                    return Err("completion lacks exact heads or kernel timestamp".into());
                }
                if completion.time < offer.submitted_monotonic_usec {
                    return Err("completion predates submission".into());
                }
                if first
                    .as_ref()
                    .is_none_or(|(_, old)| completion.time < old.time)
                {
                    first = Some((*frame, completion.clone()));
                }
            }
        }
        let (frame, completion) =
            first.ok_or("offered candidate has no exact native completion")?;
        samples.push(Sample {
            connection_epoch: offer.connection_epoch,
            content_grant_epoch: offer.content_grant_epoch,
            output: offer.output,
            candidate_generation: offer.candidate_generation,
            native_owner: frame.0,
            native_frame: frame.2,
            heads: completion.heads,
            submitted_monotonic_usec: offer.submitted_monotonic_usec,
            completed_monotonic_usec: completion.time,
            latency_usec: completion.time - offer.submitted_monotonic_usec,
        });
    }
    Ok(samples)
}

fn bounded(path: &str) -> Result<String, String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > MAX_BYTES as usize {
        return Err("evidence log too large".into());
    }
    Ok(text)
}

pub fn run(args: &[String]) -> Result<Vec<String>, String> {
    let [command, host, client, mode, expected] = args else {
        return Err("expected shell-timing join HOST CLIENT MODE EXPECTED".into());
    };
    if command != "join" {
        return Err("expected shell-timing join".into());
    }
    let offers: Vec<Offer> = serde_json::from_str(&bounded(client)?).map_err(|e| e.to_string())?;
    let samples = join(
        &bounded(host)?,
        &offers,
        mode.parse().map_err(|_| "invalid mode")?,
        expected.parse().map_err(|_| "invalid count")?,
    )?;
    Ok(vec![serde_json::json!({"schema": 1, "verdict": "joined_only", "offered": offers.len(), "completed": samples.len(), "samples": samples}).to_string()])
}
