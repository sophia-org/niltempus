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
    assert_eq!(sealed.source, ImageSource::Isb);
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

fn schema_three(log: &str) -> String {
    log.lines()
        .map(|line| {
            let count: u64 = line
                .split_whitespace()
                .find_map(|w| w.strip_prefix("cpu_ns="))
                .unwrap()
                .parse::<u64>()
                .unwrap()
                / 100;
            let line = line
                .replace("schema=1", "schema=3 mode=rust-global")
                .replace("schema=2", "schema=3");
            format!("{line} minor_faults={count} major_faults={}\n", count / 10)
        })
        .collect()
}

#[test]
fn schema_three_preserves_legacy_costs_and_splits_process_diagnostics() {
    use xtask::shell_timing_cost::measure_journal;
    let old = measure_journal(&host(), &peer(), &journal(), 2).unwrap();
    let new = measure_journal(
        &schema_three(&host()),
        &schema_three(&peer()),
        &journal(),
        2,
    )
    .unwrap();
    let mut new_json = serde_json::to_value(&new).unwrap();
    assert!(
        new_json
            .as_object_mut()
            .unwrap()
            .remove("diagnostics")
            .is_some()
    );
    assert_eq!(new_json, serde_json::to_value(old).unwrap());
    let d = new.diagnostics.unwrap();
    assert_eq!(d.counter_schema, 3);
    assert_eq!(
        (d.session.minor_faults.lower, d.session.minor_faults.upper),
        (30, 50)
    );
    assert_eq!(
        (d.peer.minor_faults.lower, d.peer.minor_faults.upper),
        (15, 15)
    );
    assert_eq!(
        (d.combined.minor_faults.lower, d.combined.minor_faults.upper),
        (45, 65)
    );
    assert_eq!(
        (d.combined.major_faults.lower, d.combined.major_faults.upper),
        (5, 7)
    );
    assert_eq!(
        d.combined.cpu_ns.lower,
        d.session.cpu_ns.lower + d.peer.cpu_ns.lower
    );
    assert_eq!(
        d.combined.cpu_ns.upper,
        d.session.cpu_ns.upper + d.peer.cpu_ns.upper
    );
    let n = d.combined.normalized.unwrap();
    assert_eq!(
        (
            n.minor_faults_per_frame.lower,
            n.minor_faults_per_frame.upper
        ),
        (22, 33)
    );
    assert_eq!(
        (
            n.major_faults_per_frame.lower,
            n.major_faults_per_frame.upper
        ),
        (2, 4)
    );
}

#[test]
fn schema_three_cpu_only_has_faults_but_no_allocation_claim() {
    use xtask::shell_timing_cost::measure_journal;
    let report = measure_journal(
        &schema_three(&cpu_only(&host())),
        &schema_three(&cpu_only(&peer())),
        &journal(),
        2,
    )
    .unwrap();
    for process in [
        &report.diagnostics.as_ref().unwrap().session,
        &report.diagnostics.as_ref().unwrap().peer,
        &report.diagnostics.as_ref().unwrap().combined,
    ] {
        assert!(process.allocation_operations.is_none());
        assert!(
            process
                .normalized
                .as_ref()
                .unwrap()
                .allocation_operations_per_frame
                .is_none()
        );
    }
}

#[test]
fn schema_three_refuses_incomplete_mixed_or_regressing_fault_evidence() {
    let h = schema_three(&host());
    let p = schema_three(&peer());
    for bad in [
        h.replace(" minor_faults=0", ""),
        h.replace(" major_faults=0", ""),
        h.replacen("minor_faults=0", "minor_faults=0 minor_faults=0", 1),
        h.replace("minor_faults=20", "minor_faults=9"),
        h.replace("major_faults=2", "major_faults=0"),
        h.replace("minor_faults=20", "minor_faults=-1"),
        h.replace("major_faults=2", "major_faults=18446744073709551616"),
        h.replacen("schema=3", "schema=2", 1),
    ] {
        assert!(measure(&bad, &p, &offers()).is_err(), "accepted {bad}");
    }
    assert!(measure(&h, &peer(), &offers()).is_err());
    assert!(measure(&host(), &p, &offers()).is_err());
}

