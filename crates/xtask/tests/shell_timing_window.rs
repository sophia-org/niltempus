//! Declared warmup windows over complete synthetic evidence. No devices.
use xtask::shell_timing::{Window, latency_report, latency_window};

/// Per-frame latencies in microseconds for one synthetic arm.
struct Timing {
    bar: fn(u64) -> u64,
    probe: Option<fn(u64) -> u64>,
    permit: fn(u64) -> u64,
    present: fn(u64) -> u64,
}

const STEP: u64 = 5_000_000;

fn arm(frames: u64, timing: &Timing, wire: &str) -> (String, String) {
    let probe = timing.probe.is_some();
    let mut journal = format!(
        "{{\"schema\":2,\"event\":\"start\",\"frames\":{frames},\"wire\":\"{wire}\",\"probe_bytes\":{}}}\n",
        if probe { 4194304 } else { 0 }
    );
    let mut host = String::new();
    for g in 1..=frames {
        let mut t = g * STEP;
        let upload = |id: u64, generation: u64, w: u32, h: u32, begin: u64| {
            format!(
                "{{\"schema\":2,\"event\":\"upload_offer\",\"upload\":{{\"connection_epoch\":1,\"content_grant_epoch\":2,\"output\":3,\"candidate_generation\":{g},\"resource_id\":{id},\"resource_generation\":{generation},\"width\":{w},\"height\":{h},\"total_bytes\":{},\"begin_usec\":{begin}}}}}\n",
                u64::from(w) * u64::from(h) * 4
            )
        };
        if let Some(latency) = timing.probe {
            journal += &upload(1, g, 1024, 1024, t);
            t += latency(g);
            journal += &format!(
                "{{\"schema\":2,\"event\":\"upload_accepted\",\"resource_id\":1,\"resource_generation\":{g},\"accepted_usec\":{t}}}\n"
            );
            t += 1;
            journal += &format!(
                "{{\"schema\":2,\"event\":\"probe_released\",\"resource_id\":1,\"resource_generation\":{g},\"released_usec\":{t}}}\n"
            );
        }
        let (id, generation) = ((g - 1) % 2 + 1 + u64::from(probe), (g - 1) / 2 + 1);
        journal += &upload(id, generation, 1920, 24, t);
        t += (timing.bar)(g);
        journal += &format!(
            "{{\"schema\":2,\"event\":\"upload_accepted\",\"resource_id\":{id},\"resource_generation\":{generation},\"accepted_usec\":{t}}}\n"
        );
        let demand = t + 1;
        t = demand + (timing.permit)(g);
        journal += &format!(
            "{{\"schema\":2,\"event\":\"permit\",\"generation\":{g},\"demand_usec\":{demand},\"received_usec\":{t}}}\n"
        );
        let submitted = t + 1;
        journal += &format!(
            "{{\"schema\":2,\"event\":\"offer\",\"connection_epoch\":1,\"content_grant_epoch\":2,\"output\":3,\"candidate_generation\":{g},\"submitted_monotonic_usec\":{submitted}}}\n"
        );
        let completed = submitted + (timing.present)(g);
        journal += &format!(
            "{{\"schema\":2,\"event\":\"terminal\",\"connection_epoch\":1,\"content_grant_epoch\":2,\"output\":3,\"candidate_generation\":{g},\"outcome\":\"presented\",\"kind\":2,\"reason\":0,\"observed_monotonic_usec\":{}}}\n",
            completed + 1
        );
        host += &format!(
            "sophia_shell_native_binding schema=1 connection_epoch=1 content_grant_epoch=2 output=3 candidate_generation={g} native_owner=5 native_frame={g} head=7 target_generation=8 heads=1 mode_refresh_millihz=120000\nsophia_shell_native_completion schema=1 output=3 native_owner=5 native_frame={g} heads=1 monotonic_usec={completed} timestamp_source=kernel missing_kernel_timestamp=0\n"
        );
    }
    journal += &format!(
        "{{\"schema\":2,\"event\":\"end\",\"offered\":{frames},\"completed\":{frames}}}\n"
    );
    (host, journal)
}

/// The two warmup frames are slow; measured frames are not.
fn timing() -> Timing {
    Timing {
        bar: |g| if g <= 2 { 9_000 } else { 100 + g },
        probe: None,
        permit: |g| if g <= 2 { 90_000 } else { 10 },
        present: |g| if g <= 2 { 50_000 } else { 1_000 + g },
    }
}

