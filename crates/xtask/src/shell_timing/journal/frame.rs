//! Schema 8: CPU SFT commits, separate source custody, exact terminal/retirement.
use super::ring::{self, CopyTiming, Event as Ring, RingEvidence};
use super::{
    AcceptedUpload, Event as Legacy, ImageSource, Journal, Offer, RetireCounters, WorkloadMode,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
enum Frame {
    #[serde(rename = "frame_opened")]
    Opened {
        stream_id: u64,
        observed_monotonic_usec: u64,
    },
    #[serde(rename = "frame_prepared")]
    Prepared {
        candidate_generation: u64,
        observed_monotonic_usec: u64,
    },
    #[serde(rename = "frame_retired")]
    Retired {
        candidate_generation: u64,
        resource_id: u64,
        resource_generation: u64,
        observed_monotonic_usec: u64,
    },
    #[serde(rename = "frame_closed")]
    Closed {
        stream_id: u64,
        observed_monotonic_usec: u64,
    },
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Event {
    Frame(Frame),
    Ring(Ring),
    Legacy(Legacy),
}
#[derive(Serialize)]
pub struct Evidence {
    pub stream_id: u64,
    pub opened_usec: u64,
    pub closed_usec: u64,
    pub prepared_usec: Vec<u64>,
    pub predecessor_released_usec: Vec<u64>,
}
fn next(lines: &mut std::str::Lines<'_>) -> Result<Event, String> {
    ring::next(lines, 8)
}

pub(super) fn collect(journal: &str, expected: usize) -> Result<Journal, String> {
    let mut lines = journal.lines();
    match next(&mut lines)? {
        Event::Ring(Ring::Start {
            frames,
            wire,
            probe_bytes: 0,
            source: ImageSource::OwnedRing,
            workload: WorkloadMode::Fresh,
            ring_slots: 2,
            interval_msec: None,
            retained_resources: None,
        }) if frames == expected && wire == "files" => {}
        _ => return Err("SFT journal requires a fresh CPU two-slot files workload".into()),
    }
    let ready = match next(&mut lines)? {
        Event::Ring(Ring::RingReady { ready })
            if ready.slots == 2
                && ready.admitted_usec.is_none()
                && ![
                    ready.connection_epoch,
                    ready.content_grant_epoch,
                    ready.channel,
                    ready.ring,
                    ready.ring_generation,
                    ready.ready_usec,
                ]
                .contains(&0)
                && (4..=4 * 1024 * 1024).contains(&ready.pixel_bytes)
                && ready.pixel_bytes.is_multiple_of(4) =>
        {
            ready
        }
        _ => return Err("invalid SFT ring readiness".into()),
    };
    let (stream_id, opened_usec) = match next(&mut lines)? {
        Event::Frame(Frame::Opened {
            stream_id,
            observed_monotonic_usec,
        }) if stream_id > 0 && observed_monotonic_usec >= ready.ready_usec => {
            (stream_id, observed_monotonic_usec)
        }
        _ => return Err("missing SFT stream admission".into()),
    };
    let mut result = Journal {
        schema: 8,
        wire: "files".into(),
        source: ImageSource::OwnedRing,
        probe_bytes: 0,
        workload: WorkloadMode::Fresh,
        retained_resources: None,
        interval_msec: None,
        ring: None,
        draw: None,
        frame: None,
        offers: Vec::with_capacity(expected),
        uploads: Vec::with_capacity(expected),
        permits: Vec::new(),
        terminal_usec: Vec::with_capacity(expected),
        placements: Vec::new(),
        live_resources: Vec::new(),
        retire_counters: None,
    };
    let mut evidence = Evidence {
        stream_id,
        opened_usec,
        closed_usec: 0,
        prepared_usec: Vec::with_capacity(expected),
        predecessor_released_usec: Vec::with_capacity(expected.saturating_sub(1)),
    };
    let mut copies = Vec::with_capacity(expected);
    let mut geometry = None;
    let mut last = opened_usec;
    for generation in 1..=expected as u64 {
        let resource_id = (generation - 1) % 2 + 1;
        let resource_generation = (generation - 1) / 2 + 1;
        let (upload, slot, fill_start_usec) = match next(&mut lines)? {
            Event::Ring(Ring::RingOffer {
                upload,
                offer,
                slot,
                slot_generation: None,
                fill_start_usec,
                due_usec: None,
            }) if offer == generation
                && slot < 2
                && fill_start_usec >= last
                && upload.begin_usec >= fill_start_usec
                && upload.connection_epoch == ready.connection_epoch
                && upload.content_grant_epoch == ready.content_grant_epoch
                && upload.output > 0
                && upload.candidate_generation == generation
                && upload.resource_id == resource_id
                && upload.resource_generation == resource_generation
                && upload.width > 0
                && upload.height > 0
                && u64::from(upload.width)
                    .checked_mul(u64::from(upload.height))
                    .and_then(|n| n.checked_mul(4))
                    == Some(upload.total_bytes)
                && upload.total_bytes == ready.pixel_bytes =>
            {
                (upload, slot, fill_start_usec)
            }
            _ => return Err("SFT source identity, capacity or fill timing changed".into()),
        };
        let current = (upload.output, upload.width, upload.height);
        if geometry.replace(current).is_some_and(|old| old != current) {
            return Err("SFT output geometry changed".into());
        }
        let submitted = match next(&mut lines)? {
            Event::Legacy(Legacy::Offer {
                connection_epoch,
                content_grant_epoch,
                output,
                candidate_generation,
                submitted_monotonic_usec,
            }) if connection_epoch == ready.connection_epoch
                && content_grant_epoch == ready.content_grant_epoch
                && output == upload.output
                && candidate_generation == generation
                && submitted_monotonic_usec >= upload.begin_usec =>
            {
                submitted_monotonic_usec
            }
            _ => return Err("SFT commit identity or timing mismatch".into()),
        };
        result.offers.push(Offer {
            connection_epoch: ready.connection_epoch,
            content_grant_epoch: ready.content_grant_epoch,
            output: upload.output,
            candidate_generation: generation,
            submitted_monotonic_usec: submitted,
            outcome: "presented".into(),
        });
        last = submitted;
        let (mut accepted, mut prepared, mut terminal, mut released, mut retired) =
            (None, None, None, None, generation == 1);
        while accepted.is_none()
            || prepared.is_none()
            || terminal.is_none()
            || released.is_none()
            || !retired
        {
            let timestamp = match next(&mut lines)? {
                Event::Ring(Ring::RingAccepted {
                    offer,
                    resource_id: id,
                    resource_generation: version,
                    accepted_usec,
                }) if accepted.is_none()
                    && offer == generation
                    && (id, version) == (resource_id, resource_generation) =>
                {
                    accepted = Some(accepted_usec);
                    accepted_usec
                }
                Event::Frame(Frame::Prepared {
                    candidate_generation,
                    observed_monotonic_usec,
                }) if accepted.is_some()
                    && prepared.is_none()
                    && candidate_generation == generation =>
                {
                    prepared = Some(observed_monotonic_usec);
                    observed_monotonic_usec
                }
                Event::Legacy(Legacy::Terminal {
                    connection_epoch,
                    content_grant_epoch,
                    output,
                    candidate_generation,
                    outcome,
                    kind: 2,
                    reason: 0,
                    observed_monotonic_usec,
                }) if prepared.is_some()
                    && terminal.is_none()
                    && connection_epoch == ready.connection_epoch
                    && content_grant_epoch == ready.content_grant_epoch
                    && output == upload.output
                    && candidate_generation == generation
                    && outcome == "presented" =>
                {
                    terminal = Some(observed_monotonic_usec);
                    observed_monotonic_usec
                }
                Event::Ring(Ring::SlotReleased {
                    offer,
                    slot: source_slot,
                    slot_generation: None,
                    resource_id: id,
                    resource_generation: version,
                    released_usec,
                }) if accepted.is_some()
                    && released.is_none()
                    && offer == generation
                    && source_slot == slot
                    && (id, version) == (resource_id, resource_generation) =>
                {
                    released = Some(released_usec);
                    released_usec
                }
                Event::Frame(Frame::Retired {
                    candidate_generation,
                    resource_id: id,
                    resource_generation: version,
                    observed_monotonic_usec,
                }) if generation > 1
                    && terminal.is_some()
                    && !retired
                    && candidate_generation == generation
                    && (id, version) == ((generation - 2) % 2 + 1, (generation - 2) / 2 + 1) =>
                {
                    retired = true;
                    evidence
                        .predecessor_released_usec
                        .push(observed_monotonic_usec);
                    result.uploads[generation as usize - 2].released_usec =
                        Some(observed_monotonic_usec);
                    observed_monotonic_usec
                }
                _ => {
                    return Err(
                        "SFT completion duplicated, misattributed, missing or out of order".into(),
                    );
                }
            };
            if timestamp < last {
                return Err("SFT observation clock regressed".into());
            }
            last = timestamp;
        }
        evidence
            .prepared_usec
            .push(prepared.ok_or("missing SFT Prepared")?);
        result
            .terminal_usec
            .push(terminal.ok_or("missing SFT Presented")?);
        copies.push(CopyTiming {
            offer: generation,
            slot,
            slot_generation: None,
            resource_id,
            resource_generation,
            fill_start_usec,
            due_usec: None,
            released_usec: released.ok_or("missing SFT source release")?,
        });
        result.uploads.push(AcceptedUpload {
            purpose: "presentation",
            upload,
            accepted_usec: accepted.ok_or("missing SFT acceptance")?,
            released_usec: None,
        });
    }
    match next(&mut lines)? {
        Event::Frame(Frame::Closed {
            stream_id: id,
            observed_monotonic_usec,
        }) if id == stream_id && observed_monotonic_usec >= last => {
            evidence.closed_usec = observed_monotonic_usec
        }
        _ => return Err("SFT stream did not close after complete settlement".into()),
    }
    match next(&mut lines)? {
        Event::Legacy(Legacy::End {
            offered,
            completed,
            retire_counter_schema: Some(1),
            explicit_retires_sent: Some(0),
            predecessor_releases_received: Some(releases),
        }) if offered == expected
            && completed == expected
            && releases == expected.saturating_sub(1) as u64 =>
        {
            result.retire_counters = Some(RetireCounters {
                retire_counter_schema: 1,
                explicit_retires_sent: 0,
                predecessor_releases_received: releases,
            });
        }
        _ => return Err("SFT terminal or retirement counts disagree".into()),
    }
    if lines.next().is_some() {
        return Err("SFT journal has trailing records".into());
    }
    result.ring = Some(RingEvidence {
        registration: ready,
        copies,
    });
    result.frame = Some(evidence);
    Ok(result)
}
