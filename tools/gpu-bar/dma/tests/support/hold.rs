use super::*;

#[test]
fn diagnostic_recipe_is_separate_and_bounded() {
    let text =
        r#"{"wire":"files","output":1,"frames":3,"height":24,"hold_msec":0,"source":"dma_buf"}"#;
    validate_config(&super::super::parse_config(text).unwrap()).unwrap();
    for bad in [
        text.replace("\"frames\":3", "\"frames\":2120"),
        text.replace("\"height\":24", "\"height\":512"),
        text.replace("\"hold_msec\":0", "\"hold_msec\":60000"),
    ] {
        assert!(validate_config(&super::super::parse_config(&bad).unwrap()).is_err());
    }
    let ring = r#"{"wire":"files","output":1,"frames":3,"height":24,"hold_msec":0,"source":"owned_ring","ring_slots":2}"#;
    assert!(validate_config(&super::super::parse_config(ring).unwrap()).is_err());
    assert!(
        validate_config(
            &super::super::parse_config(&ring.replace('}', ",\"producer\":\"cpu\"}")).unwrap()
        )
        .is_err()
    );
}

#[test]
fn ack_is_for_one_ready_challenge_not_elapsed_time() {
    let nonce = "0123456789abcdef0123456789abcdef";
    let valid = format!(r#"{{"schema":1,"status":"ack","generation":2,"nonce":"{nonce}"}}"#);
    ack(valid.as_bytes(), 2, nonce).unwrap();
    for bad in [
        valid.replace("\"schema\":1", "\"schema\":true"),
        valid.replace("\"schema\":1", "\"schema\":1.0"),
        valid.replace("\"generation\":2", "\"generation\":1"),
        valid.replace("\"generation\":2", "\"generation\":3"),
        valid.replace("ack", "ready"),
        valid.replace(nonce, "different"),
        valid.replace('}', ",\"extra\":1}"),
        valid.replace('}', ",\"generation\":2}"),
        format!("{valid}\n{valid}"),
        String::new(),
    ] {
        assert!(ack(bad.as_bytes(), 2, nonce).is_err(), "{bad}");
    }
}

#[test]
fn colour_scene_refuses_any_other_geometry_or_generation() {
    use super::super::canvas::gpu_scene as scene;
    let _ordinary = scene::scene(1920, 24, 7);
    for generation in 1..=3 {
        assert!(scene::colour_scene(1920, 24, generation).is_ok());
    }
    for (width, height, generation) in [(1919, 24, 1), (1920, 25, 1), (1920, 24, 0), (1920, 24, 4)]
    {
        assert!(scene::colour_scene(width, height, generation).is_err());
    }
}
