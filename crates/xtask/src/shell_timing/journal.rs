//! Validate the paced peer's complete journal before exporting any samples.
use super::{ImageSource, MAX_BYTES, MAX_RECORDS, Offer, WorkloadMode};
use serde::{Deserialize, Serialize};

mod draw;
mod ring;
pub use draw::{BatchTiming, DrawEvidence, DrawSource, DrawWorkload, RingReady6, ValueDueTiming};
pub use ring::RingEvidence;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Upload {
    pub connection_epoch: u64,
    pub content_grant_epoch: u64,
    pub output: u64,
    pub candidate_generation: u64,
    pub resource_id: u64,
    pub resource_generation: u64,
    pub width: u32,
    pub height: u32,
    pub total_bytes: u64,
    pub begin_usec: u64,
}

#[derive(Serialize)]
pub struct AcceptedUpload {
    pub purpose: &'static str,
    pub upload: Upload,
    pub accepted_usec: u64,
    pub released_usec: Option<u64>,
}

#[derive(Serialize)]
pub struct PermitTiming {
    pub generation: u64,
    pub demand_usec: u64,
    pub received_usec: u64,
}

/// One retained-pool reference inside a candidate. Geometry is validated
/// against the pool's tile width; the resource must already be accepted.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub resource_id: u64,
    pub resource_generation: u64,
    pub destination_x_px: i32,
    pub destination_y_px: i32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CandidatePlacements {
    pub candidate_generation: u64,
    pub placements: Vec<Placement>,
}

/// One accepted pool resource still live at journal end. Shape A has no
/// release records: tiles stay live like the fresh final displayed resource,
/// so `released_usec` is always None here and teardown is connection
/// retirement, outside the measured workload.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LiveResource {
    pub resource_id: u64,
    pub resource_generation: u64,
    pub released_usec: Option<u64>,
}

fn fresh(mode: &WorkloadMode) -> bool {
    *mode == WorkloadMode::Fresh
}

#[derive(Serialize)]
pub struct Journal {
    pub schema: u64,
    pub wire: String,
    pub source: ImageSource,
    pub probe_bytes: u64,
    #[serde(skip_serializing_if = "fresh")]
    pub workload: WorkloadMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retained_resources: Option<u64>,
    /// Schema-4 paced fresh uploads. Declaration, not proof of scheduled due times.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_msec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ring: Option<RingEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draw: Option<DrawEvidence>,
    pub offers: Vec<Offer>,
    pub uploads: Vec<AcceptedUpload>,
    pub permits: Vec<PermitTiming>,
    pub terminal_usec: Vec<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub placements: Vec<CandidatePlacements>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub live_resources: Vec<LiveResource>,
}

#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
enum Event {
    Start {
        frames: usize,
        wire: String,
        #[serde(default)]
        source: ImageSource,
        #[serde(default)]
        probe_bytes: u64,
        #[serde(default)]
        workload: Option<WorkloadMode>,
        #[serde(default)]
        retained_resources: Option<u64>,
        #[serde(default)]
        interval_msec: Option<u64>,
    },
    CandidatePlacements {
        candidate_generation: u64,
        placements: Vec<Placement>,
    },
    // Parsed only so shape A can reject it explicitly; fields never used.
    #[allow(dead_code)]
    RetainedReleased {
        resource_id: u64,
        resource_generation: u64,
        released_usec: u64,
    },
    UploadOffer {
        upload: Upload,
    },
    UploadAccepted {
        resource_id: u64,
        resource_generation: u64,
        accepted_usec: u64,
    },
    ProbeReleased {
        resource_id: u64,
        resource_generation: u64,
        released_usec: u64,
    },
    Permit {
        generation: u64,
        demand_usec: u64,
        received_usec: u64,
    },
    Offer {
        connection_epoch: u64,
        content_grant_epoch: u64,
        output: u64,
        candidate_generation: u64,
        submitted_monotonic_usec: u64,
    },
    Terminal {
        connection_epoch: u64,
        content_grant_epoch: u64,
        output: u64,
        candidate_generation: u64,
        outcome: String,
        kind: u16,
        reason: u16,
        observed_monotonic_usec: u64,
    },
    End {
        offered: usize,
        completed: usize,
    },
}

#[derive(Deserialize)]
struct Record {
    schema: u64,
    #[serde(flatten)]
    event: Event,
}

fn event(line: &str, schema: u64) -> Result<Event, String> {
    let record: Record = serde_json::from_str(line).map_err(|e| e.to_string())?;
    if record.schema != schema || !(1..=4).contains(&schema) {
        return Err("missing or unsupported journal schema".into());
    }
    Ok(record.event)
}