fn retained_journal() -> String {
    // Two frames against the shared host()/peer() counter fixtures: the
    // pool (2 tiles of 960x24) is accepted before frame 1, both candidates
    // reference it, and it stays live at End (shape A, no releases).
    let mut rows = vec![
        serde_json::json!({"schema":3,"event":"start","frames":2,"wire":"files","source":"sealed","probe_bytes":0,"workload":"retained","retained_resources":2}),
    ];
    for (id, begin, accepted) in [(2, 20, 22), (3, 23, 24)] {
        rows.push(serde_json::json!({"schema":3,"event":"upload_offer","upload":{"connection_epoch":1,"content_grant_epoch":2,"output":3,"candidate_generation":0,"resource_id":id,"resource_generation":1,"width":960,"height":24,"total_bytes":92160,"begin_usec":begin}}));
        rows.push(serde_json::json!({"schema":3,"event":"upload_accepted","resource_id":id,"resource_generation":1,"accepted_usec":accepted}));
    }
    for (f, demand, received, submitted, terminal) in [(1, 30, 31, 35, 50), (2, 60, 61, 62, 64)] {
        rows.push(serde_json::json!({"schema":3,"event":"permit","generation":f,"demand_usec":demand,"received_usec":received}));
        let placements: Vec<_> = (0..2)
            .map(|i| serde_json::json!({"resource_id":i+2,"resource_generation":1,"destination_x_px":(i+f-1)%2*960,"destination_y_px":0}))
            .collect();
        rows.push(serde_json::json!({"schema":3,"event":"candidate_placements","candidate_generation":f,"placements":placements}));
        rows.push(serde_json::json!({"schema":3,"event":"offer","connection_epoch":1,"content_grant_epoch":2,"output":3,"candidate_generation":f,"submitted_monotonic_usec":submitted}));
        rows.push(serde_json::json!({"schema":3,"event":"terminal","connection_epoch":1,"content_grant_epoch":2,"output":3,"candidate_generation":f,"outcome":"presented","kind":2,"reason":0,"observed_monotonic_usec":terminal}));
    }
    rows.push(serde_json::json!({"schema":3,"event":"end","offered":2,"completed":2}));
    rows.iter().map(|r| r.to_string() + "\n").collect()
}

#[test]
fn retained_cost_normalizes_by_actual_pool_uploads_not_frame_work() {
    use xtask::shell_timing_cost::measure_journal;
    let retained = measure_journal(&host(), &peer(), &retained_journal(), 2)
        .unwrap()
        .normalized
        .unwrap();
    assert_eq!(
        retained.workload,
        xtask::shell_timing::WorkloadMode::Retained
    );
    assert_eq!(retained.retained_resources, Some(2));
    assert_eq!(retained.frame_count, 2);
    // Two 960x24 tiles uploaded once: 184320 bytes total, while the fresh
    // two-frame journal uploads the same bytes per frame (368640).
    assert_eq!(retained.presentation_bytes, 184320);
    assert_eq!(retained.uploaded_bytes, 184320);
    assert_eq!(retained.transferred_bytes, 184320);
    let fresh_report = measure_journal(&host(), &peer(), &journal(), 2).unwrap();
    // Fresh reports keep their legacy serialized shape: no workload field.
    let fresh_json = serde_json::to_value(&fresh_report).unwrap();
    assert!(fresh_json["normalized"].get("workload").is_none());
    let fresh = fresh_report.normalized.unwrap();
    assert_eq!(fresh.workload, xtask::shell_timing::WorkloadMode::Fresh);
    assert_eq!(fresh.uploaded_bytes, 368640);
    assert_eq!(
        (
            retained.cpu_ns_per_frame.lower,
            retained.cpu_ns_per_frame.upper
        ),
        (fresh.cpu_ns_per_frame.lower, fresh.cpu_ns_per_frame.upper)
    );
    assert!(
        retained.cpu_ns_per_uploaded_mib.lower > fresh.cpu_ns_per_uploaded_mib.lower,
        "per-MiB must divide by actual uploaded bytes"
    );
    // A pool accepted after the first frame interval cannot be normalized.
    assert!(
        measure_journal(
            &host(),
            &peer(),
            &retained_journal().replace("\"accepted_usec\":22", "\"accepted_usec\":99"),
            2
        )
        .is_err()
    );
}

#[test]
fn schema_three_profile_remains_diagnostic_only() {
    use xtask::shell_timing_cost::for_mode;
    let capture = (schema_three(&host()) + &schema_three(&peer()))
        .replace("mode=rust-global", "mode=profile");
    assert!(
        for_mode("profile", &capture, &offers())
            .unwrap()
            .unwrap()
            .diagnostics
            .is_some()
    );
    for mode in ["cpu-only", "rust-global", "none"] {
        assert!(for_mode(mode, &capture, &offers()).is_err());
    }
}
