//! Full-run latency summaries after strict journal and native-custody validation.
use serde::Serialize;

#[derive(Debug, Serialize)]
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
