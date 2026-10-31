//! Delay applying the already matched Accepted/capture-complete release.
use super::{Config, Producer, Result, Source, canvas, clock_usec};
use sophia_shell_protocol::{ContentGrant, ContentResourceId};
use std::mem::ManuallyDrop;
use std::time::Duration;

pub(super) fn validate_config(config: &Config) -> Result<()> {
    if config.source != Source::DmaBuf
        || config.producer != Producer::Gpu
        || config.capture_profile
        || config.dma_reuse
        || config.frames != 1
        || config.height != 24
        || config.hold_msec != 0
    {
        return Err("release delay requires one fresh GPU DMA frame, height24, hold0".into());
    }
    Ok(())
}

// Generic only to test the real drop boundary without constructing a GPU export.
pub(super) fn release<T>(
    frame: T,
    grant: ContentGrant,
    resource: ContentResourceId,
    offer: u64,
) -> Result<()> {
    let frame = ManuallyDrop::new(frame);
    if offer == 0
        || grant.connection_epoch == 0
        || grant.content_grant_epoch == 0
        || resource
            != (ContentResourceId {
                id: 1,
                generation: 1,
            })
    {
        return Err("release-delay identity differs".into());
    }
    let record = |phase: &str, now: u64| {
        canvas::diagnostic_line(format_args!(
            "gpu_release_delay schema=1 phase={phase} offer={offer} connection_epoch={} content_grant_epoch={} resource={} generation={} monotonic_usec={now}",
            grant.connection_epoch, grant.content_grant_epoch, resource.id, resource.generation
        ));
    };
    let start = clock_usec();
    let until = start
        .checked_add(100_000)
        .ok_or("release-delay clock overflow")?;
    record("held", start);
    loop {
        let now = clock_usec();
        if now < start || now - start > 5_000_000 {
            return Err("release-delay clock/deadline".into());
        }
        if now >= until {
            record("resume", now);
            break;
        }
        std::thread::sleep(Duration::from_micros(until - now));
    }
    drop(ManuallyDrop::into_inner(frame));
    record("released", clock_usec());
    Ok(())
}

#[cfg(test)]
#[path = "../tests/support/release_delay.rs"]
mod tests;
