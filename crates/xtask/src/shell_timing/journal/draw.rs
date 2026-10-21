//! Schema 6 is the r11 draw/ring reference-client journal. It proves
//! only what the client journalled: SDK-observed submission and
//! settlement times, candidate terminals, and bursty due times. It is
//! not pixel proof and invents no upload timing the peer did not record.
use super::{
    AcceptedUpload, Event as LegacyEvent, ImageSource, Journal, Offer, PermitTiming, Upload,
    WorkloadMode,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Which upload paths the reference client used on its one connection.
/// The journal spells the ring-only arm `ring`; plan and normalized
/// serde spell the same arm `owned_ring` (see `image_source`).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DrawSource {
    Draw,
    Ring,
    Mixed,
}

impl DrawSource {
    /// Explicit resolution into the runner-wide source vocabulary:
    /// journal `ring` is plan `owned_ring`; `draw` and `mixed` keep
    /// their names. Never compare the raw strings across this boundary.
    pub fn image_source(self) -> ImageSource {
        match self {
            DrawSource::Draw => ImageSource::Draw,
            DrawSource::Ring => ImageSource::OwnedRing,
            DrawSource::Mixed => ImageSource::Mixed,
        }
    }

    /// Whether this source rasterizes through the Session draw helper
    /// (exactly one helper process) versus the ring only (none).
    pub fn uses_draw_helper(self) -> bool {
        matches!(self, DrawSource::Draw | DrawSource::Mixed)
    }
}

/// Value-tile update cadence declared at Start.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DrawWorkload {
    Continuous,
    Retained,
    Idle,
    Bursty,
}

/// Ring channel readiness as schema 6 records it (three fields only;
/// this is not the schema-5 `Ready` registration record).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RingReady6 {
    pub connection_epoch: u64,
    pub content_grant_epoch: u64,
    pub ready_usec: u64,
}

/// One settled upload batch, through whichever source carried it.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct BatchTiming {
    pub resource_id: u64,
    pub resource_generation: u64,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub submitted_usec: u64,
    pub settled_usec: u64,
    pub via_draw: bool,
}

/// One bursty due that triggered a value update.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct ValueDueTiming {
    pub candidate_generation: u64,
    pub due_usec: u64,
    pub observed_usec: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct DrawEvidence {
    pub source: DrawSource,
    pub workload: DrawWorkload,
    pub draw_ready_usec: Option<u64>,
    pub ring_ready: Option<RingReady6>,
    /// Panel tiles first, then value uploads in submission order.
    pub batches: Vec<BatchTiming>,
    pub value_dues: Vec<ValueDueTiming>,
    /// Post-terminal hold on the same connection/grant/leases, from
    /// the final terminal observation to the end of the hold. Native
    /// lifetime proof requires at least one observer tick (>= 1 s).
    pub hold_usec: u64,
}

/// Minimum post-terminal hold: one observer tick.
pub const MIN_HOLD_USEC: u64 = 1_000_000;

#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
enum Event {
    Start {
        frames: usize,
        wire: String,
        source: DrawSource,
        workload: DrawWorkload,
        output_width: u64,
        output_height: u64,
        panel_tiles: u64,
        panel_height: u64,
        tile_width: u64,
        tile_height: u64,
    },
    RingReady {
        connection_epoch: u64,
        content_grant_epoch: u64,
        ready_usec: u64,
    },
    DrawReady {
        content_grant_epoch: u64,
        ready_usec: u64,
    },
    RingSubmitted {
        upload: Upload,
        offer: u64,
        submitted_usec: u64,
    },
    RingSettled {
        offer: u64,
        resource_id: u64,
        resource_generation: u64,
        settled_usec: u64,
    },
    DrawSubmitted {
        transaction: u64,
        resource_id: u64,
        resource_generation: u64,
        width: u32,
        height: u32,
        bytes: u64,
        submitted_usec: u64,
    },
    DrawSettled {
        transaction: u64,
        resource_id: u64,
        resource_generation: u64,
        status: u16,
        reason: u16,
        settled_usec: u64,
    },
    ValueDue {
        candidate_generation: u64,
        due_usec: u64,
        observed_usec: u64,
    },
    SettledEnd {
        settled_usec: u64,
        held_until_usec: u64,
    },
    Failed {
        #[allow(dead_code)]
        error: String,
    },
}

