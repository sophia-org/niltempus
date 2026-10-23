use super::*;

fn record(event: Event<'_>) -> String {
    let mut out = Vec::new();
    write(&mut out, event).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn unexposed_sdk_fields_are_explicit_nulls() {
    assert_eq!(
        record(Event::SlotReleased {
            offer: 3,
            slot: 0,
            slot_generation: None,
            resource_id: 1,
            resource_generation: 2,
            released_usec: 9,
        }),
        "{\"schema\":7,\"event\":\"slot_released\",\"offer\":3,\"slot\":0,\
         \"slot_generation\":null,\"resource_id\":1,\"resource_generation\":2,\
         \"released_usec\":9}\n"
    );
    assert!(
        record(Event::RingReady {
            connection_epoch: 1,
            content_grant_epoch: 1,
            channel: 1,
            ring: 1,
            ring_generation: 1,
            slots: 2,
            pixel_bytes: 4,
            admitted_usec: None,
            ready_usec: 5,
        })
        .contains("\"admitted_usec\":null,\"ready_usec\":5}")
    );
}