const WINDOW: Window = Window {
    warmup: 2,
    measured: 4,
};

#[test]
fn warmup_frames_are_excluded_from_percentiles_but_still_reported() {
    let (host, journal) = arm(6, &timing(), "files");
    let window = latency_window(&host, &journal, 120000, WINDOW).unwrap();
    assert_eq!(window.offered, 6);
    assert_eq!(window.sample_scope, "declared_warmup_excluded");
    let measured = &window.measured_distributions;
    let present = measured.candidate_to_native_presented.unwrap();
    assert_eq!(
        (present.count, present.p50_usec, present.max_usec),
        (4, 1004, 1006)
    );
    assert_eq!(measured.demand_to_permit.unwrap().max_usec, 10);
    assert_eq!(
        measured.presentation_upload_to_accepted.unwrap().max_usec,
        106
    );
    assert!(measured.probe_upload_to_accepted.is_none());
    let warmup = &window.warmup_distributions;
    assert_eq!(warmup.candidate_to_native_presented.unwrap().count, 2);
    assert_eq!(warmup.demand_to_permit.unwrap().p50_usec, 90_000);
    assert_eq!(
        window.measured_samples.candidate_to_native_presented.len(),
        4
    );
    // The whole-run report keeps every frame and its existing scope.
    let all = latency_report(&host, &journal, 120000, 6).unwrap();
    assert_eq!(
        all["sample_scope"],
        "all_offered_frames_no_warmup_exclusion"
    );
    assert_eq!(all["candidate_to_native_presented"]["max_usec"], 50_000);
    assert_eq!(all["candidate_to_native_presented"]["count"], 6);
    let json = serde_json::to_value(&window).unwrap();
    assert_eq!(json["window"]["warmup"], 2);
    assert_eq!(json["window"]["measured"], 4);
    assert!(json["warmup_distributions"].is_object());
    assert!(json["measured_distributions"].is_object());
}

#[test]
fn any_invalid_frame_including_warmup_rejects_the_arm() {
    let (host, journal) = arm(6, &timing(), "files");
    // A warmup frame that was not presented.
    let rejected = journal.replacen("\"outcome\":\"presented\"", "\"outcome\":\"rejected\"", 1);
    assert!(latency_window(&host, &rejected, 120000, WINDOW).is_err());
    // A warmup frame without its native completion.
    let missing: String = host
        .lines()
        .filter(|line| !line.contains("completion") || !line.contains("native_frame=1 "))
        .map(|line| format!("{line}\n"))
        .collect();
    assert!(latency_window(&missing, &journal, 120000, WINDOW).is_err());
    // Wrong mode, window total or empty window parts.
    assert!(latency_window(&host, &journal, 60000, WINDOW).is_err());
    for window in [
        Window {
            warmup: 2,
            measured: 3,
        },
        Window {
            warmup: 0,
            measured: 6,
        },
        Window {
            warmup: 6,
            measured: 0,
        },
    ] {
        assert!(latency_window(&host, &journal, 120000, window).is_err());
    }
}

#[test]
fn uploads_reaching_the_transfer_bound_are_listed_in_warmup_and_measured_frames() {
    let slow = Timing {
        bar: |g| if g == 1 { 2_000_000 } else { 100 },
        probe: Some(|g| if g == 5 { 2_000_001 } else { 20_000 }),
        permit: |_| 10,
        present: |_| 1_000,
    };
    let (host, journal) = arm(6, &slow, "files");
    let window = latency_window(&host, &journal, 120000, WINDOW).unwrap();
    let reached: Vec<_> = window
        .transfer_bound_reached
        .iter()
        .map(|b| (b.phase, b.purpose, b.candidate_generation))
        .collect();
    assert_eq!(
        reached,
        [
            ("warmup", "presentation", 1),
            ("measured", "transfer_probe", 5)
        ]
    );
    // Exclusion changes percentiles only: the warmup upload is not measured.
    assert_eq!(
        window
            .measured_distributions
            .presentation_upload_to_accepted
            .unwrap()
            .max_usec,
        100
    );
    assert_eq!(
        window
            .measured_distributions
            .probe_upload_to_accepted
            .unwrap()
            .max_usec,
        2_000_001
    );
}