pub fn collect(journal: &str, expected: usize) -> Result<Vec<Offer>, String> {
    Ok(collect_metrics(journal, expected)?.offers)
}

pub fn collect_metrics(journal: &str, expected: usize) -> Result<Journal, String> {
    if journal.len() > MAX_BYTES as usize
        || !(1..=MAX_RECORDS).contains(&expected)
        || !journal.ends_with('\n')
    {
        return Err("invalid journal size, count or truncated final line".into());
    }
    let mut lines = journal.lines();
    let start = lines.next().ok_or("missing start")?;
    // Dispatch before the legacy event decoder: ring events have their own
    // custody records and must never masquerade as sealed UploadOffer records.
    #[derive(Deserialize)]
    struct Schema {
        schema: u64,
    }
    let start_schema = serde_json::from_str::<Schema>(start)
        .map_err(|e| e.to_string())?
        .schema;
    if start_schema == 5 {
        return ring::collect(journal, expected);
    }
    if start_schema == 6 {
        return draw::collect(journal, expected);
    }
    let schema = serde_json::from_str::<Record>(start)
        .map_err(|e| e.to_string())?
        .schema;
    let (probe_bytes, source, wire, workload, retained, interval_msec) = match event(start, schema)?
    {
        Event::Start {
            frames,
            wire,
            source,
            probe_bytes,
            workload,
            retained_resources,
            interval_msec,
        } if frames == expected
            && matches!(wire.as_str(), "files" | "ipc")
            && (source == ImageSource::Bytes
                || (source == ImageSource::Isb && matches!(schema, 2..=4) && wire == "files")
                || (source == ImageSource::DmaBuf && schema == 2 && wire == "files"))
            && matches!(probe_bytes, 0 | 4194304)
            && (schema >= 2 || probe_bytes == 0)
            && match (schema, workload, retained_resources, interval_msec) {
                // Fresh journals stay schema 1/2 and never declare a pool.
                (1 | 2, None, None, None) => true,
                // Schema 3 is retained-only: files + sealed, explicit pool.
                (3, Some(WorkloadMode::Retained), Some(count), None) => {
                    (2..=8).contains(&count) && wire == "files" && source == ImageSource::Isb
                }
                // External bursty arm only. No legacy IPC/cost acceptance.
                (4, None, None, Some(interval)) => {
                    (1..=10_000).contains(&interval)
                        && wire == "files"
                        && source == ImageSource::Isb
                        && probe_bytes == 0
                }
                _ => false,
            } =>
        {
            (
                probe_bytes,
                source,
                wire,
                workload.unwrap_or_default(),
                retained_resources,
                interval_msec,
            )
        }
        _ => return Err("wrong start count or wire".into()),
    };
    let mut offers = Vec::with_capacity(expected);
    let mut uploads = Vec::new();
    let mut permits = Vec::with_capacity(expected);
    let mut terminal_usec = Vec::with_capacity(expected);
    let mut placements = Vec::new();
    let mut last = 0;
    let mut identity = None;
    // Retained pool: exactly N uploads at candidate_generation 0, ids 2..=N+1
    // generation 1 (id 1 is the reserved transfer probe), one shared even
    // tile width and bar height, all accepted before the first candidate.
    let mut retained_identity = None;
    let mut tile_width = 0u32;
    let mut tile_height = 0u32;
    if let Some(count) = retained {
        for index in 0..count {
            let upload = match event(lines.next().ok_or("missing retained upload")?, schema)? {
                Event::UploadOffer { upload }
                    if upload.candidate_generation == 0
                        && upload.resource_id == index + 2
                        && upload.resource_generation == 1
                        && upload.width > 0
                        && upload.width % 2 == 0
                        && upload.height > 0
                        && (index == 0
                            || (upload.width == tile_width && upload.height == tile_height))
                        && u64::from(upload.width)
                            .checked_mul(u64::from(upload.height))
                            .and_then(|n| n.checked_mul(4))
                            == Some(upload.total_bytes)
                        && upload.begin_usec > 0
                        && upload.begin_usec >= last =>
                {
                    upload
                }
                _ => return Err("missing or invalid retained upload offer".into()),
            };
            if index == 0 {
                tile_width = upload.width;
                tile_height = upload.height;
            }
            let current = (
                upload.connection_epoch,
                upload.content_grant_epoch,
                upload.output,
            );
            if retained_identity
                .replace(current)
                .is_some_and(|old| old != current)
            {
                return Err("retained uploads do not share grant or output".into());
            }
            let accepted_usec =
                match event(lines.next().ok_or("missing retained acceptance")?, schema)? {
                    Event::UploadAccepted {
                        resource_id,
                        resource_generation,
                        accepted_usec,
                    } if resource_id == upload.resource_id
                        && resource_generation == 1
                        && accepted_usec >= upload.begin_usec =>
                    {
                        accepted_usec
                    }
                    _ => return Err("missing or misattributed retained acceptance".into()),
                };
            last = accepted_usec;
            uploads.push(AcceptedUpload {
                purpose: "retained",
                upload,
                accepted_usec,
                released_usec: None,
            });
        }
    }
    for generation in 1..=expected as u64 {
        let first_upload = uploads.len();
        if matches!(schema, 2 | 4) {
            for probe in [true, false] {
                if probe && probe_bytes == 0 {
                    continue;
                }
                let upload = match event(lines.next().ok_or("missing upload offer")?, schema)? {
                    Event::UploadOffer { upload }
                        if upload.candidate_generation == generation
                            && upload.resource_id
                                == if probe {
                                    1
                                } else {
                                    (generation - 1) % 2 + 1 + u64::from(probe_bytes != 0)
                                }
                            && upload.resource_generation
                                == if probe {
                                    generation
                                } else {
                                    (generation - 1) / 2 + 1
                                }
                            && (!probe
                                || (upload.width == 1024
                                    && upload.height == 1024
                                    && upload.total_bytes == probe_bytes))
                            && upload.width > 0
                            && upload.height > 0
                            && u64::from(upload.width)
                                .checked_mul(u64::from(upload.height))
                                .and_then(|n| n.checked_mul(4))
                                == Some(upload.total_bytes)
                            && upload.begin_usec > 0
                            && upload.begin_usec >= last =>
                    {
                        upload
                    }
                    _ => return Err("missing or invalid upload offer".into()),
                };
                let accepted_usec =
                    match event(lines.next().ok_or("missing upload acceptance")?, schema)? {
                        Event::UploadAccepted {
                            resource_id,
                            resource_generation,
                            accepted_usec,
                        } if resource_id == upload.resource_id
                            && resource_generation == upload.resource_generation
                            && accepted_usec >= upload.begin_usec =>
                        {
                            accepted_usec
                        }
                        _ => return Err("missing or misattributed upload acceptance".into()),
                    };
                last = accepted_usec;
                let released_usec = if probe {
                    match event(lines.next().ok_or("missing probe release")?, schema)? {
                        Event::ProbeReleased {
                            resource_id,
                            resource_generation,
                            released_usec,
                        } if resource_id == upload.resource_id
                            && resource_generation == upload.resource_generation
                            && released_usec >= accepted_usec =>
                        {
                            last = released_usec;
                            Some(released_usec)
                        }
                        _ => return Err("missing or invalid probe release".into()),
                    }
                } else {
                    None
                };
                uploads.push(AcceptedUpload {
                    purpose: if probe {
                        "transfer_probe"
                    } else {
                        "presentation"
                    },
                    upload,
                    accepted_usec,
                    released_usec,
                });
            }
        }
        if schema == 3 && probe_bytes != 0 {
            let upload = match event(lines.next().ok_or("missing probe offer")?, schema)? {
                Event::UploadOffer { upload }
                    if upload.candidate_generation == generation
                        && upload.resource_id == 1
                        && upload.resource_generation == generation
                        && upload.width == 1024
                        && upload.height == 1024
                        && upload.total_bytes == probe_bytes
                        && upload.begin_usec > 0
                        && upload.begin_usec >= last =>
                {
                    upload
                }
                _ => return Err("missing or invalid probe offer".into()),
            };
            let accepted_usec =
                match event(lines.next().ok_or("missing probe acceptance")?, schema)? {
                    Event::UploadAccepted {
                        resource_id,
                        resource_generation,
                        accepted_usec,
                    } if resource_id == 1
                        && resource_generation == generation
                        && accepted_usec >= upload.begin_usec =>
                    {
                        accepted_usec
                    }
                    _ => return Err("missing or misattributed probe acceptance".into()),
                };
            let released_usec = match event(lines.next().ok_or("missing probe release")?, schema)? {
                Event::ProbeReleased {
                    resource_id,
                    resource_generation,
                    released_usec,
                } if resource_id == 1
                    && resource_generation == generation
                    && released_usec >= accepted_usec =>
                {
                    last = released_usec;
                    released_usec
                }
                _ => return Err("missing or invalid probe release".into()),
            };
            uploads.push(AcceptedUpload {
                purpose: "transfer_probe",
                upload,
                accepted_usec,
                released_usec: Some(released_usec),
            });
        }
        let received = match event(lines.next().ok_or("missing permit")?, schema)? {
            Event::Permit {
                generation: g,
                demand_usec,
                received_usec,
            } if g == generation
                && demand_usec > 0
                && demand_usec >= last
                && received_usec >= demand_usec =>
            {
                permits.push(PermitTiming {
                    generation,
                    demand_usec,
                    received_usec,
                });
                received_usec
            }
            _ => return Err("missing, out-of-order or invalid permit".into()),
        };
        if schema == 3 {
            // Placements follow the permit and precede the offer; they may
            // only reference the pool accepted above, rotated by one tile.
            let count = retained.unwrap_or_default();
            match event(lines.next().ok_or("missing candidate placements")?, schema)? {
                Event::CandidatePlacements {
                    candidate_generation: placed,
                    placements: placed_resources,
                } if placed == generation
                    && u64::try_from(placed_resources.len()).ok() == Some(count)
                    && placed_resources
                        .iter()
                        .enumerate()
                        .all(|(index, placement)| {
                            placement.resource_id == index as u64 + 2
                                && placement.resource_generation == 1
                                && i64::from(placement.destination_x_px)
                                    == ((index as u64 + generation - 1) % count
                                        * u64::from(tile_width))
                                        as i64
                                && placement.destination_y_px == 0
                        }) =>
                {
                    placements.push(CandidatePlacements {
                        candidate_generation: generation,
                        placements: placed_resources,
                    });
                }
                _ => return Err("missing, wrong or unaccepted candidate placements".into()),
            }
        }
        let offer = match event(lines.next().ok_or("missing offer")?, schema)? {
            Event::Offer {
                connection_epoch,
                content_grant_epoch,
                output,
                candidate_generation,
                submitted_monotonic_usec,
            } if candidate_generation == generation
                && submitted_monotonic_usec >= received
                && ![connection_epoch, content_grant_epoch, output].contains(&0) =>
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
            _ => return Err("missing, out-of-order or invalid offer".into()),
        };
        let current = (
            offer.connection_epoch,
            offer.content_grant_epoch,
            offer.output,
        );
        if identity.replace(current).is_some_and(|old| old != current) {
            return Err("grant or output changed during workload".into());
        }
        if retained_identity.is_some_and(|old| old != current) {
            return Err("retained uploads do not belong to the candidate grant".into());
        }
        for upload in &uploads[first_upload..] {
            if (
                upload.upload.connection_epoch,
                upload.upload.content_grant_epoch,
                upload.upload.output,
                upload.upload.candidate_generation,
            ) != offer.key()
            {
                return Err("upload does not belong to candidate".into());
            }
        }
        match event(lines.next().ok_or("missing terminal")?, schema)? {
            Event::Terminal {
                connection_epoch,
                content_grant_epoch,
                output,
                candidate_generation,
                outcome,
                kind,
                reason,
                observed_monotonic_usec,
            } if (
                connection_epoch,
                content_grant_epoch,
                output,
                candidate_generation,
            ) == offer.key()
                && outcome == "presented"
                && kind == 2
                && reason == 0
                && observed_monotonic_usec >= offer.submitted_monotonic_usec =>
            {
                last = observed_monotonic_usec
            }
            _ => return Err("missing, misattributed or non-presented terminal".into()),
        }
        offers.push(offer);
        terminal_usec.push(last);
    }
    // Shape A: no release records. The pool stays live at End; any
    // retained_released record in this shape is a hard error, not a cue to
    // weaken custody. The live set is derived from the accepted pool below.
    match event(lines.next().ok_or("missing end")?, schema)? {
        Event::End { offered, completed } if offered == expected && completed == expected => {}
        Event::RetainedReleased { .. } => {
            return Err(
                "schema 3 shape A has no retained_released records; tiles stay live at end".into(),
            );
        }
        _ => return Err("wrong end counts".into()),
    }
    if lines.next().is_some() {
        return Err("records after end".into());
    }
    let live_resources = uploads
        .iter()
        .filter(|upload| upload.purpose == "retained")
        .map(|upload| LiveResource {
            resource_id: upload.upload.resource_id,
            resource_generation: upload.upload.resource_generation,
            released_usec: upload.released_usec,
        })
        .collect();
    Ok(Journal {
        schema,
        wire,
        source,
        probe_bytes,
        workload,
        retained_resources: retained,
        interval_msec,
        ring: None,
        draw: None,
        offers,
        uploads,
        permits,
        terminal_usec,
        placements,
        live_resources,
    })
}
