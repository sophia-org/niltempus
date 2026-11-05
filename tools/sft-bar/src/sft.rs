//! Explicit r13 reference lane: the same CPU pixels, one complete frame commit.
use super::*;
use sophia_shell_client::{FrameStream, FrameStreamState, JournalRecord};
use sophia_shell_protocol::shell_files::{
    ShellFrameCommit, ShellFrameRecord, ShellFrameRetire, ShellFrameStreamKey,
};
use std::io::Write;

pub const SCHEMA: u8 = 8;

#[derive(serde::Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Event {
    #[serde(rename = "frame_retired")]
    Retired {
        candidate_generation: u64,
        resource_id: u64,
        resource_generation: u64,
        observed_monotonic_usec: u64,
    },
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
    #[serde(rename = "frame_closed")]
    Closed {
        stream_id: u64,
        observed_monotonic_usec: u64,
    },
}

fn emit(event: Event) -> Result<()> {
    #[derive(serde::Serialize)]
    struct Record {
        schema: u8,
        #[serde(flatten)]
        event: Event,
    }
    let mut bytes = [0; 1024];
    let mut buffer = std::io::Cursor::new(bytes.as_mut_slice());
    serde_json::to_writer(
        &mut buffer,
        &Record {
            schema: SCHEMA,
            event,
        },
    )?;
    writeln!(buffer)?;
    let len = buffer.position() as usize;
    let mut output = std::io::stdout().lock();
    output.write_all(&bytes[..len])?;
    output.flush()?;
    Ok(())
}

#[derive(Debug)]
enum Observed {
    Frame(ShellFrameRecord),
    Content(TransactionId, ShellContentRecord),
}

fn take(client: &mut ShellConnection) -> Result<Option<Observed>> {
    let Some(event) = client.journal_event() else {
        return Ok(None);
    };
    let record = match event.record {
        JournalRecord::Frame(record) => Observed::Frame(record.clone()),
        JournalRecord::Content(transaction, record) => {
            Observed::Content(transaction, record.clone())
        }
        _ => return Err("unexpected reference-peer journal event".into()),
    };
    client.consume_journal_event()?;
    Ok(Some(record))
}

fn receive(client: &mut ShellConnection) -> Result<Observed> {
    let deadline = Instant::now() + WAIT;
    loop {
        client.poll_io()?;
        if let Some(record) = take(client)? {
            return Ok(record);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("SFT response timeout".into());
        }
        client.wait_for_io(left)?;
    }
}

pub struct Peer {
    stream: FrameStream,
    key: ShellFrameStreamKey,
}

impl Peer {
    pub fn open(
        client: &mut ShellConnection,
        grant: ContentGrant,
        output: ContentOutputId,
        allocation: &ContentAllocationResult,
        facts_generation: u64,
    ) -> Result<Self> {
        client.set_automatic_event_delivery(false)?;
        let key = ShellFrameStreamKey {
            grant,
            output,
            allocation: allocation.allocation,
            facts_generation,
            stream_id: 100,
        };
        let mut stream = FrameStream::new();
        stream.open(client, TransactionId::from_raw(100), key)?;
        let Observed::Frame(record @ ShellFrameRecord::Opened { .. }) = receive(client)? else {
            return Err("SFT Open refused or unexpected reply".into());
        };
        stream.observe(&record)?;
        if !stream.has_credit() {
            return Err("SFT Open returned no credit".into());
        }
        emit(Event::Opened {
            stream_id: key.stream_id,
            observed_monotonic_usec: clock_usec(),
        })?;
        Ok(Self { stream, key })
    }

