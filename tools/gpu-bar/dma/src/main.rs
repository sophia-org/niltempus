//! Arm G: the comparator paced peer's fresh workload over files at r10 with
//! a producer-completed GPU DMA-BUF source. Records, resource identities,
//! cadence and journal match ../../historical (see PROVENANCE.txt); only the
//! source differs. There is no IPC, byte, sealed, probe or retained path.
use sophia_image_producer::ResourceStatusKind;
use sophia_shell_client::{
    IMAGE_CAPABILITY_DMA_BUF, IMAGE_CAPABILITY_OWNED_RING, ImageChannelState, OfferState,
    ProducerMetadata, ProducerRoleBounds, ShellClientError, ShellClientOptions, ShellConnection,
    ShellImageConfig,
};
use sophia_shell_protocol::*;
use std::{
    io::Read,
    os::{fd::FromRawFd, unix::net::UnixStream},
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const WAIT: Duration = Duration::from_secs(5);
const JOURNAL_SCHEMA: u8 = 2;
#[path = "../../historical/src/shell_timing_peer/journal.rs"]
#[allow(dead_code)] // arm G emits no probe or retained-placement records
mod journal;
use journal::{Event, Upload};
#[path = "../../common/gpu_canvas.rs"]
mod canvas;

struct Wire {
    client: ShellConnection,
    _relay: sophia_9p_client::pipeline::Pipeline,
}

/// Validate and adopt the exclusive launch channel, as Lom does.
fn adopt(fd: i32) -> Result<UnixStream> {
    if fd < 3 {
        return Err("invalid launch descriptor".into());
    }
    // SAFETY: probed only; nothing has opened a descriptor yet that could reuse it.
    let flags = rustix::io::fcntl_getfd(unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) })?;
    // SAFETY: the launch contract transfers this inherited fd exclusively to us.
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    rustix::io::fcntl_setfd(&stream, flags | rustix::io::FdFlags::CLOEXEC)?;
    Ok(stream)
}

impl Wire {
    fn connect() -> Result<Self> {
        if std::env::var_os("SOPHIA_SHELL_SOCKET").is_some() {
            return Err("ambiguous shell wire".into());
        }
        let socket = std::env::var_os("SOPHIA_SHELL_9P_SOCKET").ok_or("missing file endpoint")?;
        let fd = std::env::var_os("SOPHIA_LAUNCH_AUTH_FD")
            .ok_or("DMA source requires an authenticated file launch")?;
        let relay = adopt(fd.to_str().ok_or("non-UTF8 launch descriptor")?.parse()?)?;
        let mut auth = sophia_launch_client::Client::with_timeout(
            UnixStream::connect(&socket)?,
            relay,
            b"shell",
            b"bar",
            WAIT,
        )?;
        while !auth.ready() {
            auth.poll()?;
            if !auth.ready() {
                auth.wait_for_io(Duration::from_millis(10))?;
            }
        }
        let attached = auth.into_authenticated()?;
        // r10 needs the owned-ring capability; the DMA one adds the source.
        let options = ShellClientOptions {
            minimum_revision: 10,
            maximum_revision: 10,
            required_capabilities: SOPHIA_SHELL_CAPABILITY_DESCRIPTOR_SWITCHER
                | SOPHIA_SHELL_CAPABILITY_CONTENT_SURFACE
                | SOPHIA_SHELL_CAPABILITY_CONTENT_DISCRETE_INPUT
                | IMAGE_CAPABILITY_OWNED_RING
                | IMAGE_CAPABILITY_DMA_BUF,
            handshake_timeout: WAIT,
        };
        Ok(Self {
            client: ShellConnection::from_attached(attached.role, attached.root, options)?,
            _relay: attached.relay,
        })
    }

