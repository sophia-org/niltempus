//! Arm G: the comparator paced peer's fresh workload over files at r10 with
//! a producer-completed GPU DMA-BUF source. Records, resource identities,
//! cadence and journal match ../../historical (see PROVENANCE.txt); only the
//! source differs. There is no IPC, byte, sealed, probe or retained path.
//! Arm R runs the same loop with `source: owned_ring`: the same GPU frame read
//! back and copied into the SDK's one persistent owned-ring registration,
//! journalled at schema 7 (see ring_journal.rs). CPU mode fills that ring directly.
use sophia_image_producer::ResourceStatusKind;
use sophia_shell_client::{
    IMAGE_CAPABILITY_DMA_BUF, IMAGE_CAPABILITY_OWNED_RING, ImageChannelState, OfferState,
    ProducerRoleBounds, RingOfferState, RingUpload, ShellClientError, ShellClientOptions,
    ShellConnection, ShellImageConfig,
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
mod dma_frame;
mod producer;
mod profile;
use producer::Producer;
#[cfg(feature = "diagnostic-colour-hold")]
mod hold;
mod ring_journal;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    DmaBuf,
    /// Registered owned-ring slots, 2..=4.
    OwnedRing {
        slots: u8,
    },
}

impl Source {
    fn label(self) -> &'static str {
        match self {
            Source::DmaBuf => "dma_buf",
            Source::OwnedRing { .. } => "owned_ring",
        }
    }

    fn schema(self) -> u8 {
        match self {
            Source::DmaBuf => JOURNAL_SCHEMA,
            Source::OwnedRing { .. } => ring_journal::SCHEMA,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Config {
    source: Source,
    producer: Producer,
    capture_profile: bool,
    /// Diagnostic DMA arm that redraws one export (see dma_frame.rs).
    dma_reuse: bool,
    output: u64,
    frames: u64,
    height: u64,
    hold_msec: u64,
}

/// Diagnostics are explicit; ordinary runs retain the original exact keys.
fn parse_config(text: &str) -> Result<Config> {
    let config: serde_json::Value = serde_json::from_str(text)?;
    let object = config
        .as_object()
        .ok_or("configuration must be an object")?;
    const KEYS: [&str; 6] = ["wire", "output", "frames", "height", "hold_msec", "source"];
    let ring = config["source"].as_str() == Some("owned_ring");
    let profiling = object.contains_key("capture_profile");
    let reusing = object.contains_key("dma_reuse");
    let producing = object.contains_key("producer");
    let allowed = |k: &str| {
        KEYS.contains(&k)
            || (ring && k == "ring_slots")
            || k == "capture_profile"
            || k == "dma_reuse"
            || k == "producer"
    };
    if object.len()
        != KEYS.len()
            + usize::from(ring)
            + usize::from(profiling)
            + usize::from(reusing)
            + usize::from(producing)
        || object.keys().any(|k| !allowed(k))
    {
        return Err(
            "configuration requires exactly wire, output, frames, height, hold_msec \
                    and source, plus ring_slots for owned_ring, optional capture_profile \
                    and, for dma_buf, optional dma_reuse; optional producer is gpu or cpu"
                .into(),
        );
    }
    let number = |key: &str| -> Result<u64> {
        config[key]
            .as_u64()
            .ok_or_else(|| format!("invalid {key}").into())
    };
    let source = match config["source"].as_str() {
        Some("dma_buf") => Source::DmaBuf,
        Some("owned_ring") => match number("ring_slots")? {
            slots @ 2..=4 => Source::OwnedRing { slots: slots as u8 },
            _ => return Err("ring_slots outside 2..=4".into()),
        },
        _ => return Err("source must be dma_buf or owned_ring".into()),
    };
    if config["wire"].as_str() != Some("files") {
        return Err("GPU files peer requires wire files".into());
    }
    let producer = match object.get("producer") {
        None => Producer::Gpu,
        Some(value) => match value.as_str() {
            Some("gpu") => Producer::Gpu,
            Some("cpu") => Producer::Cpu,
            _ => return Err("producer must be gpu or cpu".into()),
        },
    };
    let parsed = Config {
        source,
        producer,
        capture_profile: if profiling {
            config["capture_profile"]
                .as_bool()
                .ok_or("capture_profile must be boolean")?
        } else {
            false
        },
        dma_reuse: if reusing {
            if source != Source::DmaBuf {
                return Err("dma_reuse is only for source dma_buf".into());
            }
            config["dma_reuse"]
                .as_bool()
                .ok_or("dma_reuse must be boolean")?
        } else {
            false
        },
        output: number("output")?,
        frames: number("frames")?,
        height: number("height")?,
        hold_msec: number("hold_msec")?,
    };
    if producer == Producer::Cpu && (!ring || parsed.capture_profile || reusing) {
        return Err("CPU producer requires owned_ring without profiling or dma_reuse".into());
    }
    if parsed.output == 0
        || !(1..=10000).contains(&parsed.frames)
        || !(1..=512).contains(&parsed.height)
        || parsed.hold_msec > 600000
    {
        return Err("configuration outside bounded workload".into());
    }
    if parsed.capture_profile
        && (parsed.height != 24 || parsed.frames != 2120 || parsed.hold_msec != 60000)
    {
        return Err("capture_profile requires height 24, 2120 frames and 60000 ms".into());
    }
    // Diagnostic only until a separate acceptance package measures it, so no
    // comparison plan can mistake reused backing for the fresh G arm.
    if parsed.dma_reuse && !parsed.capture_profile {
        return Err("dma_reuse requires capture_profile".into());
    }
    Ok(parsed)
}

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
    fn connect(source: Source) -> Result<Self> {
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
        // Negotiation is exact, so without it Session grants the ring alone.
        let dma = match source {
            Source::DmaBuf => IMAGE_CAPABILITY_DMA_BUF,
            Source::OwnedRing { .. } => 0,
        };
        let options = ShellClientOptions {
            minimum_revision: 10,
            maximum_revision: 10,
            required_capabilities: SOPHIA_SHELL_CAPABILITY_DESCRIPTOR_SWITCHER
                | SOPHIA_SHELL_CAPABILITY_CONTENT_SURFACE
                | SOPHIA_SHELL_CAPABILITY_CONTENT_DISCRETE_INPUT
                | IMAGE_CAPABILITY_OWNED_RING
                | dma,
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

    /// Image bounds come from the content limits, as Lom derives them. A
    /// ring registers `slots` slots of exactly `capacity` bytes; DMA has none.
    fn enable_image(
        &mut self,
        socket: &std::ffi::OsStr,
        limits: &ContentLimits,
        ring: Option<(u8, u64)>,
    ) -> Result<()> {
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
            ring_slots: ring.map_or(0, |(slots, _)| slots),
            ring_capacity: ring.map_or(0, |(_, capacity)| capacity),
            content_grant_epoch: limits.grant.content_grant_epoch,
        };
        if ring.is_some() {
            self.client.enable_image(image, Instant::now(), origin_ms)?;
        } else {
            self.client
                .enable_dma_image(image, Instant::now(), origin_ms)?;
        }
        Ok(())
    }

    fn wait_ready(&mut self) -> Result<()> {
        let deadline = Instant::now() + WAIT;
        loop {
            self.client.poll_io()?;
            let adapter = self.client.image_adapter().ok_or("image adapter absent")?;
            match adapter.channel() {
                Some(ImageChannelState::Ready) => return Ok(()),
                Some(ImageChannelState::Closed) => return Err("image channel closed".into()),
                _ => {}
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("image channel not ready".into());
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

    /// A ring copy is adoptable once the SDK reports Settled: Accepted status
    /// and its exact successful SlotReleased. Returns the release observation.
    fn settle_ring(&mut self, offer: u64, resource: ContentResourceId) -> Result<u64> {
        let deadline = Instant::now() + WAIT;
        loop {
            self.client.poll_io()?;
            let info = self
                .client
                .image_ring_offer(offer)
                .ok_or("ring offer lost")?;
            if info.resource != resource.id || info.resource_generation != resource.generation {
                return Err("ring offer names another resource".into());
            }
            match info.state {
                RingOfferState::Settled if info.status == Some(ResourceStatusKind::Accepted) => {
                    let released_usec = clock_usec();
                    self.client
                        .image_forget(offer)
                        .map_err(|e| format!("image forget refused: {e:?}"))?;
                    return Ok(released_usec);
                }
                RingOfferState::Pending => {}
                state => {
                    return Err(
                        format!("ring offer ended {state:?} status {:?}", info.status).into(),
                    );
                }
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(format!(
                    "ring offer did not settle: {:?} status {:?}",
                    info.state, info.status
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

/// The run's journal schema, fixed once its source is parsed.
static SCHEMA: std::sync::OnceLock<u8> = std::sync::OnceLock::new();

fn emit(event: Event<'_>) -> Result<()> {
    let schema = SCHEMA.get().copied().unwrap_or(JOURNAL_SCHEMA);
    journal::write(&mut std::io::stdout().lock(), schema, event)?;
    Ok(())
}

fn emit_ring(event: ring_journal::Event<'_>) -> Result<()> {
    ring_journal::write(&mut std::io::stdout().lock(), event)?;
    Ok(())
}

fn run() -> Result<()> {
    run_mode(false)
}

fn run_mode(diagnostic: bool) -> Result<()> {
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
    let config = parse_config(&text)?;
    #[cfg(feature = "diagnostic-colour-hold")]
    if diagnostic {
        hold::validate_config(&config)?;
    }
    #[cfg(not(feature = "diagnostic-colour-hold"))]
    if diagnostic {
        return Err("diagnostic peer requires diagnostic-colour-hold".into());
    }
    let Config {
        source,
        producer,
        capture_profile,
        dma_reuse,
        output: output_id,
        frames,
        height,
        hold_msec: hold,
    } = config;
    let _ = SCHEMA.set(source.schema());
    let socket = std::env::var_os("SOPHIA_SHELL_9P_SOCKET").ok_or("missing file endpoint")?;
    let mut wire = Wire::connect(source)?;
    let ShellContentRecord::Limits(limits) = wire.receive()? else {
        return Err("missing limits".into());
    };
    if source == Source::DmaBuf {
        wire.enable_image(&socket, &limits, None)?;
    }
    producer.open(limits.grant.connection_epoch, source, capture_profile)?;
    if capture_profile {
        canvas::diagnostic_line(format_args!(
            "gpu_bar_profile schema=1 source={} enabled=true",
            source.label()
        ));
    }
    let mut backing = dma_frame::Backing::new(dma_reuse);
    #[cfg(feature = "diagnostic-colour-hold")]
    {
        backing.colour_hold = diagnostic;
    }
    if source == Source::DmaBuf {
        canvas::diagnostic_line(format_args!(
            "gpu_bar_dma_mode schema=1 mode={}",
            backing.mode()
        ));
    }
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
    let (width, height) = (allocation.pixel.width, allocation.pixel.height);
    let length = canvas::rgba_bytes(width, height).ok_or("upload size overflow")?;
    // The ring's fixed slot capacity is the exact frame, known only now.
    if let Source::OwnedRing { slots } = source {
        wire.enable_image(&socket, &limits, Some((slots, length)))?;
    }
    wire.wait_ready()?;
    #[cfg(feature = "diagnostic-colour-hold")]
    let mut hold_control = if diagnostic {
        Some(hold::Control::connect(&socket, &mut wire)?)
    } else {
        None
    };
    let ready_usec = clock_usec();
    let mut ring_canvas = None;
    match source {
        Source::DmaBuf => emit(Event::Start {
            frames,
            wire: "files",
            probe_bytes: 0,
            source: Some("dma_buf"),
            workload: None,
            retained_resources: None,
        })?,
        Source::OwnedRing { slots } => {
            let identity = wire
                .client
                .image_adapter()
                .and_then(|adapter| adapter.ring_identity())
                .ok_or("ready ring has no identity")?;
            emit_ring(ring_journal::Event::Start {
                frames,
                wire: "files",
                probe_bytes: 0,
                source: "owned_ring",
                workload: "fresh",
                ring_slots: u64::from(slots),
            })?;
            emit_ring(ring_journal::Event::RingReady {
                connection_epoch: identity.connection_epoch,
                content_grant_epoch: identity.content_grant_epoch,
                channel: identity.channel_generation,
                ring: identity.ring,
                ring_generation: identity.generation,
                slots: u64::from(slots),
                pixel_bytes: length,
                admitted_usec: None,
                ready_usec,
            })?;
            ring_canvas = Some(canvas::Canvas::new(usize::try_from(length)?));
        }
    }
    let mut timing_start = None;
    let mut timing_end = None;
    for generation in 1..=frames {
        let resource = resource_for_frame(generation);
        if let Some(canvas) = &mut ring_canvas {
            let upload = Upload {
                connection_epoch: grant.connection_epoch,
                content_grant_epoch: grant.content_grant_epoch,
                output: output.output.id,
                candidate_generation: generation,
                resource_id: resource.id,
                resource_generation: resource.generation,
                width,
                height,
                total_bytes: length,
                begin_usec: 0,
            };
            ring_frame(
                &mut wire,
                canvas,
                producer,
                upload,
                grant,
                &allocation,
                output.output,
                facts.facts_generation,
            )?;
        } else {
            dma_frame::dma_frame(
                &mut wire,
                &mut backing,
                grant,
                resource,
                generation,
                width,
                height,
                length,
                output.output,
                allocation.allocation.id,
                facts.facts_generation,
            )?;
        }
        let presentation_epoch = present(
            &mut wire,
            grant,
            &allocation,
            output.output,
            facts.facts_generation,
            generation,
        )?;
        #[cfg(feature = "diagnostic-colour-hold")]
        if let Some(control) = &mut hold_control {
            control.presented(
                &mut wire,
                grant,
                output.output,
                generation,
                presentation_epoch,
            )?;
        }
        #[cfg(not(feature = "diagnostic-colour-hold"))]
        let _ = presentation_epoch;
        if capture_profile && source == Source::DmaBuf && matches!(generation, 120 | 2120) {
            let snapshot =
                canvas::with_gpu(|gpu| gpu.dma_timing().ok_or("DMA timing absent".into()))?;
            if generation == 120 {
                timing_start = Some(snapshot);
            } else {
                timing_end = Some(snapshot);
            }
        }
    }
    // A reused export is destroyed only now, after the counted window.
    drop(backing);
    emit(Event::End {
        offered: frames,
        completed: frames,
    })?;
    #[cfg(feature = "diagnostic-colour-hold")]
    if let Some(control) = &mut hold_control {
        control.finish(&mut wire)?;
    }
    if capture_profile && source == Source::DmaBuf {
        let record = if dma_reuse {
            profile::reuse_record
        } else {
            profile::record
        };
        canvas::diagnostic_line(format_args!(
            "{}",
            record(
                timing_start.ok_or("missing timing start")?,
                timing_end.ok_or("missing timing end")?
            )?
        ));
    }
    if !diagnostic {
        std::thread::sleep(Duration::from_millis(hold));
    }
    Ok(())
}

/// One owned-ring frame, journalled only after exact acceptance and settlement.
fn ring_frame(
    wire: &mut Wire,
    canvas: &mut canvas::Canvas,
    producer: Producer,
    mut upload: Upload,
    grant: ContentGrant,
    allocation: &ContentAllocationResult,
    output: ContentOutputId,
    facts_generation: u64,
) -> Result<()> {
    let resource = ContentResourceId {
        id: upload.resource_id,
        generation: upload.resource_generation,
    };
    let mut fill_start_usec = clock_usec();
    let metadata = RingUpload {
        resource: resource.id,
        resource_generation: resource.generation,
        allocation: allocation.allocation.id,
        allocation_generation: allocation.allocation.generation,
        output: output.id,
        output_generation: output.generation,
        topology_generation: facts_generation,
        width: upload.width,
        height: upload.height,
    };
    let length = usize::try_from(upload.total_bytes)?;
    let offer = match producer {
        Producer::Gpu => {
            let pixels = canvas.fill(length, upload.width, upload.height, upload.candidate_generation)?;
            canvas::prepared(upload.candidate_generation, resource.id, fill_start_usec, clock_usec());
            upload.begin_usec = clock_usec();
            wire.client.image_upload_ring(metadata, pixels, Instant::now())
        }
        Producer::Cpu => {
            let offer = wire.client.image_upload_ring_fill(metadata, length, Instant::now(), |pixels| {
                fill_start_usec = clock_usec();
                producer::fill_cpu(pixels, upload.candidate_generation);
                upload.begin_usec = clock_usec();
            });
            if offer.is_ok() {
                eprintln!("gpu_bar_prepare schema=1 arm=owned_ring generation={} resource_id={} start_usec={} end_usec={}",
                    upload.candidate_generation, resource.id, fill_start_usec, upload.begin_usec);
            }
            offer
        }
    }.map_err(|e| format!("ring upload refused: {e:?}"))?;
    let slot = u64::from(
        wire.client
            .image_ring_offer(offer)
            .ok_or("ring offer lost")?
            .slot,
    );
    emit_ring(ring_journal::Event::RingOffer {
        upload,
        offer,
        slot,
        slot_generation: None,
        fill_start_usec,
    })?;
    expect_status(wire, grant, resource, 2)?;
    let accepted_usec = clock_usec();
    let released_usec = wire.settle_ring(offer, resource)?;
    emit_ring(ring_journal::Event::RingAccepted {
        offer,
        resource_id: resource.id,
        resource_generation: resource.generation,
        accepted_usec,
    })?;
    emit_ring(ring_journal::Event::SlotReleased {
        offer,
        slot,
        slot_generation: None,
        resource_id: resource.id,
        resource_generation: resource.generation,
        released_usec,
    })
}

/// Demand, permit, the one-placement candidate through presentation, then
/// retirement of the predecessor once its successor is shown.
fn present(
    wire: &mut Wire,
    grant: ContentGrant,
    allocation: &ContentAllocationResult,
    output: ContentOutputId,
    facts_generation: u64,
    generation: u64,
) -> Result<u64> {
    let resource = resource_for_frame(generation);
    let demand_time = clock_usec();
    wire.send(
        5,
        &[ShellContentRecord::FrameDemand(ContentFrameDemand {
            grant,
            output,
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
        || permit.output != output
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
            output,
            candidate_generation: generation,
            facts_generation,
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
        output: output.id,
        candidate_generation: generation,
        submitted_monotonic_usec: clock_usec(),
    })?;
    wire.send(6, &records)?;
    let mut prepared = false;
    let presentation_epoch = loop {
        let ShellContentRecord::CandidateOutcome(outcome) = wire.receive()? else {
            return Err("unexpected candidate response".into());
        };
        if outcome.grant != grant
            || outcome.output != output
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
            output: output.id,
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
        break outcome.presentation_epoch;
    };
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
    Ok(presentation_epoch)
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

#[cfg(feature = "diagnostic-colour-hold")]
#[allow(dead_code)] // Called only by the separate colour-hold binary.
pub(crate) fn diagnostic_main() {
    if let Err(error) = run_mode(true) {
        canvas::diagnostic_line(format_args!(
            "gpu_colour_hold_refused schema=1 error={error:?}"
        ));
        std::process::exit(1);
    }
}

#[cfg(test)]
#[path = "../tests/support/config.rs"]
mod tests;