    pub fn frame(
        &mut self,
        client: &mut ShellConnection,
        generation: u64,
        allocation: &ContentAllocationResult,
        counters: &mut RetireCounters,
    ) -> Result<()> {
        let resource = resource_for_frame(generation);
        let transaction = TransactionId::from_raw(100 + 2 * generation);
        let predecessor = (generation > 1).then(|| ShellFrameRetire {
            transaction: TransactionId::from_raw(101 + 2 * generation),
            resource: resource_for_frame(generation - 1),
        });
        let mut record = ShellFrameRecord::Commit(ShellFrameCommit {
            transaction,
            candidate: ContentCandidate {
                grant: self.key.grant,
                output: self.key.output,
                candidate_generation: generation,
                facts_generation: self.key.facts_generation,
                pacing_permit: self.key.stream_id,
                interaction_generation: 1,
                surfaces: vec![ContentSurface {
                    allocation: allocation.allocation,
                    scale_generation: allocation.scale_generation,
                    role: 1,
                    edge: 1,
                    margins: allocation.margins,
                    reservation_extent: 0,
                    parent_surface_index: u16::MAX,
                    anchor_parent_rect: ContentPixelRect::default(),
                }],
                placements: vec![ContentPlacement {
                    resource,
                    surface_index: 0,
                    destination_x_px: 0,
                    destination_y_px: 0,
                }],
                targets: vec![],
            },
            image: None,
            retire: predecessor,
        });
        let (width, height) = (allocation.pixel.width, allocation.pixel.height);
        let length = byte_len(width, height).ok_or("SFT upload size overflow")?;
        let upload = RingUpload {
            resource: resource.id,
            resource_generation: resource.generation,
            allocation: allocation.allocation.id,
            allocation_generation: allocation.allocation.generation,
            output: self.key.output.id,
            output_generation: self.key.output.generation,
            topology_generation: self.key.facts_generation,
            width,
            height,
        };
        let mut fill_start_usec = 0;
        let mut begin_usec = 0;
        let (_, offer) =
            self.stream
                .submit_ring(client, &mut record, upload, Instant::now(), |pixels| {
                    fill_start_usec = clock_usec();
                    fill_cpu(pixels, generation);
                    begin_usec = clock_usec();
                })?;
        eprintln!(
            "gpu_bar_prepare schema=1 arm=owned_ring generation={generation} resource_id={} start_usec={fill_start_usec} end_usec={begin_usec}",
            resource.id
        );
        let slot = u64::from(
            client
                .image_ring_offer(offer)
                .ok_or("SFT source offer lost")?
                .slot,
        );
        emit_ring(
            SCHEMA,
            ring_journal::Event::RingOffer {
                upload: Upload {
                    connection_epoch: self.key.grant.connection_epoch,
                    content_grant_epoch: self.key.grant.content_grant_epoch,
                    output: self.key.output.id,
                    candidate_generation: generation,
                    resource_id: resource.id,
                    resource_generation: resource.generation,
                    width,
                    height,
                    total_bytes: length,
                    begin_usec,
                },
                offer,
                slot,
                slot_generation: None,
                fill_start_usec,
            },
        )?;
        super::emit(
            SCHEMA,
            super::Event::Offer {
                connection_epoch: self.key.grant.connection_epoch,
                content_grant_epoch: self.key.grant.content_grant_epoch,
                output: self.key.output.id,
                candidate_generation: generation,
                submitted_monotonic_usec: clock_usec(),
            },
        )?;
        let deadline = Instant::now() + WAIT;
        let mut presented = false;
        let mut accepted = false;
        let mut released = predecessor.is_none();
        let mut source_released = false;
        loop {
            client.poll_io()?;
            let progressed = if let Some(event) = take(client)? {
                match event {
                    Observed::Frame(record @ ShellFrameRecord::Outcome { .. }) => {
                        self.stream.observe(&record)?;
                        let ShellFrameRecord::Outcome { outcome, .. } = record else {
                            unreachable!()
                        };
                        match outcome.kind {
                            1 => emit(Event::Prepared {
                                candidate_generation: generation,
                                observed_monotonic_usec: clock_usec(),
                            })?,
                            2 if outcome.presentation_epoch != 0 => {
                                presented = true;
                                super::emit(
                                    SCHEMA,
                                    super::Event::Terminal {
                                        connection_epoch: self.key.grant.connection_epoch,
                                        content_grant_epoch: self.key.grant.content_grant_epoch,
                                        output: self.key.output.id,
                                        candidate_generation: generation,
                                        outcome: "presented",
                                        kind: outcome.kind,
                                        reason: outcome.reason,
                                        observed_monotonic_usec: clock_usec(),
                                    },
                                )?;
                            }
                            _ => return Err("SFT candidate was not presented".into()),
                        }
                    }
                    Observed::Content(_, ShellContentRecord::ResourceStatus(value)) => {
                        if accepted
                            || value.grant != self.key.grant
                            || value.resource != resource
                            || value.status != 2
                            || value.reason != 0
                            || value.admitted_bytes != length
                        {
                            return Err("SFT resource acceptance mismatch".into());
                        }
                        accepted = true;
                        emit_ring(
                            SCHEMA,
                            ring_journal::Event::RingAccepted {
                                offer,
                                resource_id: resource.id,
                                resource_generation: resource.generation,
                                accepted_usec: clock_usec(),
                            },
                        )?;
                    }
                    Observed::Content(transaction, ShellContentRecord::ResourceReleased(value)) => {
                        let expected = predecessor.ok_or("SFT unexpected predecessor release")?;
                        if !presented
                            || released
                            || transaction != expected.transaction
                            || value.grant != self.key.grant
                            || value.resource != expected.resource
                            || value.reason != 0
                        {
                            return Err("SFT predecessor release mismatch".into());
                        }
                        released = true;
                        counters.predecessor_releases_received += 1;
                        emit(Event::Retired {
                            candidate_generation: generation,
                            resource_id: value.resource.id,
                            resource_generation: value.resource.generation,
                            observed_monotonic_usec: clock_usec(),
                        })?;
                    }
                    other => {
                        return Err(format!(
                            "SFT frame refused or unexpected journal record: {other:?}"
                        )
                        .into());
                    }
                }
                true
            } else {
                false
            };
            if !source_released {
                let info = client
                    .image_ring_offer(offer)
                    .ok_or("SFT source custody lost")?;
                if info.resource != resource.id || info.resource_generation != resource.generation {
                    return Err("SFT source identity changed".into());
                }
                match info.state {
                    RingOfferState::Settled
                        if info.status == Some(ResourceStatusKind::Accepted) =>
                    {
                        emit_ring(
                            SCHEMA,
                            ring_journal::Event::SlotReleased {
                                offer,
                                slot,
                                slot_generation: None,
                                resource_id: resource.id,
                                resource_generation: resource.generation,
                                released_usec: clock_usec(),
                            },
                        )?;
                        client
                            .image_forget(offer)
                            .map_err(|e| format!("SFT forget: {e:?}"))?;
                        source_released = true;
                    }
                    RingOfferState::Pending => {}
                    _ => return Err("SFT source did not settle successfully".into()),
                }
            }
            if accepted && presented && released && source_released && self.stream.has_credit() {
                return Ok(());
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("SFT frame settlement timeout".into());
            }
            if !progressed {
                client.wait_for_io(left)?;
            }
        }
    }

    pub fn close(&mut self, client: &mut ShellConnection, frames: u64) -> Result<()> {
        self.stream
            .close(client, TransactionId::from_raw(102 + 2 * frames))?;
        let Observed::Frame(record @ ShellFrameRecord::Closed { .. }) = receive(client)? else {
            return Err("SFT close did not settle".into());
        };
        self.stream.observe(&record)?;
        if self.stream.state() != FrameStreamState::Closed {
            return Err("SFT work remains after close".into());
        }
        emit(Event::Closed {
            stream_id: self.key.stream_id,
            observed_monotonic_usec: clock_usec(),
        })
    }
}
