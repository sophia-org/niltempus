use xtask::{shell_timing::Offer, shell_timing_cost::measure};

#[test]
fn sealed_cost_uses_the_same_logical_bytes_and_reports_cpu_per_frame() {
    use xtask::{shell_timing::ImageSource, shell_timing_cost::measure_journal};
    let bytes = measure_journal(&host(), &peer(), &journal(), 2)
        .unwrap()
        .normalized
        .unwrap();
    let input = journal().replacen(
        "\"event\":\"start\"",
        "\"event\":\"start\",\"source\":\"sealed\"",
        1,
    );
    let sealed = measure_journal(&host(), &peer(), &input, 2)
        .unwrap()
        .normalized
        .unwrap();
    assert_eq!(sealed.source, ImageSource::Sealed);
    assert_eq!(sealed.transferred_bytes, bytes.uploaded_bytes);
    assert_eq!(sealed.uploaded_bytes, sealed.transferred_bytes);
    assert_eq!(
        sealed.cpu_ns_per_uploaded_mib.lower,
        bytes.cpu_ns_per_uploaded_mib.lower
    );
    assert_eq!(
        sealed.cpu_ns_per_uploaded_mib.upper,
        bytes.cpu_ns_per_uploaded_mib.upper
    );
    assert_eq!(sealed.cpu_ns_per_frame.lower, bytes.cpu_ns_per_frame.lower);
    assert_eq!(sealed.cpu_ns_per_frame.upper, bytes.cpu_ns_per_frame.upper);
    assert!(sealed.cpu_ns_per_frame.lower > 0);
}

