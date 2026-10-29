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

#[test]
fn capture_profile_is_explicit_boolean_and_bounded() {
    assert!(!parse_config(RING).unwrap().capture_profile);
    let profiled = RING.replace('}', r#","capture_profile":true}"#);
    assert!(parse_config(&profiled).unwrap().capture_profile);
    let dma = DMA
        .replace("512", "24")
        .replace('}', r#","capture_profile":true}"#);
    assert!(parse_config(&dma).unwrap().capture_profile);
    for value in ["null", "1", "\"true\""] {
        assert!(parse_config(&profiled.replace("true", value)).is_err());
    }
    for changed in [
        profiled.replace("2120", "1000"),
        profiled.replace("60000", "120000"),
        profiled.replace("\"height\":24", "\"height\":512"),
    ] {
        assert!(parse_config(&changed).is_err());
    }
}

#[test]
fn dma_reuse_is_a_dma_only_boolean_diagnostic() {
    let profiled = DMA
        .replace("512", "24")
        .replace('}', r#","capture_profile":true}"#);
    // Absent or explicitly false is the unchanged fresh arm.
    assert!(!parse_config(DMA).unwrap().dma_reuse);
    let fresh = parse_config(&DMA.replace('}', r#","dma_reuse":false}"#)).unwrap();
    assert!(!fresh.dma_reuse && !fresh.capture_profile);
    let reuse = parse_config(&profiled.replace('}', r#","dma_reuse":true}"#)).unwrap();
    assert!(reuse.dma_reuse && reuse.capture_profile);
    // Reuse is diagnostic only: it needs capture_profile and its bounds.
    assert!(parse_config(&DMA.replace('}', r#","dma_reuse":true}"#)).is_err());
    assert!(
        parse_config(&DMA.replace('}', r#","capture_profile":false,"dma_reuse":true}"#)).is_err()
    );
    for value in ["null", "1", "\"true\""] {
        let text = profiled.replace('}', &format!(r#","dma_reuse":{value}}}"#));
        assert!(parse_config(&text).is_err(), "{value}");
    }
    // Any presence outside dma_buf is refused, even false.
    for value in ["true", "false"] {
        let ring = RING.replace('}', &format!(r#","dma_reuse":{value}}}"#));
        assert!(parse_config(&ring).is_err(), "{value}");
    }
}

#[test]
fn cpu_is_explicit_and_only_the_plain_owned_ring_path() {
    assert_eq!(parse_config(RING).unwrap().producer, Producer::Gpu);
    assert_eq!(parse_config(DMA).unwrap().producer, Producer::Gpu);
    let cpu = RING.replace('}', r#","producer":"cpu"}"#);
    assert_eq!(parse_config(&cpu).unwrap().producer, Producer::Cpu);
    assert_eq!(
        parse_config(&RING.replace('}', r#","producer":"gpu"}"#)).unwrap(),
        parse_config(RING).unwrap()
    );
    for bad in ["null", "true", "1", "[]", "{}", "\"CPU\"", "\"other\""] {
        assert!(parse_config(&RING.replace('}', &format!(",\"producer\":{bad}}}"))).is_err());
    }
    for bad in [
        DMA.replace('}', r#","producer":"cpu"}"#),
        cpu.replace('}', r#","capture_profile":true}"#),
        cpu.replace('}', r#","dma_reuse":false}"#),
    ] {
        assert!(parse_config(&bad).is_err(), "{bad}");
    }
}
