use super::retained_ring_config;
use serde_json::{Value, json};

#[test]
fn retained_config_refuses_changed_geometry_and_extra_peer_modes() {
    let valid = json!({
        "wire": "files", "source": "owned_ring", "output": 1,
        "frames": 2120, "hold_msec": 60000, "height": 512,
        "workload": "retained", "retained_resources": 4, "ring_slots": 2
    });
    assert!(retained_ring_config(&valid));
    for key in valid.as_object().unwrap().keys() {
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(!retained_ring_config(&missing), "missing {key}");
    }
    for (key, value) in [
        ("wire", json!("ipc")),
        ("source", json!("sealed")),
        ("workload", json!("fresh")),
        ("height", json!(24)),
        ("retained_resources", json!(3)),
        ("retained_resources", json!(5)),
        ("retained_resources", json!("4")),
        ("ring_slots", json!(3)),
        ("producer", json!("cpu")),
        ("interval_msec", json!(16)),
        ("retire_on_present", json!(false)),
        ("capture_profile", json!(false)),
        ("probe_bytes", json!(0)),
    ] {
        let mut changed = valid.clone();
        changed[key] = value;
        assert!(!retained_ring_config(&changed), "changed {key}");
    }
    for invalid in [Value::Null, json!([]), json!(true)] {
        assert!(!retained_ring_config(&invalid));
    }
}
