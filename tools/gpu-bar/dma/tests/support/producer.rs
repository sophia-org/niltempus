use super::*;

#[test]
fn cpu_open_and_fill_need_no_gpu_and_match_external_shades() {
    Producer::Cpu
        .open(1, Source::OwnedRing { slots: 2 }, false)
        .unwrap();
    assert!(canvas::with_gpu(|_| Ok(())).is_err());
    let mut storage = vec![0xA5; 1920 * 24 * 4 + 8];
    for (generation, shade) in [
        (1, 1),
        (119, 119),
        (120, 120),
        (127, 127),
        (128, 0),
        (129, 1),
        (2120, 72),
        (u64::MAX, 127),
    ] {
        fill_cpu(&mut storage[4..1920 * 24 * 4 + 4], generation);
        assert_eq!(&storage[..4], &[0xA5; 4]);
        assert_eq!(&storage[1920 * 24 * 4 + 4..], &[0xA5; 4]);
        assert!(
            storage[4..1920 * 24 * 4 + 4]
                .chunks_exact(4)
                .all(|p| p == [shade, shade, shade, 255])
        );
    }
}

#[test]
#[should_panic]
fn partial_pixels_are_not_silently_left_unwritten() {
    fill_cpu(&mut [0; 5], 1);
}

#[test]
fn direct_fill_cannot_reuse_a_slot_before_its_exact_release() {
    use sophia_image_producer::{ResourceStatusKind, RingProducer, RingProducerConfig, RingUpload};
    use sophia_shell_protocol::shell_files::{RingEvent, RingEventKind, RingIdentity};

    let identity = RingIdentity {
        connection_epoch: 7,
        content_grant_epoch: 3,
        channel_generation: 9,
        ring: 1,
        generation: 1,
    };
    let event = |kind| RingEvent {
        kind,
        identity,
        sequence: 5,
        offer_id: 0,
        slot: 0,
        slot_generation: 0,
        resource: 0,
        resource_generation: 0,
        reason: 0,
        changed_generation: false,
    };
    let mut ring = RingProducer::new(
        RingProducerConfig {
            identity,
            slots: 2,
            capacity: 8,
            max_resource_bytes: 8,
            transfer_timeout_ms: 2000,
        },
        0,
    )
    .unwrap();
    ring.registration().unwrap();
    ring.ring_event(event(RingEventKind::RingAdmitted), 1)
        .unwrap();
    ring.source_packet().unwrap();
    ring.source_sent().unwrap();
    ring.ring_event(event(RingEventKind::RingReady), 2).unwrap();
    let mut first_offer = 0;
    for generation in 1..=2 {
        let mut slot = ring.acquire_slot(3).unwrap();
        assert_eq!(slot.slot(), (generation - 1) as u32);
        fill_cpu(slot.pixels_mut(), generation);
        let (offer, _) = slot
            .commit(
                RingUpload {
                    resource: generation,
                    resource_generation: 1,
                    allocation: 21,
                    allocation_generation: 1,
                    output: 31,
                    output_generation: 1,
                    topology_generation: 41,
                    width: 2,
                    height: 1,
                },
                8,
                4,
            )
            .unwrap();
        if generation == 1 {
            first_offer = offer;
        }
    }
    assert!(ring.acquire_slot(5).is_err());
    ring.resource_status(1, 1, ResourceStatusKind::Accepted, 6)
        .unwrap();
    assert!(ring.acquire_slot(7).is_err());
    ring.ring_event(
        RingEvent {
            offer_id: first_offer,
            slot_generation: 2,
            resource: 1,
            resource_generation: 1,
            ..event(RingEventKind::SlotReleased)
        },
        8,
    )
    .unwrap();
    let mut slot = ring.acquire_slot(9).unwrap();
    assert_eq!(slot.slot(), 0);
    assert_eq!(slot.pixels_mut(), &[1, 1, 1, 255, 1, 1, 1, 255]);
    fill_cpu(slot.pixels_mut(), 3);
    assert_eq!(slot.pixels_mut(), &[3, 3, 3, 255, 3, 3, 3, 255]);
}
