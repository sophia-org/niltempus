use xtask::shell_timing::{Offer, join};

fn offer(generation: u64) -> Offer {
    Offer {
        connection_epoch: 1,
        content_grant_epoch: 2,
        output: 3,
        candidate_generation: generation,
        submitted_monotonic_usec: 100,
        outcome: "presented".into(),
    }
}

fn binding(candidate: u64, frame: u64, head: u64, count: u64) -> String {
    format!(
        "sophia_shell_native_binding schema=1 connection_epoch=1 content_grant_epoch=2 output=3 candidate_generation={candidate} native_owner=5 native_frame={frame} head={head} target_generation=8 heads={count} mode_refresh_millihz=60000\n"
    )
}

fn completion(frame: u64, heads: u64, time: u64) -> String {
    format!(
        "sophia_shell_native_completion schema=1 output=3 native_owner=5 native_frame={frame} heads={heads} monotonic_usec={time} timestamp_source=kernel missing_kernel_timestamp=0\n"
    )
}

fn host() -> String {
    binding(4, 6, 7, 2) + &binding(4, 6, 9, 2) + &completion(6, 2, 150)
}

#[test]
fn exact_mirror_join_uses_kernel_time_and_preserves_every_offer() {
    let samples = join(&host(), &[offer(4)], 60000, 1).unwrap();
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].latency_usec, 50);
    assert_eq!(samples[0].native_frame, 6);
    assert!(join(&host(), &[offer(4), offer(5)], 60000, 2).is_err());
    assert!(join(&host(), &[offer(4)], 60000, 2).is_err());
}

#[test]
fn repeated_redraws_use_first_exact_completion_not_later_frame_time() {
    let log = host() + &binding(4, 10, 7, 2) + &binding(4, 10, 9, 2) + &completion(10, 2, 200);
    assert_eq!(
        join(&log, &[offer(4)], 60000, 1).unwrap()[0].latency_usec,
        50
    );
    assert_eq!(
        join(&(host() + &host()), &[offer(4)], 60000, 1)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn native_identity_clock_and_topology_mismatches_fail_closed() {
    for bad in [
        host().replace("native_owner=5", "native_owner=0"),
        host().replace(
            "timestamp_source=kernel",
            "timestamp_source=observation_fallback",
        ),
        host().replace("missing_kernel_timestamp=0", "missing_kernel_timestamp=1"),
        host().replace("monotonic_usec=150", "monotonic_usec=99"),
        host().replace("mode_refresh_millihz=60000", "mode_refresh_millihz=120000"),
        binding(4, 6, 7, 2) + &completion(6, 2, 150),
        binding(4, 6, 7, 1) + &completion(6, 2, 150),
        binding(4, 6, 7, 1) + &completion(99, 1, 150),
        host() + &completion(6, 2, 151),
        host() + &binding(4, 6, 7, 2).replace("target_generation=8", "target_generation=9"),
        host() + &binding(4, 10, 7, 1),
        host().replace("head=7", "head=7 head=7"),
        host().replace("head=7", "head=7 invented=1"),
    ] {
        assert!(join(&bad, &[offer(4)], 60000, 1).is_err(), "accepted {bad}");
    }
}

#[test]
fn offers_cannot_hide_rejection_duplicate_identity_or_same_role_frame_alias() {
    let mut rejected = offer(4);
    rejected.outcome = "superseded".into();
    assert!(join(&host(), &[rejected], 60000, 1).is_err());
    assert!(join(&host(), &[offer(4), offer(4)], 60000, 2).is_err());
    let alias = host() + &binding(5, 6, 7, 2) + &binding(5, 6, 9, 2);
    assert!(join(&alias, &[offer(4), offer(5)], 60000, 2).is_err());
    let other_role = host()
        + &host()
            .replace("connection_epoch=1", "connection_epoch=11")
            .replace("content_grant_epoch=2", "content_grant_epoch=12");
    let mut other = offer(4);
    other.connection_epoch = 11;
    other.content_grant_epoch = 12;
    assert_eq!(
        join(&other_role, &[offer(4), other], 60000, 2)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn command_joins_explicit_files_without_a_checkout_and_refuses_missing_offers() {
    let root = std::env::temp_dir().join(format!("shell-timing-cli-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let host_path = root.join("host.log");
    let client_path = root.join("client.json");
    std::fs::write(&host_path, host()).unwrap();
    std::fs::write(&client_path, r#"[{"connection_epoch":1,"content_grant_epoch":2,"output":3,"candidate_generation":4,"submitted_monotonic_usec":100,"outcome":"presented"}]"#).unwrap();
    let invoke = |expected: &str| {
        std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
            .current_dir(&root)
            .args(["shell-timing", "join"])
            .arg(&host_path)
            .arg(&client_path)
            .args(["60000", expected])
            .output()
            .unwrap()
    };
    let result = invoke("1");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["verdict"], "joined_only");
    assert_eq!(report["samples"][0]["latency_usec"], 50);
    assert!(!invoke("2").status.success());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_card_clock_evidence_is_validated_without_rewriting_legacy_samples() {
    for value in ["0", "1"] {
        let log = host().replace(
            "missing_kernel_timestamp=0",
            &format!("missing_kernel_timestamp=0 drm_timestamp_monotonic={value}"),
        );
        assert_eq!(
            join(&log, &[offer(4)], 60000, 1).unwrap()[0].latency_usec,
            50
        );
    }
    let invalid = host().replace(
        "missing_kernel_timestamp=0",
        "missing_kernel_timestamp=0 drm_timestamp_monotonic=2",
    );
    assert!(join(&invalid, &[offer(4)], 60000, 1).is_err());
}
