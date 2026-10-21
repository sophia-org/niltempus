//! Full-run latency summaries after strict journal and native-custody validation.
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Distribution {
    pub count: usize,
    pub p50_usec: u64,
    pub p95_usec: u64,
    pub p99_usec: u64,
    pub max_usec: u64,
}

impl Distribution {
    /// Nearest-rank percentiles; an absent measurement is never a zero sample.
    pub fn from_samples(samples: &[u64]) -> Option<Self> {
        if samples.is_empty() {
            return None;
        }
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let rank = |percent: usize| sorted[(sorted.len() * percent).div_ceil(100) - 1];
        Some(Self {
            count: sorted.len(),
            p50_usec: rank(50),
            p95_usec: rank(95),
            p99_usec: rank(99),
            max_usec: sorted[sorted.len() - 1],
        })
    }
}

pub fn latency_report(
    host: &str,
    journal: &str,
    mode: u64,
    expected: usize,
) -> Result<serde_json::Value, String> {
    let metrics = super::collect_metrics(journal, expected)?;
    if metrics.workload == super::WorkloadMode::Retained {
        return Err("retained workload has no latency acceptance until designed".into());
    }
    let native = super::join(host, &metrics.offers, mode, expected)?;
    let presentation: Vec<_> = native.iter().map(|s| s.latency_usec).collect();
    let permits: Vec<_> = metrics
        .permits
        .iter()
        .map(|s| s.received_usec - s.demand_usec)
        .collect();
    let uploads = |purpose| {
        metrics
            .uploads
            .iter()
            .filter(|s| s.purpose == purpose)
            .map(|s| s.accepted_usec - s.upload.begin_usec)
            .collect::<Vec<_>>()
    };
    let panel = uploads("presentation");
    let probe = uploads("transfer_probe");
    // Retain every over-bound upload, even when it eventually succeeded.
    let transfer_bound_reached: Vec<_> = metrics
        .uploads
        .iter()
        .filter(|s| s.accepted_usec - s.upload.begin_usec >= 2_000_000)
        .collect();
    Ok(serde_json::json!({
        "schema": 1,
        "verdict": "distributions_only",
        "percentile_method": "nearest_rank",
        "sample_scope": "all_offered_frames_no_warmup_exclusion",
        "mode_millihz": mode,
        "offered": expected,
        "candidate_to_native_presented": Distribution::from_samples(&presentation),
        "demand_to_permit": Distribution::from_samples(&permits),
        "presentation_upload_to_accepted": Distribution::from_samples(&panel),
        "probe_upload_to_accepted": Distribution::from_samples(&probe),
        "transfer_bound_reached": transfer_bound_reached,
        "native_samples": native,
        "journal_metrics": metrics,
    }))
}

/// Accepted uploads at or beyond this bound fail the transfer safety gate,
/// in warmup as well as measured frames.
pub const TRANSFER_BOUND_USEC: u64 = 2_000_000;

/// A declared latency window: the first `warmup` offered frames are excluded
/// from percentiles only; every frame must still complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Window {
    pub warmup: usize,
    pub measured: usize,
}

/// One value per latency measure, in the report's field names.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Measures<T> {
    pub candidate_to_native_presented: T,
    pub demand_to_permit: T,
    pub presentation_upload_to_accepted: T,
    pub probe_upload_to_accepted: T,
}

#[derive(Debug, Serialize)]
pub struct BoundReached {
    pub phase: &'static str,
    pub purpose: &'static str,
    pub candidate_generation: u64,
    pub resource_id: u64,
    pub resource_generation: u64,
    pub total_bytes: u64,
    pub upload_to_accepted_usec: u64,
}

