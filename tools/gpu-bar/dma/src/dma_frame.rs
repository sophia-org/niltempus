//! Arm G's DMA-BUF frames: one export per frame by default, or, for the
//! diagnostic `dma_reuse` arm, one export redrawn every frame after its
//! previous offer settled. Offers, statuses and the journal are identical.
use super::{Event, Result, Upload, Wire, canvas, clock_usec, emit, expect_status};
use sophia_shell_client::ProducerMetadata;
use sophia_shell_gpu::DmaFrame;
use sophia_shell_protocol::*;
use std::time::Instant;

/// The DMA backing policy and, when reusing, the one export kept between
/// frames. A frame enters `retained` only after its offer settled Accepted
/// with a capture-complete release and `image_forget` succeeded.
pub(super) struct Backing {
    reuse: bool,
    retained: Option<DmaFrame>,
}

impl Backing {
    pub(super) fn new(reuse: bool) -> Self {
        Self {
            reuse,
            retained: None,
        }
    }

    pub(super) fn mode(&self) -> &'static str {
        if self.reuse { "reuse" } else { "fresh" }
    }
}

/// Redraw `frame` with this generation's shade, keeping its export.
///
/// # Safety
///
/// As `GpuPreview::render_dma_buf_reusing`: every foreign access to `frame`
/// finished and nothing can send it again. Only [`dma_frame`] proves this,
/// from the settle it owns.
unsafe fn reuse_dma(frame: DmaFrame, generation: u64) -> Result<DmaFrame> {
    let (width, height) = (frame.width(), frame.height());
    Ok(canvas::with_gpu(|gpu| {
        // SAFETY: forwarded from this function's contract.
        unsafe { gpu.render_dma_buf_reusing(&mut canvas::scene(width, height, generation), frame) }
    })?)
}

/// One DMA-BUF frame through Accepted and capture-complete release.
#[allow(clippy::too_many_arguments)]
pub(super) fn dma_frame(
    wire: &mut Wire,
    backing: &mut Backing,
    grant: ContentGrant,
    resource: ContentResourceId,
    generation: u64,
    width: u32,
    height: u32,
    length: u64,
    output: ContentOutputId,
    allocation_id: u64,
    facts_generation: u64,
) -> Result<()> {
    let prepare_usec = clock_usec();
    let frame = match backing.retained.take() {
        // SAFETY: the frame entered `retained` only after the previous
        // offer settled Accepted with Session's capture-complete release
        // (sent after its signalled read) and `image_forget` succeeded. The
        // only descriptor duplicate went to that forgotten offer, so nothing
        // queued can send the buffer again.
        Some(previous) => unsafe { reuse_dma(previous, generation) }?,
        None => canvas::render_dma(width, height, generation)?,
    };
    canvas::prepared(generation, resource.id, prepare_usec, clock_usec());
    // As in the comparator, the offer is journalled after pixel preparation.
    emit(Event::UploadOffer {
        upload: Upload {
            connection_epoch: grant.connection_epoch,
            content_grant_epoch: grant.content_grant_epoch,
            output: output.id,
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
        output_id: output.id,
        allocation_id,
        topology_generation: facts_generation,
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
    let settled =
        expect_status(wire, grant, resource, 2).and_then(|()| wire.settle(offer, resource));
    if let Err(error) = settled {
        // Release is unproven: keep the export allocated until process exit.
        std::mem::forget(frame);
        return Err(error);
    }
    // Session holds its own captured copy; the export may now be freed or,
    // when reusing, kept for the next redraw.
    if backing.reuse {
        backing.retained = Some(frame);
    } else {
        drop(frame);
    }
    emit(Event::UploadAccepted {
        resource_id: resource.id,
        resource_generation: resource.generation,
        accepted_usec: clock_usec(),
    })?;
    Ok(())
}
