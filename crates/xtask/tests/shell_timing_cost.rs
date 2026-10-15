use xtask::{shell_timing::Offer, shell_timing_cost::measure};

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

#[test]
fn outer_and_inner_samples_produce_conservative_bounds_not_interpolated_counts() {
    let report = measure(&host(), &peer(), &offers()).unwrap();
    let first = &report.frames[0].cost;
    assert_eq!(
        (
            first.allocation_operations.lower,
            first.allocation_operations.upper
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
            second.allocation_operations.lower,
            second.allocation_operations.upper
        ),
        (4, 24)
    );
    assert_eq!(
        (
            report.whole_workload.allocation_operations.lower,
            report.whole_workload.allocation_operations.upper
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