    fn send(&mut self, id: u64, records: &[ShellContentRecord]) -> Result<()> {
        let transaction = TransactionId::from_raw(id);
        let deadline = Instant::now() + WAIT;
        loop {
            let result = if records.len() == 1 {
                self.client.enqueue_content(transaction, &records[0])
            } else {
                self.client.enqueue_content_group(transaction, records)
            };
            match result {
                Ok(()) => break,
                Err(ShellClientError::QueueSaturated) if Instant::now() < deadline => {
                    self.client
                        .wait_for_io(deadline.saturating_duration_since(Instant::now()))?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        self.client.poll_io()?;
        Ok(())
    }

    fn receive(&mut self) -> Result<ShellContentRecord> {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some((_, record)) = self.client.poll_content()? {
                return Ok(record);
            }
            if Instant::now() >= deadline {
                return Err("response timeout".into());
            }
            self.client
                .wait_for_io(deadline.saturating_duration_since(Instant::now()))?;
        }
    }

    /// Image bounds come from the content limits, as Lom derives them.
    fn enable_image(&mut self, socket: &std::ffi::OsStr, limits: &ContentLimits) -> Result<()> {
        let endpoint_dir = std::path::Path::new(socket)
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .ok_or("file endpoint has no directory")?;
        let origin_ms = clock_usec() / 1000;
        let image = ShellImageConfig {
            endpoint_dir: endpoint_dir.to_owned(),
            bounds: ProducerRoleBounds {
                max_resource_bytes: limits.max_resource_bytes,
                max_width: limits.max_width_px,
                max_height: limits.max_height_px,
                max_admitted_outputs: limits.max_outputs,
                transfer_slots: limits.max_open_transfers,
            },
            transfer_timeout_ms: limits.transfer_timeout_ms,
            startup_deadline_ms: origin_ms + WAIT.as_millis() as u64,
            formats_capacity: 4096,
            ring_slots: 0,
            ring_capacity: 0,
            content_grant_epoch: limits.grant.content_grant_epoch,
        };
        self.client
            .enable_dma_image(image, Instant::now(), origin_ms)?;
        Ok(())
    }

    fn wait_ready(&mut self) -> Result<()> {
        let deadline = Instant::now() + WAIT;
        loop {
            self.client.poll_io()?;
            let adapter = self.client.image_adapter().ok_or("image adapter absent")?;
            match adapter.channel() {
                Some(ImageChannelState::Ready) => return Ok(()),
                Some(ImageChannelState::Closed) => return Err("DMA image channel closed".into()),
                _ => {}
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("DMA image channel not ready".into());
            }
            self.client.wait_for_io(left)?;
        }
    }

    /// Upload acceptance needs Accepted status and a capture-complete
    /// (disposition 2) SourceReleased; the caller keeps the frame until then.
    fn settle(&mut self, offer: u64, resource: ContentResourceId) -> Result<()> {
        let deadline = Instant::now() + WAIT;
        loop {
            self.client.poll_io()?;
            let info = self
                .client
                .image_adapter()
                .and_then(|adapter| adapter.dma_offer(offer))
                .ok_or("DMA offer lost")?;
            if info.resource != resource.id || info.resource_generation != resource.generation {
                return Err("DMA offer names another resource".into());
            }
            match info.state {
                OfferState::Settled
                    if info.status == Some(ResourceStatusKind::Accepted)
                        && info.disposition == 2 =>
                {
                    self.client
                        .image_forget(offer)
                        .map_err(|e| format!("image forget refused: {e:?}"))?;
                    return Ok(());
                }
                state @ (OfferState::Settled | OfferState::Refused | OfferState::Unknown) => {
                    return Err(format!(
                        "DMA offer ended {state:?} status {:?} disposition {}",
                        info.status, info.disposition
                    )
                    .into());
                }
                // As in Lom, Ambiguous or Expired offers may still settle.
                _ => {}
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(format!(
                    "DMA offer did not settle: {:?} status {:?} disposition {}",
                    info.state, info.status, info.disposition
                )
                .into());
            }
            self.client.wait_for_io(left)?;
        }
    }
}

fn clock_usec() -> u64 {
    let t = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    u64::try_from(t.tv_sec).unwrap() * 1_000_000 + u64::try_from(t.tv_nsec).unwrap() / 1000
}

fn emit(event: Event<'_>) -> Result<()> {
    journal::write(&mut std::io::stdout().lock(), JOURNAL_SCHEMA, event)?;
    Ok(())
}

fn run() -> Result<()> {
    if std::env::args().skip(1).collect::<Vec<_>>() != ["--serve"] {
        return Err("expected --serve and explicit SOPHIA_SHELL_CONFIG".into());
    }
    let config_path = std::env::var_os("SOPHIA_SHELL_CONFIG").ok_or("missing configuration")?;
    let mut text = String::new();
    std::fs::File::open(config_path)?
        .take(4097)
        .read_to_string(&mut text)?;
    if text.len() > 4096 {
        return Err("configuration too large".into());
    }
    let config: serde_json::Value = serde_json::from_str(&text)?;
    let object = config
        .as_object()
        .ok_or("configuration must be an object")?;
    const KEYS: [&str; 6] = ["wire", "output", "frames", "height", "hold_msec", "source"];
    if object.len() != KEYS.len() || object.keys().any(|k| !KEYS.contains(&k.as_str())) {
        return Err(
            "configuration requires exactly wire, output, frames, height, hold_msec and source"
                .into(),
        );
    }
    if config["wire"].as_str() != Some("files") || config["source"].as_str() != Some("dma_buf") {
        return Err("DMA peer requires wire files and source dma_buf".into());
    }
    let number = |key: &str| -> Result<u64> {
        config[key]
            .as_u64()
            .ok_or_else(|| format!("invalid {key}").into())
    };
    let output_id = number("output")?;
    let frames = number("frames")?;
    let height = number("height")?;
    let hold = number("hold_msec")?;
    if output_id == 0
        || !(1..=10000).contains(&frames)
        || !(1..=512).contains(&height)
        || hold > 600000
    {
        return Err("configuration outside bounded workload".into());
    }
    let socket = std::env::var_os("SOPHIA_SHELL_9P_SOCKET").ok_or("missing file endpoint")?;
    let mut wire = Wire::connect()?;
    let ShellContentRecord::Limits(limits) = wire.receive()? else {
        return Err("missing limits".into());
    };
    wire.enable_image(&socket, &limits)?;
    canvas::open(limits.grant.connection_epoch, "dma_buf")?;
    let ShellContentRecord::OutputFacts(facts) = wire.receive()? else {
        return Err("missing output facts".into());
    };
    let output = facts
        .outputs
        .iter()
        .find(|entry| entry.output.id == output_id)
        .ok_or("selected output absent")?;
    let grant = limits.grant;
    if facts.grant != grant || facts.facts_generation == 0 || output.output.generation == 0 {
        return Err("stale or misattributed output facts".into());
    }
    if limits.max_resource_ids < 2 {
        return Err("insufficient resource identities for paced workload".into());
    }
    wire.send(
        1,
        &[ShellContentRecord::AllocationRequest(
            ContentAllocationRequest {
                grant,
                output: output.output,
                allocation_request_id: 1,
                operation: 1,
                role: 1,
                edge: 1,
                prior: ContentAllocationId::default(),
                parent: ContentAllocationId::default(),
                parent_presentation_epoch: 0,
                anchor_parent_rect: ContentPixelRect::default(),
                desired_width: output.local_width,
                desired_height: height as u32,
                margins: ContentMargins::default(),
            },
        )],
    )?;
    let ShellContentRecord::AllocationResult(allocation) = wire.receive()? else {
        return Err("missing allocation".into());
    };
    if allocation.status != 1
        || allocation.grant != grant
        || allocation.output != output.output
        || allocation.allocation_request_id != 1
    {
        return Err("allocation refused or misattributed".into());
    }
    // A smaller grant would silently measure a smaller presented surface.
    if allocation.pixel.width != output.local_width || allocation.pixel.height != height as u32 {
        return Err("allocation geometry differs from the requested panel".into());
    }
    wire.wait_ready()?;
    emit(Event::Start {
        frames,
        wire: "files",
        probe_bytes: 0,
        source: Some("dma_buf"),
        workload: None,
        retained_resources: None,
    })?;
    let (width, height) = (allocation.pixel.width, allocation.pixel.height);
    let length = canvas::rgba_bytes(width, height).ok_or("upload size overflow")?;
    for generation in 1..=frames {
        let resource = resource_for_frame(generation);
        let prepare_usec = clock_usec();
        let frame = canvas::render_dma(width, height, generation)?;
        canvas::prepared(generation, resource.id, prepare_usec, clock_usec());
        // As in the comparator, the offer is journalled after pixel preparation.
        emit(Event::UploadOffer {
            upload: Upload {
                connection_epoch: grant.connection_epoch,
                content_grant_epoch: grant.content_grant_epoch,
                output: output.output.id,
                candidate_generation: generation,
                resource_id: resource.id,
                resource_generation: resource.generation,
                width,
                height,
                total_bytes: length,
                begin_usec: clock_usec(),
            },
        })?;
        let metadata = ProducerMetadata {
            offer_id: 0,
            resource: resource.id,
            resource_generation: resource.generation,
            admission_epoch: grant.content_grant_epoch,
            output_id: output.output.id,
            allocation_id: allocation.allocation.id,
            topology_generation: facts.facts_generation,
            source_kind: 2,
            source_format: u32::from_le_bytes(*b"XR24"),
            width,
            height,
            stride: 0,
            plane_count: 1,
            descriptor_count: 1,
            declared_bytes: length,
            modifier: 0,
        };
        let source_bytes = frame.source_bytes();
        let begun = wire.client.image_begin_dma_buf(
            metadata,
            frame.dup_fd()?,
            frame.offset(),
            frame.stride(),
            Instant::now(),
        );
        let offer = match begun {
            Ok(admission) => admission.offer,
            Err(refused) => {
                // Without the descriptor back, Session may hold the export.
                if refused.source.is_none() {
                    std::mem::forget(frame);
                }
                return Err(format!(
                    "image begin refused: {:?} (source_bytes {source_bytes})",
                    refused.error
                )
                .into());
            }
        };
        let settled = expect_status(&mut wire, grant, resource, 2)
            .and_then(|()| wire.settle(offer, resource));
        if let Err(error) = settled {
            // Release is unproven: keep the export allocated until process exit.
            std::mem::forget(frame);
            return Err(error);
        }
        // Session holds its own captured copy; the export may now be freed.
        drop(frame);
        emit(Event::UploadAccepted {
            resource_id: resource.id,
            resource_generation: resource.generation,
            accepted_usec: clock_usec(),
        })?;
        let demand_time = clock_usec();
        wire.send(
            5,
            &[ShellContentRecord::FrameDemand(ContentFrameDemand {
                grant,
                output: output.output,
                allocation: ContentAllocationId::default(),
                demand_id: generation,
                reason: 1,
            })],
        )?;
        let ShellContentRecord::FramePermit(permit) = wire.receive()? else {
            return Err("missing permit".into());
        };
        if permit.state != 1
            || permit.grant != grant
            || permit.output != output.output
            || permit.demand_id != generation
            || permit.permit_id == 0
        {
            return Err("permit refused or misattributed".into());
        }
        emit(Event::Permit {
            generation,
            demand_usec: demand_time,
            received_usec: clock_usec(),
        })?;
        let placements = vec![ContentPlacement {
            resource,
            surface_index: 0,
            destination_x_px: 0,
            destination_y_px: 0,
        }];
        let records = [
            ShellContentRecord::CandidateBegin(ContentCandidateBegin {
                grant,
                output: output.output,
                candidate_generation: generation,
                facts_generation: facts.facts_generation,
                pacing_permit: permit.permit_id,
                interaction_generation: 1,
                surface_count: 1,
                placement_count: 1,
                target_count: 0,
            }),
            ShellContentRecord::CandidateChunk(ContentCandidateChunk {
                grant,
                candidate_generation: generation,
                chunk_ordinal: 0,
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
                placements,
                targets: vec![],
            }),
            ShellContentRecord::CandidateEnd(ContentCandidateEnd {
                grant,
                candidate_generation: generation,
                surface_count: 1,
                placement_count: 1,
                target_count: 0,
            }),
        ];
        emit(Event::Offer {
            connection_epoch: grant.connection_epoch,
            content_grant_epoch: grant.content_grant_epoch,
            output: output.output.id,
            candidate_generation: generation,
            submitted_monotonic_usec: clock_usec(),
        })?;
        wire.send(6, &records)?;
        let mut prepared = false;
        loop {
            let ShellContentRecord::CandidateOutcome(outcome) = wire.receive()? else {
                return Err("unexpected candidate response".into());
            };
            if outcome.grant != grant
                || outcome.output != output.output
                || outcome.candidate_generation != generation
            {
                return Err("misattributed candidate outcome".into());
            }
            if outcome.kind == 1 {
                if prepared {
                    return Err("duplicate prepared outcome".into());
                }
                prepared = true;
                continue;
            }
            emit(Event::Terminal {
                connection_epoch: grant.connection_epoch,
                content_grant_epoch: grant.content_grant_epoch,
                output: output.output.id,
                candidate_generation: generation,
                outcome: if outcome.kind == 2 {
                    "presented"
                } else {
                    "rejected"
                },
                kind: outcome.kind,
                reason: outcome.reason,
                observed_monotonic_usec: clock_usec(),
            })?;
            if outcome.kind != 2 {
                return Err("candidate was not presented".into());
            }
            if !prepared || outcome.presentation_epoch == 0 {
                return Err("presentation lacks preparation or epoch".into());
            }
            break;
        }
        // Keep the currently displayed resource until a successor is shown.
        if generation > 1 {
            let old = resource_for_frame(generation - 1);
            wire.send(
                7,
                &[ShellContentRecord::ResourceRetire(ContentResourceRetire {
                    grant,
                    resource: old,
                })],
            )?;
            let ShellContentRecord::ResourceReleased(released) = wire.receive()? else {
                return Err("missing retired resource release".into());
            };
            if released.grant != grant || released.resource != old || released.reason != 0 {
                return Err("wrong retired resource release".into());
            }
        }
    }
    emit(Event::End {
        offered: frames,
        completed: frames,
    })?;
    std::thread::sleep(Duration::from_millis(hold));
    Ok(())
}

fn expect_status(
    wire: &mut Wire,
    grant: ContentGrant,
    resource: ContentResourceId,
    expected: u16,
) -> Result<()> {
    match wire.receive()? {
        ShellContentRecord::ResourceStatus(s)
            if s.grant == grant && s.resource == resource && s.status == expected =>
        {
            Ok(())
        }
        other => Err(format!("unexpected resource response: {other:?}").into()),
    }
}

fn resource_for_frame(frame: u64) -> ContentResourceId {
    ContentResourceId {
        id: (frame - 1) % 2 + 1,
        generation: (frame - 1) / 2 + 1,
    }
}

fn main() {
    if let Err(error) = run() {
        let _ = emit(Event::Failed {
            error: &error.to_string(),
        });
        std::process::exit(1);
    }
}
