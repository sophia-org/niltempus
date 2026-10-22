//! Explicit timing declarations; frame joins do not prove a sampled idle window.
use super::{ImageSource, PeerConfig, Request, WorkloadMode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "workload", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExternalTiming {
    Idle { seconds: u64 },
    Bursty { interval_msec: u64, warmup: usize },
}

impl ExternalTiming {
    pub fn interval(&self) -> Option<u64> {
        match self {
            Self::Bursty { interval_msec, .. } => Some(*interval_msec),
            Self::Idle { .. } => None,
        }
    }
}

pub(super) fn validate(request: &Request, peer: &PeerConfig) -> Result<(), String> {
    let Some(timing) = &request.external_timing else {
        return if peer.interval_msec.is_none() {
            Ok(())
        } else {
            Err("interval_msec requires an explicit external bursty timing plan".into())
        };
    };
    // Owned ring is the production shell path; sealed ISB stays for history.
    if request.wire != "files"
        || !matches!(request.source, ImageSource::Isb | ImageSource::OwnedRing)
        || request.instrumentation != "none"
        || peer.workload != WorkloadMode::Fresh
        || peer.probe_bytes != 0
        || peer.height != 24
        || peer.interval_msec != timing.interval()
    {
        return Err(
            "external idle/bursty timing requires an uninstrumented sealed or owned-ring panel"
                .into(),
        );
    }
    match timing {
        ExternalTiming::Idle { seconds } => {
            if *seconds != 60 || request.frames != 1 || peer.hold_msec < 64_000 {
                return Err("idle needs one frame and a hold covering +2..+62s with margin".into());
            }
        }
        ExternalTiming::Bursty {
            interval_msec,
            warmup,
        } => {
            let scheduled = (request.frames.saturating_sub(1) as u64)
                .checked_mul(*interval_msec)
                .and_then(|n| n.checked_add(4000));
            if !(1..=10_000).contains(interval_msec)
                || *warmup >= request.frames
                || scheduled.is_none_or(|n| n > request.runtime_ms)
            {
                return Err(
                    "bursty timing exceeds count, interval or capture duration bounds".into(),
                );
            }
        }
    }
    // Still no promise that presentation occurs soon enough for the entire
    // idle sampling window: the observer must retain and validate its brackets.
    Ok(())
}