#[derive(Serialize)]
pub struct WindowedLatency {
    pub schema: u8,
    pub verdict: &'static str,
    pub percentile_method: &'static str,
    pub sample_scope: &'static str,
    pub mode_millihz: u64,
    pub window: Window,
    pub offered: usize,
    pub wire: String,
    pub source: super::ImageSource,
    pub probe_bytes: u64,
    pub warmup_distributions: Measures<Option<Distribution>>,
    pub measured_distributions: Measures<Option<Distribution>>,
    /// Raw measured samples, kept so a condition can pool its pairs.
    pub measured_samples: Measures<Vec<u64>>,
    pub transfer_bound_reached: Vec<BoundReached>,
    pub native_samples: Vec<super::Sample>,
    pub journal_metrics: super::Journal,
}

fn distributions(samples: &Measures<Vec<u64>>) -> Measures<Option<Distribution>> {
    Measures {
        candidate_to_native_presented: Distribution::from_samples(
            &samples.candidate_to_native_presented,
        ),
        demand_to_permit: Distribution::from_samples(&samples.demand_to_permit),
        presentation_upload_to_accepted: Distribution::from_samples(
            &samples.presentation_upload_to_accepted,
        ),
        probe_upload_to_accepted: Distribution::from_samples(&samples.probe_upload_to_accepted),
    }
}

/// Validate the whole arm exactly as `latency_report` does, then split it at
/// the declared warmup. Any invalid frame, warmup included, rejects the arm.
pub fn latency_window(
    host: &str,
    journal: &str,
    mode: u64,
    window: Window,
) -> Result<WindowedLatency, String> {
    let offered = window
        .warmup
        .checked_add(window.measured)
        .filter(|_| window.warmup != 0 && window.measured != 0)
        .ok_or("window requires positive warmup and measured frame counts")?;
    let metrics = super::collect_metrics(journal, offered)?;
    if metrics.workload == super::WorkloadMode::Retained {
        return Err("retained workload has no latency acceptance until designed".into());
    }
    let native = super::join(host, &metrics.offers, mode, offered)?;
    if native.len() != offered {
        return Err("native evidence does not cover every offered frame".into());
    }
    let warm = |generation: u64| generation <= window.warmup as u64;
    let mut parts: [Measures<Vec<u64>>; 2] = Default::default();
    for sample in &native {
        parts[usize::from(!warm(sample.candidate_generation))]
            .candidate_to_native_presented
            .push(sample.latency_usec);
    }
    for permit in &metrics.permits {
        parts[usize::from(!warm(permit.generation))]
            .demand_to_permit
            .push(permit.received_usec - permit.demand_usec);
    }
    let mut transfer_bound_reached = Vec::new();
    for accepted in &metrics.uploads {
        let generation = accepted.upload.candidate_generation;
        let latency = accepted.accepted_usec - accepted.upload.begin_usec;
        let part = &mut parts[usize::from(!warm(generation))];
        if accepted.purpose == "transfer_probe" {
            part.probe_upload_to_accepted.push(latency);
        } else {
            part.presentation_upload_to_accepted.push(latency);
        }
        if latency >= TRANSFER_BOUND_USEC {
            transfer_bound_reached.push(BoundReached {
                phase: if warm(generation) {
                    "warmup"
                } else {
                    "measured"
                },
                purpose: accepted.purpose,
                candidate_generation: generation,
                resource_id: accepted.upload.resource_id,
                resource_generation: accepted.upload.resource_generation,
                total_bytes: accepted.upload.total_bytes,
                upload_to_accepted_usec: latency,
            });
        }
    }
    let [warmup, measured] = parts;
    Ok(WindowedLatency {
        schema: 1,
        verdict: "distributions_only",
        percentile_method: "nearest_rank",
        sample_scope: "declared_warmup_excluded",
        mode_millihz: mode,
        window,
        offered,
        wire: metrics.wire.clone(),
        source: metrics.source,
        probe_bytes: metrics.probe_bytes,
        warmup_distributions: distributions(&warmup),
        measured_distributions: distributions(&measured),
        measured_samples: measured,
        transfer_bound_reached,
        native_samples: native,
        journal_metrics: metrics,
    })
}
