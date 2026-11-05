use super::*;
const CONFIG: &str = r#"{"wire":"files","source":"owned_ring","producer":"cpu","ring_slots":2,"output":1,"frames":2120,"height":24,"hold_msec":60000}"#;
#[test]
fn mode_is_explicit_and_the_workload_is_bounded() {
    assert!(!Config::parse(CONFIG).unwrap().frame_transactions);
    let mut value: serde_json::Value = serde_json::from_str(CONFIG).unwrap();
    value["frame_transactions"] = true.into();
    assert_eq!(
        Config::parse(&value.to_string()).is_ok(),
        cfg!(feature = "experimental-frame-transactions")
    );
    value["frame_transactions"] = false.into();
    for (key, bad) in [
        ("frame_transactions", serde_json::json!("true")),
        ("frame_transactions", serde_json::json!(null)),
        ("ring_slots", serde_json::json!(3)),
        ("producer", serde_json::json!("gpu")),
        ("source", serde_json::json!("dma_buf")),
        ("wire", serde_json::json!("ipc")),
        ("frames", serde_json::json!(0)),
        ("frames", serde_json::json!(10001)),
        ("height", serde_json::json!(513)),
        ("output", serde_json::json!(0)),
        ("hold_msec", serde_json::json!(600001)),
        ("retire_on_present", serde_json::json!(true)),
    ] {
        let mut bad_value = value.clone();
        bad_value[key] = bad;
        assert!(Config::parse(&bad_value.to_string()).is_err(), "{key}");
    }
}
#[test]
fn pixels_match_the_held_shade_workload_and_keep_alpha_opaque() {
    for generation in [1, 127, 128, 129, 2120] {
        let mut pixels = vec![0xa5; 1920 * 24 * 4];
        fill_cpu(&mut pixels, generation);
        assert!(pixels.chunks_exact(4).all(|p| p
            == [
                (generation % 128) as u8,
                (generation % 128) as u8,
                (generation % 128) as u8,
                255
            ]));
    }
}

#[test]
fn pixel_fill_handles_short_unaligned_slices_without_touching_neighbors() {
    for pixels in [0, 1, 3, 4, 7, 8, 17, 65] {
        for prefix in 0..4 {
            for generation in [0, 127, 128, u64::MAX] {
                let end = prefix + pixels * 4;
                let mut bytes = vec![0xa5; end + 4];
                fill_cpu(&mut bytes[prefix..end], generation);
                assert!(bytes[..prefix].iter().all(|&b| b == 0xa5));
                assert!(bytes[end..].iter().all(|&b| b == 0xa5));
                for pixel in bytes[prefix..end].chunks_exact(4) {
                    let shade = (generation % 128) as u8;
                    assert_eq!(pixel, [shade, shade, shade, 255]);
                }
            }
        }
    }
}
