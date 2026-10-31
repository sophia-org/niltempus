//! Substitute only the import descriptor after a real, completed GPU export.
use super::{Config, Producer, Result, Source, Wire, canvas};
use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, ftruncate, memfd_create};
use sophia_shell_client::ProducerMetadata;
use sophia_shell_gpu::DmaFrame;
use sophia_shell_protocol::*;
use std::mem::ManuallyDrop;
use std::os::fd::OwnedFd;
use std::time::Instant;

pub(super) fn validate_config(config: &Config) -> Result<()> {
    if config.source != Source::DmaBuf
        || config.producer != Producer::Gpu
        || config.capture_profile
        || config.dma_reuse
        || config.frames != 1
        || config.height != 24
        || config.hold_msec != 0
    {
        return Err("import refusal requires one fresh GPU DMA frame, height24, hold0".into());
    }
    Ok(())
}

fn replacement(bytes: u64) -> Result<OwnedFd> {
    if !(1920 * 24 * 4..=i64::MAX as u64).contains(&bytes) {
        return Err("invalid import backing length".into());
    }
    let fd = memfd_create(
        c"gpu-invalid-import-not-dmabuf",
        MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
    )?;
    ftruncate(&fd, bytes)?;
    fcntl_add_seals(
        &fd,
        SealFlags::GROW | SealFlags::SHRINK | SealFlags::WRITE | SealFlags::SEAL,
    )?;
    Ok(fd)
}

fn refusal(
    record: ShellContentRecord,
    grant: ContentGrant,
    resource: ContentResourceId,
) -> Result<ContentResourceStatus> {
    match record {
        ShellContentRecord::ResourceStatus(status)
            if status.grant == grant
                && status.resource == resource
                && status.status == 3
                && matches!(status.reason, 3 | 9)
                && status.next_ordinal == 0
                && status.admitted_bytes == 0 =>
        {
            Ok(status)
        }
        other => Err(format!("not a matching consumer import refusal: {other:?}").into()),
    }
}

pub(super) fn offer(
    wire: &mut Wire,
    frame: DmaFrame,
    metadata: ProducerMetadata,
    grant: ContentGrant,
    resource: ContentResourceId,
) -> Result<()> {
    // Even errors leave the real export allocated until this diagnostic exits.
    let frame = ManuallyDrop::new(frame);
    let bytes = frame.source_bytes();
    let admission = wire.client.image_begin_dma_buf(
        metadata,
        replacement(bytes)?,
        frame.offset(),
        frame.stride(),
        Instant::now(),
    );
    let offer = admission
        .map_err(|e| format!("local invalid-import admission failed: {:?}", e.error))?
        .offer;
    canvas::diagnostic_line(format_args!(
        "gpu_invalid_import schema=1 transport=sophia status=injected offer={offer} connection_epoch={} content_grant_epoch={} resource={} generation={} bytes={bytes}",
        grant.connection_epoch, grant.content_grant_epoch, resource.id, resource.generation
    ));
    let status = refusal(wire.receive()?, grant, resource)?;
    canvas::diagnostic_line(format_args!(
        "gpu_invalid_import_refused schema=1 transport=sophia offer={offer} connection_epoch={} content_grant_epoch={} resource={} generation={} status={} reason={} released=0",
        grant.connection_epoch,
        grant.content_grant_epoch,
        resource.id,
        resource.generation,
        status.status,
        status.reason
    ));
    std::process::exit(42);
}

#[cfg(test)]
#[path = "../tests/support/invalid_import.rs"]
mod tests;
