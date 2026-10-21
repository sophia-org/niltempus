//! Schema 5 is a distinct ring experiment, never a sealed upload journal.
use super::{
    AcceptedUpload, Event as LegacyEvent, ImageSource, Journal, Offer, PermitTiming, Upload,
    WorkloadMode,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Ready {
    pub connection_epoch: u64,
    pub content_grant_epoch: u64,
    pub channel: u64,
    pub ring: u64,
    pub ring_generation: u64,
    pub slots: u64,
    pub pixel_bytes: u64,
    pub admitted_usec: u64,
    pub ready_usec: u64,
}

#[derive(Serialize)]
pub struct CopyTiming {
    pub offer: u64,
    pub slot: u64,
    pub slot_generation: u64,
    pub resource_id: u64,
    pub resource_generation: u64,
    pub fill_start_usec: u64,
    pub released_usec: u64,
}

#[derive(Serialize)]
pub struct RingEvidence {
    pub registration: Ready,
    pub copies: Vec<CopyTiming>,
}

#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
enum Event {
    Start {
        frames: usize,
        wire: String,
        probe_bytes: u64,
        source: ImageSource,
        workload: WorkloadMode,
        ring_slots: u64,
    },
    RingReady {
        #[serde(flatten)]
        ready: Ready,
    },
    RingOffer {
        upload: Upload,
        offer: u64,
        slot: u64,
        slot_generation: u64,
        fill_start_usec: u64,
    },
    RingAccepted {
        offer: u64,
        resource_id: u64,
        resource_generation: u64,
        accepted_usec: u64,
    },
    SlotReleased {
        offer: u64,
        slot: u64,
        slot_generation: u64,
        resource_id: u64,
        resource_generation: u64,
        released_usec: u64,
    },
}

fn next<T: serde::de::DeserializeOwned>(lines: &mut std::str::Lines<'_>) -> Result<T, String> {
    #[derive(Deserialize)]
    struct Record<T> {
        schema: u64,
        #[serde(flatten)]
        event: T,
    }
    let record: Record<T> = serde_json::from_str(lines.next().ok_or("incomplete ring journal")?)
        .map_err(|e| e.to_string())?;
    if record.schema != 5 {
        return Err("mixed ring journal schemas".into());
    }
    Ok(record.event)
}

pub(super) fn collect(journal: &str, expected: usize) -> Result<Journal, String> {
    let mut lines = journal.lines();
    let slots = match next(&mut lines)? {
        Event::Start {
            frames,
            wire,
            probe_bytes: 0,
            source: ImageSource::OwnedRing,
            workload: WorkloadMode::Fresh,
            ring_slots,
        } if frames == expected && wire == "files" && (2..=4).contains(&ring_slots) => ring_slots,
        _ => return Err("ring journal requires the declared fresh panel arm".into()),
    };
    let ready = match next(&mut lines)? {
        Event::RingReady { ready }
            if ready.slots == slots
                && ![
                    ready.connection_epoch,
                    ready.content_grant_epoch,
                    ready.channel,
                    ready.ring,
                    ready.ring_generation,
                    ready.admitted_usec,
                ]
                .contains(&0)
                && ready.ready_usec >= ready.admitted_usec
                && (4..=4 * 1024 * 1024).contains(&ready.pixel_bytes)
                && ready.pixel_bytes % 4 == 0 =>
        {
            ready
        }
        _ => return Err("missing or invalid ring readiness".into()),
    };
    let mut result = Journal {
        schema: 5,
        wire: "files".into(),
        source: ImageSource::OwnedRing,
        probe_bytes: 0,
        workload: WorkloadMode::Fresh,
        retained_resources: None,
        interval_msec: None,
        ring: None,
        draw: None,
        offers: Vec::with_capacity(expected),
        uploads: Vec::with_capacity(expected),
        permits: Vec::with_capacity(expected),
        terminal_usec: Vec::with_capacity(expected),
        placements: Vec::new(),
        live_resources: Vec::new(),
    };
    let mut copies = Vec::with_capacity(expected);
    let mut last = ready.ready_usec;
    let mut geometry = None;
    for frame in 1..=expected as u64 {
        let slot = (frame - 1) % slots;
        let slot_generation = frame.checked_mul(2).ok_or("ring generation overflow")?;
        let resource_id = (frame - 1) % 2 + 1;
        let resource_generation = (frame - 1) / 2 + 1;
        let (upload, fill_start_usec) = match next(&mut lines)? {
            Event::RingOffer {
                upload,
                offer,
                slot: s,
                slot_generation: g,
                fill_start_usec,
            } if offer == frame
                && s == slot
                && g == slot_generation
                && upload.connection_epoch == ready.connection_epoch
                && upload.content_grant_epoch == ready.content_grant_epoch
                && upload.output > 0
                && upload.candidate_generation == frame
                && upload.resource_id == resource_id
                && upload.resource_generation == resource_generation
                && upload.width > 0
                && upload.height > 0
                && u64::from(upload.width)
                    .checked_mul(u64::from(upload.height))
                    .and_then(|n| n.checked_mul(4))
                    == Some(upload.total_bytes)
                && upload.total_bytes == ready.pixel_bytes
                && fill_start_usec >= last
                && upload.begin_usec >= fill_start_usec =>
            {
                (upload, fill_start_usec)
            }
            _ => {
                return Err("invalid ring offer identity, geometry, slot custody or timing".into());
            }
        };
        let current = (upload.output, upload.width, upload.height);
        if geometry.replace(current).is_some_and(|old| old != current) {
            return Err("ring geometry or output changed".into());
        }
        let accepted_usec = match next(&mut lines)? {
            Event::RingAccepted {
                offer,
                resource_id: id,
                resource_generation: generation,
                accepted_usec,
            } if offer == frame
                && id == resource_id
                && generation == resource_generation
                && accepted_usec >= upload.begin_usec =>
            {
                accepted_usec
            }
            _ => return Err("missing or misattributed ring acceptance".into()),
        };
        let released_usec = match next(&mut lines)? {
            Event::SlotReleased {
                offer,
                slot: s,
                slot_generation: g,
                resource_id: id,
                resource_generation: generation,
                released_usec,
            } if offer == frame
                && s == slot
                && g == slot_generation
                && id == resource_id
                && generation == resource_generation
                && released_usec >= accepted_usec =>
            {
                released_usec
            }
            _ => return Err("slot release must follow its exact accepted copy".into()),
        };
        let received_usec = match next::<LegacyEvent>(&mut lines)? {
            LegacyEvent::Permit {
                generation,
                demand_usec,
                received_usec,
            } if generation == frame
                && demand_usec >= released_usec
                && received_usec >= demand_usec =>
            {
                result.permits.push(PermitTiming {
                    generation,
                    demand_usec,
                    received_usec,
                });
                received_usec
            }
            _ => return Err("ring permit precedes settlement or has wrong identity".into()),
        };
        let offer = match next::<LegacyEvent>(&mut lines)? {
            LegacyEvent::Offer {
                connection_epoch,
                content_grant_epoch,
                output,
                candidate_generation,
                submitted_monotonic_usec,
            } if connection_epoch == ready.connection_epoch
                && content_grant_epoch == ready.content_grant_epoch
                && output == upload.output
                && candidate_generation == frame
                && submitted_monotonic_usec >= received_usec =>
            {
                Offer {
                    connection_epoch,
                    content_grant_epoch,
                    output,
                    candidate_generation,
                    submitted_monotonic_usec,
                    outcome: "presented".into(),
                }
            }
            _ => return Err("ring candidate does not match its accepted resource".into()),
        };
        match next::<LegacyEvent>(&mut lines)? {
            LegacyEvent::Terminal {
                connection_epoch,
                content_grant_epoch,
                output,
                candidate_generation,
                outcome,
                kind: 2,
                reason: 0,
                observed_monotonic_usec,
            } if (
                connection_epoch,
                content_grant_epoch,
                output,
                candidate_generation,
            ) == offer.key()
                && outcome == "presented"
                && observed_monotonic_usec >= offer.submitted_monotonic_usec =>
            {
                last = observed_monotonic_usec
            }
            _ => return Err("missing or invalid ring candidate presentation".into()),
        }
        copies.push(CopyTiming {
            offer: frame,
            slot,
            slot_generation,
            resource_id,
            resource_generation,
            fill_start_usec,
            released_usec,
        });
        result.uploads.push(AcceptedUpload {
            purpose: "presentation",
            upload,
            accepted_usec,
            released_usec: None,
        });
        result.offers.push(offer);
        result.terminal_usec.push(last);
    }
    if !matches!(next::<LegacyEvent>(&mut lines)?, LegacyEvent::End { offered, completed }
        if offered == expected && completed == expected)
        || lines.next().is_some()
    {
        return Err("wrong ring end count or records after end".into());
    }
    result.ring = Some(RingEvidence {
        registration: ready,
        copies,
    });
    Ok(result)
}
