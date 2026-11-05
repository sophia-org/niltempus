//! CPU-only matched reference: unchanged fill/ring/scene, explicit r10 or SFT.
use sophia_image_producer::ResourceStatusKind;
use sophia_shell_client::{
    IMAGE_CAPABILITY_OWNED_RING, ImageChannelState, ProducerRoleBounds, RingOfferState, RingUpload,
    ShellClientError, ShellClientOptions, ShellConnection, ShellImageConfig,
};
use sophia_shell_protocol::*;
use std::{
    io::Read,
    os::{fd::FromRawFd, unix::net::UnixStream},
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const WAIT: Duration = Duration::from_secs(5);
#[path = "../../gpu-bar/historical/src/shell_timing_peer/journal.rs"]
#[allow(dead_code)]
mod journal;
use journal::{Event, RetireCounters, Upload};
mod legacy;
mod ring_journal;
mod sft;
mod wire;
use wire::Wire;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    wire: String,
    source: String,
    producer: String,
    ring_slots: u8,
    output: u64,
    frames: u64,
    height: u32,
    hold_msec: u64,
    #[serde(default)]
    frame_transactions: bool,
}
impl Config {
    fn parse(text: &str) -> Result<Self> {
        let config: Self = serde_json::from_str(text)?;
        if config.wire != "files"
            || config.source != "owned_ring"
            || config.producer != "cpu"
            || config.ring_slots != 2
            || config.output == 0
            || !(1..=10000).contains(&config.frames)
            || !(1..=512).contains(&config.height)
            || config.hold_msec > 600000
            || (config.frame_transactions && !cfg!(feature = "experimental-frame-transactions"))
        {
            return Err(
                "requires bounded CPU/two-slot/files workload and explicit SFT build permission"
                    .into(),
            );
        }
        Ok(config)
    }
}

fn byte_len(width: u32, height: u32) -> Option<u64> {
    u64::from(width)
        .checked_mul(u64::from(height))?
        .checked_mul(4)
}

// Same shade_mod128_a255 loop as gpu-bar-dma e730d3910 producer::fill_cpu.
fn fill_cpu(pixels: &mut [u8], frame: u64) {
    assert!(pixels.len().is_multiple_of(4));
    let shade = (frame % 128) as u8;
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[shade, shade, shade, 255]);
    }
}

fn resource_for_frame(frame: u64) -> ContentResourceId {
    ContentResourceId {
        id: (frame - 1) % 2 + 1,
        generation: (frame - 1) / 2 + 1,
    }
}
fn clock_usec() -> u64 {
    let t = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    u64::try_from(t.tv_sec).unwrap() * 1_000_000 + u64::try_from(t.tv_nsec).unwrap() / 1000
}
static SCHEMA: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
fn emit(schema: u8, event: Event<'_>) -> Result<()> {
    journal::write(&mut std::io::stdout().lock(), schema, event)?;
    Ok(())
}
fn emit_ring(schema: u8, event: ring_journal::Event<'_>) -> Result<()> {
    ring_journal::write(&mut std::io::stdout().lock(), event, schema)?;
    Ok(())
}

fn run() -> Result<()> {
    let path = std::env::var_os("SOPHIA_SHELL_CONFIG").ok_or("missing configuration")?;
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(4097)
        .read_to_string(&mut text)?;
    if text.len() > 4096 {
        return Err("configuration too large".into());
    }
    let config = Config::parse(&text)?;
    let _ = SCHEMA.set(if config.frame_transactions {
        sft::SCHEMA
    } else {
        ring_journal::SCHEMA
    });
    let socket = std::env::var_os("SOPHIA_SHELL_9P_SOCKET").ok_or("missing file endpoint")?;
    let (mut wire, _relay) = Wire::connect(config.frame_transactions)?;
    eprintln!("cpu_bar schema=1 source=owned_ring producer=cpu_shm fill=shade_mod128_a255");
    run_connected(&mut wire, &config, &socket)
}

fn run_connected(wire: &mut Wire, config: &Config, socket: &std::ffi::OsStr) -> Result<()> {
    let ShellContentRecord::Limits(limits) = wire.receive()? else {
        return Err("missing limits".into());
    };
    let ShellContentRecord::OutputFacts(facts) = wire.receive()? else {
        return Err("missing facts".into());
    };
    let output = facts
        .outputs
        .iter()
        .find(|o| o.output.id == config.output)
        .ok_or("output absent")?;
    let grant = limits.grant;
    if facts.grant != grant
        || facts.facts_generation == 0
        || output.output.generation == 0
        || limits.max_resource_ids < 2
    {
        return Err("stale facts or insufficient resources".into());
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
                desired_height: config.height,
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
        || allocation.pixel.width != output.local_width
        || allocation.pixel.height != config.height
    {
        return Err("allocation refused, misattributed or changed size".into());
    }
    let length =
        byte_len(allocation.pixel.width, allocation.pixel.height).ok_or("pixel length overflow")?;
    if length > 4 * 1024 * 1024 {
        return Err("frame exceeds initial SFT byte bound".into());
    }
    wire.enable_image(socket, &limits, length)?;
    wire.wait_ready()?;
    let identity = wire
        .client
        .image_adapter()
        .and_then(|a| a.ring_identity())
        .ok_or("ring has no identity")?;
    emit_ring(
        if config.frame_transactions {
            sft::SCHEMA
        } else {
            7
        },
        ring_journal::Event::Start {
            frames: config.frames,
            wire: "files",
            probe_bytes: 0,
            source: "owned_ring",
            workload: "fresh",
            ring_slots: 2,
        },
    )?;
    emit_ring(
        if config.frame_transactions {
            sft::SCHEMA
        } else {
            7
        },
        ring_journal::Event::RingReady {
            connection_epoch: identity.connection_epoch,
            content_grant_epoch: identity.content_grant_epoch,
            channel: identity.channel_generation,
            ring: identity.ring,
            ring_generation: identity.generation,
            slots: 2,
            pixel_bytes: length,
            admitted_usec: None,
            ready_usec: clock_usec(),
        },
    )?;
    let mut stream = if config.frame_transactions {
        Some(sft::Peer::open(
            &mut wire.client,
            grant,
            output.output,
            &allocation,
            facts.facts_generation,
        )?)
    } else {
        None
    };
    let mut counters = RetireCounters::new();
    for generation in 1..=config.frames {
        if let Some(stream) = &mut stream {
            stream.frame(&mut wire.client, generation, &allocation, &mut counters)?;
        } else {
            legacy::frame(
                wire,
                grant,
                &allocation,
                output.output,
                facts.facts_generation,
                generation,
                &mut counters,
            )?;
        }
    }
    if let Some(stream) = &mut stream {
        stream.close(&mut wire.client, config.frames)?;
    }
    emit(
        if config.frame_transactions {
            sft::SCHEMA
        } else {
            7
        },
        Event::End {
            offered: config.frames,
            completed: config.frames,
            retire: Some(counters),
        },
    )?;
    std::thread::sleep(Duration::from_millis(config.hold_msec));
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        let _ = emit(
            *SCHEMA.get().unwrap_or(&7),
            Event::Failed {
                error: &error.to_string(),
            },
        );
        std::process::exit(1);
    }
}
#[cfg(all(test, feature = "experimental-frame-transactions"))]
#[path = "../tests/support/sft_owner/mod.rs"]
mod sft_owner;
#[cfg(test)]
#[path = "../tests/support/config.rs"]
mod tests;
