use super::frame_ring_config;
use serde_json::json;
#[test]
fn exact_sft_config_refuses_missing_extra_or_wrong_mode_keys() {
    let good = json!({"wire":"files","source":"owned_ring","producer":"cpu","ring_slots":2,
        "output":1,"frames":2120,"height":24,"hold_msec":60000,"frame_transactions":true});
    assert!(frame_ring_config(&good));
    for key in good.as_object().unwrap().keys() {
        let mut changed = good.clone();
        changed.as_object_mut().unwrap().remove(key);
        assert!(!frame_ring_config(&changed), "missing {key}");
    }
    for (key, value) in [
        ("wire", json!("ipc")),
        ("source", json!("dma_buf")),
        ("producer", json!("gpu")),
        ("ring_slots", json!(3)),
        ("frame_transactions", json!(false)),
        ("frame_transactions", json!("true")),
        ("retire_on_present", json!(false)),
        ("capture_profile", json!(false)),
    ] {
        let mut changed = good.clone();
        changed[key] = value;
        assert!(!frame_ring_config(&changed), "{key}");
    }
    assert!(!frame_ring_config(&json!(null)));
}

#[test]
fn frame_peer_terminal_only_key_is_optional_and_boolean() {
    let mut config = serde_json::json!({"wire":"files","source":"owned_ring","producer":"cpu","ring_slots":2,
        "output":1,"frames":2120,"height":24,"hold_msec":60000,"frame_transactions":true});
    for value in [true, false] {
        config["terminal_only"] = value.into();
        assert!(frame_ring_config(&config));
    }
    for value in [
        serde_json::json!(null),
        serde_json::json!("true"),
        serde_json::json!(1),
    ] {
        config["terminal_only"] = value;
        assert!(!frame_ring_config(&config));
    }
}
