use super::*;

const DMA: &str = r#"{"wire":"files","output":1,"frames":2120,"height":512,"hold_msec":60000,"source":"dma_buf"}"#;
const RING: &str = r#"{"wire":"files","output":1,"frames":2120,"height":24,"hold_msec":60000,"source":"owned_ring","ring_slots":2}"#;

#[test]
fn each_source_takes_exactly_its_keys() {
    assert_eq!(parse_config(DMA).unwrap().source, Source::DmaBuf);
    let ring = parse_config(RING).unwrap();
    assert_eq!(ring.source, Source::OwnedRing { slots: 2 });
    assert_eq!((ring.frames, ring.height), (2120, 24));
    // DMA refuses ring_slots; the ring requires it, within 2..=4.
    assert!(parse_config(&DMA.replace('}', r#","ring_slots":2}"#)).is_err());
    assert!(parse_config(&RING.replace(r#","ring_slots":2"#, "")).is_err());
    for slots in ["1", "5", "\"2\""] {
        assert!(parse_config(&RING.replace(":2}", &format!(":{slots}}}"))).is_err());
    }
    assert!(parse_config(&RING.replace("\"files\"", "\"ipc\"")).is_err());
    assert!(parse_config(&DMA.replace("dma_buf", "sealed")).is_err());
    assert!(parse_config(&DMA.replace('}', r#","extra":1}"#)).is_err());
    assert!(parse_config(&RING.replace("\"height\":24", "\"height\":513")).is_err());
}