fn cpu_only(log: &str) -> String {
    log.lines()
        .map(|line| {
            line.split_whitespace()
                .map(|word| {
                    if word == "schema=1" {
                        "schema=2 mode=cpu-only"
                    } else if word.starts_with("allocations=") {
                        "allocations=0"
                    } else if word.starts_with("reallocations=") {
                        "reallocations=0"
                    } else if word.starts_with("deallocations=") {
                        "deallocations=0"
                    } else if word.starts_with("bytes_allocated=") {
                        "bytes_allocated=0"
                    } else {
                        word
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
                + "\n"
        })
        .collect()
}

#[test]
fn cpu_only_retains_cpu_bounds_but_never_claims_allocation_results() {
    use xtask::shell_timing_cost::{for_mode, measure_journal};
    let host = cpu_only(&host());
    let peer = cpu_only(&peer());
    let report = measure_journal(&host, &peer, &journal(), 2).unwrap();
    assert_eq!(report.instrumentation, "cpu-only");
    assert_eq!(report.allocation_scope, "not_measured");
    assert!(report.whole_workload.allocation_operations.is_none());
    let normalized = report.normalized.unwrap();
    assert!(normalized.allocation_operations_per_frame.is_none());
    assert_eq!(
        (
            normalized.cpu_ns_per_uploaded_mib.lower,
            normalized.cpu_ns_per_uploaded_mib.upper
        ),
        (12800, 18489)
    );
    let capture = host.clone() + &peer;
    assert!(for_mode("cpu-only", &capture, &offers()).unwrap().is_some());
    assert!(for_mode("rust-global", &capture, &offers()).is_err());
    assert!(for_mode("none", &capture, &offers()).is_err());
    assert!(measure(&host, &self::peer(), &offers()).is_err());
    assert!(
        measure(
            &host.replace("allocations=0", "allocations=1"),
            &peer,
            &offers()
        )
        .is_err()
    );
    assert!(
        measure(
            &host.replace("mode=cpu-only", "mode=unknown"),
            &peer,
            &offers()
        )
        .is_err()
    );
    assert!(
        measure(
            &host.replacen("schema=2 mode=cpu-only", "schema=1", 1),
            &peer,
            &offers()
        )
        .is_err()
    );
    assert!(measure(&host.replace("pid=10 ", ""), &peer, &offers()).is_err());
}

#[test]
fn profiling_records_cannot_supply_cost_or_latency_acceptance_arms() {
    use xtask::shell_timing_cost::for_mode;
    let capture = (host() + &peer()).replace("schema=1", "schema=2 mode=profile");
    assert!(for_mode("profile", &capture, &offers()).unwrap().is_some());
    assert!(for_mode("rust-global", &capture, &offers()).is_err());
    assert!(for_mode("cpu-only", &capture, &offers()).is_err());
    assert!(for_mode("none", &capture, &offers()).is_err());
}

fn row(process: &str, stage: &str, sequence: u64, time: u64, count: u64) -> String {
    let pid = if process == "session" { 10 } else { 20 };
    format!(
        "sophia_comparator_counters schema=1 process={process} pid={pid} stage={stage} sequence={sequence} before_usec={time} after_usec={} cpu_ns={} allocations={count} reallocations=0 deallocations=0 bytes_allocated={}\n",
        time + 1,
        count * 100,
        count * 16
    )
}

fn host() -> String {
    (0..7)
        .map(|i| {
            row(
                "session",
                if i == 0 {
                    "start"
                } else if i == 6 {
                    "end"
                } else {
                    "sample"
                },
                i,
                10 + i * 10,
                i * 10,
            )
        })
        .collect()
}

fn peer() -> String {
    row("peer", "frame_start", 1, 25, 5)
        + &row("peer", "frame_end", 1, 55, 15)
        + &row("peer", "frame_start", 2, 58, 16)
        + &row("peer", "frame_end", 2, 65, 20)
}

fn offers() -> Vec<Offer> {
    [35, 62]
        .into_iter()
        .enumerate()
        .map(|(i, time)| Offer {
            connection_epoch: 1,
            content_grant_epoch: 2,
            output: 3,
            candidate_generation: i as u64 + 1,
            submitted_monotonic_usec: time,
            outcome: "presented".into(),
        })
        .collect()
}

fn journal() -> String {
    let mut rows = vec![serde_json::json!({"schema":2,"event":"start","frames":2,"wire":"files"})];
    for (index, (begin, accepted, submitted, terminal)) in
        [(27, 28, 35, 50), (60, 61, 62, 64)].into_iter().enumerate()
    {
        let generation = index + 1;
        rows.extend([
            serde_json::json!({"schema":2,"event":"upload_offer","upload":{"connection_epoch":1,"content_grant_epoch":2,"output":3,"candidate_generation":generation,"resource_id":generation,"resource_generation":1,"width":1920,"height":24,"total_bytes":184320,"begin_usec":begin}}),
            serde_json::json!({"schema":2,"event":"upload_accepted","resource_id":generation,"resource_generation":1,"accepted_usec":accepted}),
            serde_json::json!({"schema":2,"event":"permit","generation":generation,"demand_usec":accepted,"received_usec":accepted}),
            serde_json::json!({"schema":2,"event":"offer","connection_epoch":1,"content_grant_epoch":2,"output":3,"candidate_generation":generation,"submitted_monotonic_usec":submitted}),
            serde_json::json!({"schema":2,"event":"terminal","connection_epoch":1,"content_grant_epoch":2,"output":3,"candidate_generation":generation,"outcome":"presented","kind":2,"reason":0,"observed_monotonic_usec":terminal}),
        ]);
    }
    rows.push(serde_json::json!({"schema":2,"event":"end","offered":2,"completed":2}));
    rows.iter().map(|r| r.to_string() + "\n").collect()
}

#[test]
fn complete_upload_bytes_normalize_whole_workload_with_outward_rounding() {
    use xtask::shell_timing_cost::measure_journal;
    let report = measure_journal(&host(), &peer(), &journal(), 2).unwrap();
    let normalized = report.normalized.unwrap();
    assert_eq!(normalized.uploaded_bytes, 368640);
    assert_eq!(normalized.presentation_bytes, 368640);
    assert_eq!(normalized.transfer_probe_bytes, 0);
    assert_eq!(normalized.frame_count, 2);
    assert_eq!(
        (
            normalized
                .allocation_operations_per_frame
                .as_ref()
                .unwrap()
                .lower,
            normalized
                .allocation_operations_per_frame
                .as_ref()
                .unwrap()
                .upper
        ),
        (22, 33)
    );
    assert_eq!(
        (
            normalized.cpu_ns_per_uploaded_mib.lower,
            normalized.cpu_ns_per_uploaded_mib.upper
        ),
        (12800, 18489)
    );
    assert!(report.cpu_scope.contains("including_foreign_code"));
    assert!(
        report
            .allocation_scope
            .contains("excluding_foreign_allocators")
    );
    // A valid journal is insufficient if its upload started before the counter window.
    assert!(
        measure_journal(
            &host(),
            &peer(),
            &journal().replace("\"begin_usec\":27", "\"begin_usec\":24"),
            2
        )
        .is_err()
    );
    assert!(
        measure_journal(
            &host(),
            &peer(),
            &journal().replace("\"total_bytes\":184320", "\"total_bytes\":184321"),
            2
        )
        .is_err()
    );
    assert!(measure_journal(&host(), &peer(), &journal(), 1).is_err());
    assert!(
        measure_journal(
            &host(),
            &peer(),
            &journal().replace(
                "\"observed_monotonic_usec\":64",
                "\"observed_monotonic_usec\":67"
            ),
            2
        )
        .is_err()
    );
}

#[test]
fn outer_and_inner_samples_produce_conservative_bounds_not_interpolated_counts() {
    let report = measure(&host(), &peer(), &offers()).unwrap();
    let first = &report.frames[0].cost;
    assert_eq!(
        (
            first.allocation_operations.as_ref().unwrap().lower,
            first.allocation_operations.as_ref().unwrap().upper
        ),
        (30, 50)
    );
    assert_eq!(
        (first.process_cpu_ns.lower, first.process_cpu_ns.upper),
        (3000, 5000)
    );
    let second = &report.frames[1].cost;
    assert_eq!(
        (
            second.allocation_operations.as_ref().unwrap().lower,
            second.allocation_operations.as_ref().unwrap().upper
        ),
        (4, 24)
    );
    assert_eq!(
        (
            report
                .whole_workload
                .allocation_operations
                .as_ref()
                .unwrap()
                .lower,
            report
                .whole_workload
                .allocation_operations
                .as_ref()
                .unwrap()
                .upper
        ),
        (45, 65)
    );
}

#[test]
fn missing_lifecycle_records_clock_regressions_and_counter_resets_fail() {
    for bad in [
        host().replace("stage=end", "stage=exhausted"),
        host().replace("sequence=3 ", "sequence=2 "),
        host().replace("before_usec=40", "before_usec=0"),
        host().replace("cpu_ns=3000", "cpu_ns=0"),
        host().replace("allocations=30", "allocations=0"),
        host().replace("before_usec=40", "before_usec=19"),
        host().replace(
            "pid=10 stage=sample sequence=3",
            "pid=11 stage=sample sequence=3",
        ),
        host().replace("schema=1", "schema=1 unknown=1"),
        host().replace("schema=1", "schema=1 schema=1"),
        host().trim_end().into(),
        host().lines().take(6).collect::<Vec<_>>().join("\n") + "\n",
    ] {
        assert!(measure(&bad, &peer(), &offers()).is_err(), "accepted {bad}");
    }
    for bad in [
        peer().replace("frame_end", "sample"),
        peer().replace("sequence=2", "sequence=3"),
        peer().replace("pid=20", "pid=10"),
        peer().lines().take(3).collect::<Vec<_>>().join("\n") + "\n",
    ] {
        assert!(measure(&host(), &bad, &offers()).is_err());
    }
}

#[test]
fn offers_and_entire_counter_windows_must_match() {
    let mut offered = offers();
    offered[0].submitted_monotonic_usec = 24;
    assert!(measure(&host(), &peer(), &offered).is_err());
    let late = peer().replace(
        "before_usec=65 after_usec=66",
        "before_usec=80 after_usec=81",
    );
    assert!(measure(&host(), &late, &offers()).is_err());
    let early = peer().replace("before_usec=25 after_usec=26", "before_usec=5 after_usec=6");
    assert!(measure(&host(), &early, &offers()).is_err());
    assert!(measure(&host(), &peer(), &offers()[..1]).is_err());
}

#[test]
fn instrumentation_cannot_silently_disappear_or_enter_a_latency_only_run() {
    use xtask::shell_timing_cost::for_mode;
    let capture = host() + &peer();
    assert!(
        for_mode("rust-global", &capture, &offers())
            .unwrap()
            .is_some()
    );
    assert!(for_mode("none", &capture, &offers()).is_err());
    assert!(for_mode("rust-global", "ordinary capture\n", &offers()).is_err());
    assert!(
        for_mode("none", "ordinary capture\n", &offers())
            .unwrap()
            .is_none()
    );
    assert!(for_mode("unknown", &capture, &offers()).is_err());
}
