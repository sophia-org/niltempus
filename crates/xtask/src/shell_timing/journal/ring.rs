//! Schema 5 records raw ring custody; schema 7 records SDK observations.
//! Fresh by default; an optional Start interval adds per-offer due times, and
//! the retained workload copies a tile pool once, then places it every frame.
use super::{
    AcceptedUpload, CandidatePlacements, Event as LegacyEvent, ImageSource, Journal, LiveResource,
    Offer, PermitTiming, Upload, WorkloadMode,
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
    pub admitted_usec: Option<u64>,
    pub ready_usec: u64,
}

#[derive(Serialize)]
pub struct CopyTiming {
    pub offer: u64,
    pub slot: u64,
    pub slot_generation: Option<u64>,
    pub resource_id: u64,
    pub resource_generation: u64,
    pub fill_start_usec: u64,
    /// Peer's absolute schedule for paced journals; fill start is observed work.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_usec: Option<u64>,
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
        /// Absent in fresh journals, including every historical capture.
        #[serde(default)]
        interval_msec: Option<u64>,
        /// Retained pool size; absent in fresh journals.
        #[serde(default)]
        retained_resources: Option<u64>,
    },
    RingReady {
        #[serde(flatten)]
        ready: Ready,
    },
    RingOffer {
        upload: Upload,
        offer: u64,
        slot: u64,
        slot_generation: Option<u64>,
        fill_start_usec: u64,
        #[serde(default)]
        due_usec: Option<u64>,
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
        slot_generation: Option<u64>,
        resource_id: u64,
        resource_generation: u64,
        released_usec: u64,
    },
}

fn next<T: serde::de::DeserializeOwned>(
    lines: &mut std::str::Lines<'_>,
    schema: u64,
) -> Result<T, String> {
    #[derive(Deserialize)]
    struct Record<T> {
        schema: u64,
        #[serde(flatten)]
        event: T,
    }
    let record: Record<T> = serde_json::from_str(lines.next().ok_or("incomplete ring journal")?)
        .map_err(|e| e.to_string())?;
    if record.schema != schema {
        return Err("mixed ring journal schemas".into());
    }
    Ok(record.event)
}

/// One ring copy as the peer journals it: offer, acceptance, slot release.
struct ExpectedCopy {
    offer: u64,
    slots: u64,
    resource_id: u64,
    resource_generation: u64,
    candidate_generation: u64,
}

