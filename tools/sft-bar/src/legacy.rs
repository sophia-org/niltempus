//! Held r10 demand/permit and explicit-retirement dialogue from gpu-bar-dma e730d3910.
use super::*;

fn present(
    wire: &mut Wire,
    grant: ContentGrant,
    allocation: &ContentAllocationResult,
    output: ContentOutputId,
    facts_generation: u64,
    generation: u64,
    counters: &mut RetireCounters,
) -> Result<()> {
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
    emit(
        7,
        Event::Permit {
            generation,
            demand_usec: demand_time,
            received_usec: clock_usec(),
        },
    )?;
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
    emit(
        7,
        Event::Offer {
            connection_epoch: grant.connection_epoch,
            content_grant_epoch: grant.content_grant_epoch,
            output: output.id,
            candidate_generation: generation,
            submitted_monotonic_usec: clock_usec(),
        },
    )?;
    wire.send(6, &records)?;
    let mut prepared = false;
    loop {
        let (_, record) = wire.receive_tx()?;
        let ShellContentRecord::CandidateOutcome(outcome) = record else {
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
        emit(
            7,
            Event::Terminal {
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
            },
        )?;
        // A named predecessor of a rejected candidate stays ours (Kept).
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
        counters.explicit_retires_sent += 1;
        let ShellContentRecord::ResourceReleased(released) = wire.receive()? else {
            return Err("missing retired resource release".into());
        };
        if released.grant != grant || released.resource != old || released.reason != 0 {
            return Err("wrong retired resource release".into());
        }
        counters.predecessor_releases_received += 1;
    }
    Ok(())
}

pub fn frame(
    wire: &mut Wire,
    grant: ContentGrant,
    allocation: &ContentAllocationResult,
    output: ContentOutputId,
    facts_generation: u64,
    generation: u64,
    counters: &mut RetireCounters,
) -> Result<()> {
    let resource = resource_for_frame(generation);
    let (width, height) = (allocation.pixel.width, allocation.pixel.height);
    let length = byte_len(width, height).ok_or("pixel length overflow")?;
    let metadata = RingUpload {
        resource: resource.id,
        resource_generation: resource.generation,
        allocation: allocation.allocation.id,
        allocation_generation: allocation.allocation.generation,
        output: output.id,
        output_generation: output.generation,
        topology_generation: facts_generation,
        width,
        height,
    };
    let mut fill_start_usec = 0;
    let mut begin_usec = 0;
    let offer = wire
        .client
        .image_upload_ring_fill(
            metadata,
            usize::try_from(length)?,
            Instant::now(),
            |pixels| {
                fill_start_usec = clock_usec();
                fill_cpu(pixels, generation);
                begin_usec = clock_usec();
            },
        )
        .map_err(|e| format!("ring upload: {e:?}"))?;
    crate::prepare_log::write(
        &mut std::io::stderr().lock(),
        generation,
        resource.id,
        fill_start_usec,
        begin_usec,
    )?;
    let slot = u64::from(
        wire.client
            .image_ring_offer(offer)
            .ok_or("ring offer lost")?
            .slot,
    );
    emit_ring(
        7,
        ring_journal::Event::RingOffer {
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
                begin_usec,
            },
            offer,
            slot,
            slot_generation: None,
            fill_start_usec,
        },
    )?;
    match wire.receive()? {
        ShellContentRecord::ResourceStatus(status)
            if status.grant == grant && status.resource == resource && status.status == 2 => {}
        _ => return Err("ring resource was not accepted exactly".into()),
    }
    let accepted_usec = clock_usec();
    let released_usec = wire.settle_ring(offer, resource)?;
    emit_ring(
        7,
        ring_journal::Event::RingAccepted {
            offer,
            resource_id: resource.id,
            resource_generation: resource.generation,
            accepted_usec,
        },
    )?;
    emit_ring(
        7,
        ring_journal::Event::SlotReleased {
            offer,
            slot,
            slot_generation: None,
            resource_id: resource.id,
            resource_generation: resource.generation,
            released_usec,
        },
    )?;
    present(
        wire,
        grant,
        allocation,
        output,
        facts_generation,
        generation,
        counters,
    )
}
