//! Schema 7: the SDK owned-ring journal. It keeps schema 5's ring records,
//! but `admitted_usec` and `slot_generation` are always null because the SDK
//! exposes neither. The slot is the SDK's real one, which can be 0 every frame.
//! RingReady's `slots`/`pixel_bytes` are the configured registration bounds;
//! every `_usec` here is when the peer observed the event, and two may be equal.
//! Permit, Offer, Terminal, End and Failed use the shared journal at schema 7.
use super::journal::Upload;
use serde::Serialize;
use std::io::{self, Write};

pub const SCHEMA: u8 = 7;

#[derive(Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event<'a> {
    Start {
        frames: u64,
        wire: &'a str,
        probe_bytes: u64,
        source: &'a str,
        workload: &'a str,
        ring_slots: u64,
    },
    RingReady {
        connection_epoch: u64,
        content_grant_epoch: u64,
        channel: u64,
        ring: u64,
        ring_generation: u64,
        slots: u64,
        pixel_bytes: u64,
        admitted_usec: Option<u64>,
        ready_usec: u64,
    },
    RingOffer {
        upload: Upload,
        offer: u64,
        slot: u64,
        slot_generation: Option<u64>,
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
        slot_generation: Option<u64>,
        resource_id: u64,
        resource_generation: u64,
        released_usec: u64,
    },
}

/// The same bounded, flushed record framing as the shared journal.
pub fn write(output: &mut impl Write, event: Event<'_>) -> io::Result<()> {
    #[derive(Serialize)]
    struct Record<'a> {
        schema: u8,
        #[serde(flatten)]
        event: Event<'a>,
    }
    let mut bytes = [0u8; 1024];
    let mut buffer = io::Cursor::new(bytes.as_mut_slice());
    serde_json::to_writer(
        &mut buffer,
        &Record {
            schema: SCHEMA,
            event,
        },
    )?;
    writeln!(buffer)?;
    let len = buffer.position() as usize;
    output.write_all(&bytes[..len])?;
    output.flush()
}

#[cfg(test)]
#[path = "../tests/support/ring_journal.rs"]
mod tests;