fn copy(
    lines: &mut std::str::Lines<'_>,
    schema: u64,
    ready: &Ready,
    expect: ExpectedCopy,
    not_before: u64,
) -> Result<(Upload, CopyTiming, u64), String> {
    let ExpectedCopy {
        offer: number,
        slots,
        resource_id,
        resource_generation,
        candidate_generation,
    } = expect;
    let expected_slot = (number - 1) % slots;
    let expected_generation = number.checked_mul(2).ok_or("ring generation overflow")?;
    let (upload, slot, slot_generation, fill_start_usec, due_usec) = match next(lines, schema)? {
        Event::RingOffer {
            upload,
            offer,
            slot: s,
            slot_generation: g,
            fill_start_usec,
            due_usec,
        } if offer == number
            && s < slots
            && if schema == 5 {
                s == expected_slot && g == Some(expected_generation)
            } else {
                g.is_none()
            }
            && upload.connection_epoch == ready.connection_epoch
            && upload.content_grant_epoch == ready.content_grant_epoch
            && upload.output > 0
            && upload.candidate_generation == candidate_generation
            && upload.resource_id == resource_id
            && upload.resource_generation == resource_generation
            && upload.width > 0
            && upload.height > 0
            && u64::from(upload.width)
                .checked_mul(u64::from(upload.height))
                .and_then(|n| n.checked_mul(4))
                == Some(upload.total_bytes)
            && upload.total_bytes <= ready.pixel_bytes
            && fill_start_usec >= not_before
            && upload.begin_usec >= fill_start_usec =>
        {
            (upload, s, g, fill_start_usec, due_usec)
        }
        _ => return Err("invalid ring offer identity, geometry, slot custody or timing".into()),
    };
    let accepted_usec = match next(lines, schema)? {
        Event::RingAccepted {
            offer,
            resource_id: id,
            resource_generation: generation,
            accepted_usec,
        } if offer == number
            && id == resource_id
            && generation == resource_generation
            && accepted_usec >= upload.begin_usec =>
        {
            accepted_usec
        }
        _ => return Err("missing or misattributed ring acceptance".into()),
    };
    let released_usec = match next(lines, schema)? {
        Event::SlotReleased {
            offer,
            slot: s,
            slot_generation: g,
            resource_id: id,
            resource_generation: generation,
            released_usec,
        } if offer == number
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
    let timing = CopyTiming {
        offer: number,
        slot,
        slot_generation,
        resource_id,
        resource_generation,
        fill_start_usec,
        due_usec,
        released_usec,
    };
    Ok((upload, timing, accepted_usec))
}

pub(super) fn collect(journal: &str, expected: usize, schema: u64) -> Result<Journal, String> {
    let mut lines = journal.lines();
    let (slots, workload, retained, interval_msec) = match next(&mut lines, schema)? {
        Event::Start {
            frames,
            wire,
            probe_bytes: 0,
            source: ImageSource::OwnedRing,
            workload,
            ring_slots,
            interval_msec,
            retained_resources,
        } if frames == expected
            && wire == "files"
            && (2..=4).contains(&ring_slots)
            && (schema == 5
                || (workload == WorkloadMode::Fresh
                    && interval_msec.is_none()
                    && retained_resources.is_none()))
            && interval_msec.is_none_or(|n| (1..=10_000).contains(&n))
            // Retained is vsync-paced, as the peer refuses an interval.
            && (workload == WorkloadMode::Fresh || interval_msec.is_none()) =>
        {
            let retained = workload.check(retained_resources)?;
            (ring_slots, workload, retained, interval_msec)
        }
        _ => return Err("ring journal requires the declared fresh or retained panel arm".into()),
    };
    let ready = match next(&mut lines, schema)? {
        Event::RingReady { ready }
            if ready.slots == slots
                && ![
                    ready.connection_epoch,
                    ready.content_grant_epoch,
                    ready.channel,
                    ready.ring,
                    ready.ring_generation,
                    ready.ready_usec,
                ]
                .contains(&0)
                && if schema == 5 {
                    ready
                        .admitted_usec
                        .is_some_and(|t| t > 0 && ready.ready_usec >= t)
                } else {
                    ready.admitted_usec.is_none()
                }
                && (4..=4 * 1024 * 1024).contains(&ready.pixel_bytes)
                && ready.pixel_bytes % 4 == 0 =>
        {
            ready
        }
        _ => return Err("missing or invalid ring readiness".into()),
    };
    let mut result = Journal {
        schema,
        wire: "files".into(),
        source: ImageSource::OwnedRing,
        probe_bytes: 0,
        workload,
        retained_resources: retained,
        interval_msec,
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
    // Retained pool: tiles 1..=N, generation 1, copied once at candidate
    // generation 0 as offers 1..=N, with one shared even width and height.
    // Each tile stays live through the frame phase; its slot may recycle.
    for index in 1..=retained.unwrap_or(0) {
        let expect = ExpectedCopy {
            offer: index,
            slots,
            resource_id: index,
            resource_generation: 1,
            candidate_generation: 0,
        };
        let (upload, timing, accepted_usec) = copy(&mut lines, schema, &ready, expect, last)?;
        let current = (upload.output, upload.width, upload.height);
        if upload.width % 2 != 0 || geometry.replace(current).is_some_and(|old| old != current) {
            return Err("retained tiles need one even width, height and output".into());
        }
        last = timing.released_usec;
        copies.push(timing);
        result.live_resources.push(LiveResource {
            resource_id: index,
            resource_generation: 1,
            released_usec: None,
        });
        result.uploads.push(AcceptedUpload {
            purpose: "retained",
            upload,
            accepted_usec,
            released_usec: None,
        });
    }
    let mut first_due = None;
    for frame in 1..=expected as u64 {
        let mut not_before = last;
        if retained.is_none() {
            let expect = ExpectedCopy {
                offer: frame,
                slots,
                resource_id: (frame - 1) % 2 + 1,
                resource_generation: (frame - 1) / 2 + 1,
                candidate_generation: frame,
            };
            let (upload, timing, accepted_usec) = copy(&mut lines, schema, &ready, expect, last)?;
            // A paced journal names every due time on one absolute schedule that
            // starts at or after readiness; late fills keep later deadlines fixed.
            let on_schedule = match (interval_msec, timing.due_usec) {
                (None, None) => true,
                (Some(interval), Some(due)) => {
                    let first = *first_due.get_or_insert(due);
                    first >= ready.ready_usec
                        && timing.fill_start_usec >= due
                        && (frame - 1)
                            .checked_mul(interval * 1000)
                            .and_then(|n| n.checked_add(first))
                            == Some(due)
                }
                _ => false,
            };
            if !on_schedule {
                return Err("ring due time absent, off schedule or after fill start".into());
            }
            let current = (upload.output, upload.width, upload.height);
            if upload.total_bytes != ready.pixel_bytes
                || geometry.replace(current).is_some_and(|old| old != current)
            {
                return Err("ring geometry or output changed".into());
            }
            not_before = timing.released_usec;
            copies.push(timing);
            result.uploads.push(AcceptedUpload {
                purpose: "presentation",
                upload,
                accepted_usec,
                released_usec: None,
            });
        }
        let output = geometry.map_or(0, |(output, _, _)| output);
        let received_usec = match next::<LegacyEvent>(&mut lines, schema)? {
            LegacyEvent::Permit {
                generation,
                demand_usec,
                received_usec,
            } if generation == frame
                && demand_usec >= not_before
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
        if let (Some(count), Some((_, tile_width, _))) = (retained, geometry) {
            // Every candidate places the whole pool, rotated by one tile per frame.
            match next::<LegacyEvent>(&mut lines, schema)? {
                LegacyEvent::CandidatePlacements {
                    candidate_generation,
                    placements,
                } if candidate_generation == frame
                    && placements.len() as u64 == count
                    && placements.iter().zip(0u64..).all(|(placement, index)| {
                        placement.resource_id == index + 1
                            && placement.resource_generation == 1
                            && placement.destination_y_px == 0
                            && u64::try_from(placement.destination_x_px).ok()
                                == Some((index + frame - 1) % count * u64::from(tile_width))
                    }) =>
                {
                    result.placements.push(CandidatePlacements {
                        candidate_generation,
                        placements,
                    });
                }
                _ => return Err("missing, wrong or unaccepted ring candidate placements".into()),
            }
        }
        let offer = match next::<LegacyEvent>(&mut lines, schema)? {
            LegacyEvent::Offer {
                connection_epoch,
                content_grant_epoch,
                output: o,
                candidate_generation,
                submitted_monotonic_usec,
            } if connection_epoch == ready.connection_epoch
                && content_grant_epoch == ready.content_grant_epoch
                && o == output
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
        match next::<LegacyEvent>(&mut lines, schema)? {
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
        result.offers.push(offer);
        result.terminal_usec.push(last);
    }
    if !matches!(next::<LegacyEvent>(&mut lines, schema)?, LegacyEvent::End { offered, completed }
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
