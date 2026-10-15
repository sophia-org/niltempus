//! Validate the paced peer's complete journal before exporting any samples.
use super::{MAX_BYTES, MAX_RECORDS, Offer};
use serde::{Deserialize, Serialize};

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

#[derive(Serialize)]
pub struct Journal {
    pub schema: u64,
    pub probe_bytes: u64,
    pub offers: Vec<Offer>,
    pub uploads: Vec<AcceptedUpload>,
    pub permits: Vec<PermitTiming>,
    pub terminal_usec: Vec<u64>,
}

#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
enum Event {
    Start {
        frames: usize,
        wire: String,
        #[serde(default)]
        probe_bytes: u64,
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
    if record.schema != schema || !matches!(schema, 1 | 2) {
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
    let schema = serde_json::from_str::<Record>(start)
        .map_err(|e| e.to_string())?
        .schema;
    let probe_bytes = match event(start, schema)? {
        Event::Start {
            frames,
            wire,
            probe_bytes,
        } if frames == expected
            && matches!(wire.as_str(), "files" | "ipc")
            && matches!(probe_bytes, 0 | 4194304)
            && (schema == 2 || probe_bytes == 0) =>
        {
            probe_bytes
        }
        _ => return Err("wrong start count or wire".into()),
    };
    let mut offers = Vec::with_capacity(expected);
    let mut uploads = Vec::new();
    let mut permits = Vec::with_capacity(expected);
    let mut terminal_usec = Vec::with_capacity(expected);
    let mut last = 0;
    let mut identity = None;
    for generation in 1..=expected as u64 {
        let first_upload = uploads.len();
        if schema == 2 {
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
    match event(lines.next().ok_or("missing end")?, schema)? {
        Event::End { offered, completed } if offered == expected && completed == expected => {}
        _ => return Err("wrong end counts".into()),
    }
    if lines.next().is_some() {
        return Err("records after end".into());
    }
    Ok(Journal {
        schema,
        probe_bytes,
        offers,
        uploads,
        permits,
        terminal_usec,
    })
}