enum Parsed {
    Draw(Event),
    Legacy(LegacyEvent),
}

fn parse(line: &str) -> Result<Parsed, String> {
    #[derive(Deserialize)]
    struct Record<T> {
        schema: u64,
        #[serde(flatten)]
        event: T,
    }
    if let Ok(record) = serde_json::from_str::<Record<Event>>(line) {
        if record.schema != 6 {
            return Err("mixed draw journal schemas".into());
        }
        return Ok(Parsed::Draw(record.event));
    }
    let record: Record<LegacyEvent> = serde_json::from_str(line).map_err(|e| e.to_string())?;
    if record.schema != 6 {
        return Err("mixed draw journal schemas".into());
    }
    Ok(Parsed::Legacy(record.event))
}

const BURST_INTERVAL_USEC: u64 = 250_000;

struct Cursor<'a> {
    lines: std::str::Lines<'a>,
    last: u64,
    seen: HashSet<(u64, u64)>,
    generations: HashMap<u64, u64>,
}

impl Cursor<'_> {
    fn next(&mut self) -> Result<Parsed, String> {
        parse(self.lines.next().ok_or("incomplete draw journal")?)
    }

    fn clock(&mut self, usec: u64) -> Result<(), String> {
        if usec < self.last {
            return Err("draw journal timestamps regress".into());
        }
        self.last = usec;
        Ok(())
    }

    fn register(&mut self, batch: &BatchTiming) -> Result<(), String> {
        if !self
            .seen
            .insert((batch.resource_id, batch.resource_generation))
        {
            return Err("duplicate resource identity in draw journal".into());
        }
        match self.generations.get(&batch.resource_id) {
            Some(&previous) if batch.resource_generation <= previous => {
                Err("resource generation did not increase for its id".into())
            }
            _ => {
                self.generations
                    .insert(batch.resource_id, batch.resource_generation);
                Ok(())
            }
        }
    }

    /// One submitted/settled pair; `via_draw` selects the source shape.
    fn pair(&mut self, via_draw: bool) -> Result<BatchTiming, String> {
        let batch = if via_draw {
            let (transaction, resource_id, resource_generation, width, height, bytes, submitted) =
                match self.next()? {
                    Parsed::Draw(Event::DrawSubmitted {
                        transaction,
                        resource_id,
                        resource_generation,
                        width,
                        height,
                        bytes,
                        submitted_usec,
                    }) if transaction > 0 && resource_id > 0 && resource_generation > 0 => (
                        transaction,
                        resource_id,
                        resource_generation,
                        width,
                        height,
                        bytes,
                        submitted_usec,
                    ),
                    Parsed::Draw(Event::Failed { .. }) => {
                        return Err("reference client reported failure".into());
                    }
                    _ => return Err("expected draw submission".into()),
                };
            self.clock(submitted)?;
            match self.next()? {
                Parsed::Draw(Event::DrawSettled {
                    transaction: settled_tx,
                    resource_id: id,
                    resource_generation: generation,
                    status: 2,
                    reason: 0,
                    settled_usec,
                }) if settled_tx == transaction
                    && id == resource_id
                    && generation == resource_generation =>
                {
                    self.clock(settled_usec)?;
                    BatchTiming {
                        resource_id,
                        resource_generation,
                        width,
                        height,
                        bytes,
                        submitted_usec: submitted,
                        settled_usec,
                        via_draw: true,
                    }
                }
                Parsed::Draw(Event::Failed { .. }) => {
                    return Err("reference client reported failure".into());
                }
                _ => return Err("draw batch did not settle as accepted".into()),
            }
        } else {
            let (upload, offer, submitted) = match self.next()? {
                Parsed::Draw(Event::RingSubmitted {
                    upload,
                    offer,
                    submitted_usec,
                }) if offer > 0
                    && upload.resource_id > 0
                    && upload.resource_generation > 0
                    && upload.begin_usec == submitted_usec
                    && u64::from(upload.width)
                        .checked_mul(u64::from(upload.height))
                        .and_then(|n| n.checked_mul(4))
                        == Some(upload.total_bytes) =>
                {
                    (upload, offer, submitted_usec)
                }
                Parsed::Draw(Event::Failed { .. }) => {
                    return Err("reference client reported failure".into());
                }
                _ => return Err("expected ring submission".into()),
            };
            self.clock(submitted)?;
            match self.next()? {
                Parsed::Draw(Event::RingSettled {
                    offer: settled_offer,
                    resource_id: id,
                    resource_generation: generation,
                    settled_usec,
                }) if settled_offer == offer
                    && id == upload.resource_id
                    && generation == upload.resource_generation =>
                {
                    self.clock(settled_usec)?;
                    BatchTiming {
                        resource_id: id,
                        resource_generation: generation,
                        width: upload.width,
                        height: upload.height,
                        bytes: upload.total_bytes,
                        submitted_usec: submitted,
                        settled_usec,
                        via_draw: false,
                    }
                }
                Parsed::Draw(Event::Failed { .. }) => {
                    return Err("reference client reported failure".into());
                }
                _ => return Err("ring offer did not settle".into()),
            }
        };
        self.register(&batch)?;
        Ok(batch)
    }
}

