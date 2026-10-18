//! One bounded CPU worker for an explicitly launched timing load.
//!
//! No children, devices, allocation loop or scheduling changes. The runner
//! owns worker count and verifies inherited scheduling and affinity. This
//! command is not a benchmark and its checksum is not a campaign verdict.
use serde::Serialize;
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

pub const MAX_DURATION_MS: u64 = 600_000;
const BATCH_ITERATIONS: u64 = 1024;

#[derive(Debug, Serialize)]
pub struct Report {
    pub schema: u32,
    pub pid: u32,
    pub requested_ms: u64,
    pub elapsed_usec: u128,
    pub batches: u64,
    pub checksum: u64,
}

/// Stop at the first batch boundary after the monotonic deadline. Scheduler
/// delays can extend wall time; no userspace loop can promise a hard bound.
pub fn work(duration_ms: u64) -> Result<Report, String> {
    if !(1..=MAX_DURATION_MS).contains(&duration_ms) {
        return Err(format!(
            "CPU worker duration must be in 1..={MAX_DURATION_MS} ms"
        ));
    }
    let start = Instant::now();
    let duration = Duration::from_millis(duration_ms);
    let mut state = black_box(0x3380_3310_9e37_79b9_u64);
    let mut batches = 0_u64;
    while start.elapsed() < duration {
        for _ in 0..BATCH_ITERATIONS {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state = black_box(state.wrapping_mul(0x2545_f491_4f6c_dd1d));
        }
        batches += 1;
    }
    Ok(Report {
        schema: 1,
        pid: std::process::id(),
        requested_ms: duration_ms,
        elapsed_usec: start.elapsed().as_micros(),
        batches,
        checksum: black_box(state),
    })
}

pub fn run(arguments: &[String]) -> Result<Vec<String>, String> {
    let [duration] = arguments else {
        return Err("usage: xtask shell-timing cpu-worker DURATION_MS".into());
    };
    if duration.is_empty()
        || duration.starts_with('0')
        || !duration.bytes().all(|b| b.is_ascii_digit())
    {
        return Err("CPU worker duration must be canonical positive decimal milliseconds".into());
    }
    let duration = duration
        .parse()
        .map_err(|_| "CPU worker duration overflow")?;
    serde_json::to_string(&work(duration)?)
        .map(|s| vec![s])
        .map_err(|e| e.to_string())
}