pub(super) fn collect(journal: &str, expected: usize) -> Result<Journal, String> {
    let mut cursor = Cursor {
        lines: journal.lines(),
        last: 0,
        seen: HashSet::new(),
        generations: HashMap::new(),
    };
    // Readiness records precede Start in the reference client.
    let mut draw_ready: Option<(u64, u64)> = None;
    let mut ring_ready: Option<RingReady6> = None;
    let mut start: Option<Event> = None;
    for _ in 0..3 {
        match cursor.next()? {
            Parsed::Draw(Event::DrawReady {
                content_grant_epoch,
                ready_usec,
            }) if content_grant_epoch > 0 && draw_ready.is_none() => {
                cursor.clock(ready_usec)?;
                draw_ready = Some((content_grant_epoch, ready_usec));
            }
            Parsed::Draw(Event::RingReady {
                connection_epoch,
                content_grant_epoch,
                ready_usec,
            }) if connection_epoch > 0 && content_grant_epoch > 0 && ring_ready.is_none() => {
                cursor.clock(ready_usec)?;
                ring_ready = Some(RingReady6 {
                    connection_epoch,
                    content_grant_epoch,
                    ready_usec,
                });
            }
            Parsed::Draw(event @ Event::Start { .. }) => {
                start = Some(event);
                break;
            }
            Parsed::Draw(Event::Failed { .. }) => {
                return Err("reference client reported failure".into());
            }
            _ => return Err("draw journal must open with readiness then start".into()),
        }
    }
    let (source, workload, output_width, panel_tiles, panel_height, tile_width, tile_height) =
        match start.ok_or("missing draw start")? {
            Event::Start {
                frames,
                wire,
                source,
                workload,
                output_width,
                output_height,
                panel_tiles,
                panel_height,
                tile_width,
                tile_height,
            } if frames == expected
                && wire == "files"
                && output_width > 0
                && output_height >= panel_height
                && panel_tiles >= 1
                && panel_height > 0
                && tile_width > 0
                && tile_height == panel_height =>
            {
                (
                    source,
                    workload,
                    output_width,
                    panel_tiles,
                    panel_height,
                    tile_width,
                    tile_height,
                )
            }
            _ => return Err("wrong draw start count, wire or geometry".into()),
        };
    match source {
        DrawSource::Draw if draw_ready.is_none() || ring_ready.is_some() => {
            return Err("draw source requires draw readiness only".into());
        }
        DrawSource::Ring if ring_ready.is_none() || draw_ready.is_some() => {
            return Err("ring source requires ring readiness only".into());
        }
        DrawSource::Mixed if ring_ready.is_none() || draw_ready.is_none() => {
            return Err("mixed source requires both readiness records".into());
        }
        _ => {}
    }
    if let (Some((grant_epoch, _)), Some(ready)) = (draw_ready, ring_ready)
        && grant_epoch != ready.content_grant_epoch
    {
        return Err("readiness grant epochs disagree".into());
    }
    let connection_epoch = ring_ready.map_or(0, |r| r.connection_epoch);
    let content_grant_epoch = ring_ready.map_or_else(
        || draw_ready.map_or(0, |(g, _)| g),
        |r| r.content_grant_epoch,
    );

    let mut result = Journal {
        schema: 6,
        wire: "files".into(),
        source: source.image_source(),
        probe_bytes: 0,
        workload: WorkloadMode::Fresh,
        retained_resources: None,
        interval_msec: None,
        ring: None,
        draw: None,
        offers: Vec::with_capacity(expected),
        uploads: Vec::new(),
        permits: Vec::with_capacity(expected),
        terminal_usec: Vec::with_capacity(expected),
        placements: Vec::new(),
        live_resources: Vec::new(),
    };
    let mut batches: Vec<BatchTiming> = Vec::new();
    let mut value_dues: Vec<ValueDueTiming> = Vec::new();
    let mut upload_record = |result: &mut Journal, batch: &BatchTiming| {
        result.uploads.push(AcceptedUpload {
            purpose: "presentation",
            upload: Upload {
                connection_epoch,
                content_grant_epoch,
                output: 0,
                candidate_generation: 0,
                resource_id: batch.resource_id,
                resource_generation: batch.resource_generation,
                width: batch.width,
                height: batch.height,
                total_bytes: batch.bytes,
                begin_usec: batch.submitted_usec,
            },
            accepted_usec: batch.settled_usec,
            released_usec: None,
        });
    };

    // Panel tiles: exactly panel_tiles settled batches before frame 1.
    let mut panel_width = 0u64;
    for _ in 0..panel_tiles {
        let via_draw = matches!(source, DrawSource::Draw);
        let batch = cursor.pair(via_draw)?;
        if u64::from(batch.height) != panel_height {
            return Err("panel tile height mismatch".into());
        }
        panel_width = panel_width
            .checked_add(u64::from(batch.width))
            .ok_or("panel width overflow")?;
        upload_record(&mut result, &batch);
        batches.push(batch);
    }
    if panel_width != output_width {
        return Err("panel tiles do not cover the output width".into());
    }

    let mut value_resource: Option<u64> = None;
    for frame in 1..=expected as u64 {
        // Frame section: [value_due?] [value upload pair?] permit offer
        // terminal. Consume the optional due and upload by peeking at
        // the parsed stream one record at a time.
        let mut due: Option<ValueDueTiming> = None;
        let mut updated = false;
        let mut record = cursor.next()?;
        if let Parsed::Draw(Event::ValueDue {
            candidate_generation,
            due_usec,
            observed_usec,
        }) = record
        {
            if candidate_generation != frame || observed_usec < due_usec {
                return Err("invalid bursty value due".into());
            }
            if workload != DrawWorkload::Bursty {
                return Err("value_due outside the bursty workload".into());
            }
            if let Some(previous) = value_dues.last() {
                let gap = due_usec
                    .checked_sub(previous.due_usec)
                    .ok_or("bursty dues regress")?;
                if gap < BURST_INTERVAL_USEC || gap % BURST_INTERVAL_USEC != 0 {
                    return Err("bursty dues are not on the 4 Hz grid".into());
                }
            }
            cursor.clock(observed_usec)?;
            due = Some(ValueDueTiming {
                candidate_generation,
                due_usec,
                observed_usec,
            });
            record = cursor.next()?;
        }
        // A submission here is this frame's value update. Hand it back
        // by parsing the pair inline from the consumed submission.
        let permit = match record {
            Parsed::Draw(Event::DrawSubmitted {
                transaction,
                resource_id,
                resource_generation,
                width,
                height,
                bytes,
                submitted_usec,
            }) => {
                cursor.clock(submitted_usec)?;
                let batch = match cursor.next()? {
                    Parsed::Draw(Event::DrawSettled {
                        transaction: settled_tx,
                        resource_id: id,
                        resource_generation: generation,
                        status: 2,
                        reason: 0,
                        settled_usec,
                    }) if settled_tx == transaction
                        && id == resource_id
                        && generation == resource_generation =>
                    {
                        cursor.clock(settled_usec)?;
                        BatchTiming {
                            resource_id,
                            resource_generation,
                            width,
                            height,
                            bytes,
                            submitted_usec,
                            settled_usec,
                            via_draw: true,
                        }
                    }
                    Parsed::Draw(Event::Failed { .. }) => {
                        return Err("reference client reported failure".into());
                    }
                    _ => return Err("draw batch did not settle as accepted".into()),
                };
                if source == DrawSource::Ring {
                    return Err("draw value update in a ring-only journal".into());
                }
                cursor.register(&batch)?;
                finish_value(
                    &batch,
                    tile_width,
                    tile_height,
                    &mut value_resource,
                    &mut upload_record,
                    &mut result,
                    &mut batches,
                )?;
                updated = true;
                match cursor.next()? {
                    Parsed::Legacy(LegacyEvent::Permit {
                        generation,
                        demand_usec,
                        received_usec,
                    }) => (generation, demand_usec, received_usec),
                    _ => return Err("missing draw permit after value update".into()),
                }
            }
            Parsed::Draw(Event::RingSubmitted {
                upload,
                offer,
                submitted_usec,
            }) => {
                if upload.begin_usec != submitted_usec
                    || u64::from(upload.width)
                        .checked_mul(u64::from(upload.height))
                        .and_then(|n| n.checked_mul(4))
                        != Some(upload.total_bytes)
                {
                    return Err("invalid ring submission in draw journal".into());
                }
                cursor.clock(submitted_usec)?;
                let batch = match cursor.next()? {
                    Parsed::Draw(Event::RingSettled {
                        offer: settled_offer,
                        resource_id: id,
                        resource_generation: generation,
                        settled_usec,
                    }) if settled_offer == offer
                        && id == upload.resource_id
                        && generation == upload.resource_generation =>
                    {
                        cursor.clock(settled_usec)?;
                        BatchTiming {
                            resource_id: id,
                            resource_generation: generation,
                            width: upload.width,
                            height: upload.height,
                            bytes: upload.total_bytes,
                            submitted_usec,
                            settled_usec,
                            via_draw: false,
                        }
                    }
                    Parsed::Draw(Event::Failed { .. }) => {
                        return Err("reference client reported failure".into());
                    }
                    _ => return Err("ring offer did not settle".into()),
                };
                if source == DrawSource::Draw {
                    return Err("ring value update in a draw-only journal".into());
                }
                cursor.register(&batch)?;
                finish_value(
                    &batch,
                    tile_width,
                    tile_height,
                    &mut value_resource,
                    &mut upload_record,
                    &mut result,
                    &mut batches,
                )?;
                updated = true;
                match cursor.next()? {
                    Parsed::Legacy(LegacyEvent::Permit {
                        generation,
                        demand_usec,
                        received_usec,
                    }) => (generation, demand_usec, received_usec),
                    _ => return Err("missing draw permit after value update".into()),
                }
            }
            Parsed::Legacy(LegacyEvent::Permit {
                generation,
                demand_usec,
                received_usec,
            }) => (generation, demand_usec, received_usec),
            Parsed::Draw(Event::Failed { .. }) => {
                return Err("reference client reported failure".into());
            }
            _ => return Err("unexpected record in draw frame section".into()),
        };
        let (permit_generation, demand_usec, received_usec) = permit;
        if permit_generation == 0 || received_usec < demand_usec {
            return Err("invalid draw permit timing".into());
        }
        cursor.clock(received_usec)?;
        result.permits.push(PermitTiming {
            generation: permit_generation,
            demand_usec,
            received_usec,
        });
        if let Some(due) = due {
            if !updated {
                return Err("bursty due without a value update".into());
            }
            value_dues.push(due);
        }
        let expect_update = match workload {
            DrawWorkload::Continuous => true,
            DrawWorkload::Retained => frame % 8 == 1,
            DrawWorkload::Idle => frame == 1,
            DrawWorkload::Bursty => frame == 1 || due.is_some(),
        };
        if updated != expect_update {
            return Err("value update does not match the declared workload".into());
        }

        let offer = match cursor.next()? {
            Parsed::Legacy(LegacyEvent::Offer {
                connection_epoch: conn,
                content_grant_epoch: grant,
                output,
                candidate_generation,
                submitted_monotonic_usec,
            }) if conn > 0
                && grant > 0
                && output > 0
                && candidate_generation == frame
                && submitted_monotonic_usec >= received_usec =>
            {
                cursor.clock(submitted_monotonic_usec)?;
                Offer {
                    connection_epoch: conn,
                    content_grant_epoch: grant,
                    output,
                    candidate_generation,
                    submitted_monotonic_usec,
                    outcome: "presented".into(),
                }
            }
            _ => return Err("missing or invalid draw candidate offer".into()),
        };
        match cursor.next()? {
            Parsed::Legacy(LegacyEvent::Terminal {
                connection_epoch,
                content_grant_epoch,
                output,
                candidate_generation,
                outcome,
                kind: 2,
                reason: 0,
                observed_monotonic_usec,
            }) if (
                connection_epoch,
                content_grant_epoch,
                output,
                candidate_generation,
            ) == offer.key()
                && outcome == "presented" =>
            {
                cursor.clock(observed_monotonic_usec)?;
                result.terminal_usec.push(observed_monotonic_usec);
            }
            _ => return Err("missing or invalid draw candidate presentation".into()),
        }
        result.offers.push(offer);
    }
    let hold_usec = match cursor.next()? {
        Parsed::Draw(Event::SettledEnd {
            settled_usec,
            held_until_usec,
        }) if settled_usec >= cursor.last && held_until_usec >= settled_usec => {
            cursor.clock(held_until_usec)?;
            held_until_usec - settled_usec
        }
        _ => return Err("missing settled_end after the final terminal".into()),
    };
    if hold_usec < MIN_HOLD_USEC {
        return Err("post-terminal hold shorter than one observer tick".into());
    }
    match cursor.next()? {
        Parsed::Legacy(LegacyEvent::End { offered, completed })
            if offered == expected && completed == expected => {}
        _ => return Err("wrong draw end count".into()),
    }
    if cursor.lines.next().is_some() {
        return Err("records after draw end".into());
    }
    result.draw = Some(DrawEvidence {
        source,
        workload,
        draw_ready_usec: draw_ready.map(|(_, usec)| usec),
        ring_ready,
        batches,
        value_dues,
        hold_usec,
    });
    Ok(result)
}

fn finish_value(
    batch: &BatchTiming,
    tile_width: u64,
    tile_height: u64,
    value_resource: &mut Option<u64>,
    upload_record: &mut impl FnMut(&mut Journal, &BatchTiming),
    result: &mut Journal,
    batches: &mut Vec<BatchTiming>,
) -> Result<(), String> {
    if u64::from(batch.width) != tile_width || u64::from(batch.height) != tile_height {
        return Err("value tile geometry mismatch".into());
    }
    match value_resource {
        None => *value_resource = Some(batch.resource_id),
        Some(id) if *id != batch.resource_id => {
            return Err("value tile resource id changed".into());
        }
        _ => {}
    }
    upload_record(result, batch);
    batches.push(batch.clone());
    Ok(())
}
