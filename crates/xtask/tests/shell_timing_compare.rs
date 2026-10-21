//! Paired gate comparisons over synthetic arms. No devices, no campaign files.
use xtask::shell_timing::{ImageSource, Window, latency_window};
use xtask::shell_timing_compare::*;
use xtask::shell_timing_cost::{Bounds, Cost, Normalized, Report};

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

const WINDOW: Window = Window {
    warmup: 1,
    measured: 4,
};

fn quick() -> Timing {
    Timing {
        bar: |_| 100,
        probe: None,
        permit: |_| 10,
        present: |_| 1_000,
    }
}

fn latency(
    label: &str,
    wire: &str,
    source: ImageSource,
    workload: Workload,
    timing: Timing,
) -> LatencyArm {
    shaped(label, wire, source, workload, timing, &|journal| journal)
}

/// A valid arm whose journal text is edited before it is read.
fn shaped(
    label: &str,
    wire: &str,
    source: ImageSource,
    workload: Workload,
    timing: Timing,
    edit: &dyn Fn(String) -> String,
) -> LatencyArm {
    let timing = Timing {
        probe: if workload == Workload::Bulk {
            Some(timing.probe.unwrap_or(|_| 20_000))
        } else {
            None
        },
        ..timing
    };
    let (host, journal) = arm(5, &timing, wire);
    let mut journal = edit(journal);
    if source == ImageSource::Isb {
        journal = journal.replacen(
            "\"event\":\"start\"",
            "\"event\":\"start\",\"source\":\"sealed\"",
            1,
        );
    }
    LatencyArm {
        label: label.into(),
        wire: wire.into(),
        source,
        workload,
        latency: latency_window(&host, &journal, 120000, WINDOW).unwrap(),
    }
}

fn bounds(lower: u64, upper: u64) -> Bounds {
    Bounds { lower, upper }
}

fn cost(
    label: &str,
    wire: &str,
    source: ImageSource,
    workload: Workload,
    instrumentation: &'static str,
    value: (u64, u64),
) -> CostArm {
    let probe = if workload == Workload::Bulk {
        1000 * 4194304
    } else {
        0
    };
    let allocations = (instrumentation == "rust-global").then(|| bounds(value.0, value.1));
    CostArm {
        label: label.into(),
        wire: wire.into(),
        source,
        workload,
        report: Report {
            diagnostics: None,
            instrumentation,
            scope: "synthetic",
            cpu_scope: "synthetic",
            allocation_scope: "synthetic",
            frames: Vec::new(),
            whole_workload: Cost {
                allocation_operations: None,
                process_cpu_ns: bounds(1, 2),
                outer_start_usec: 1,
                outer_end_usec: 2,
            },
            normalized: Some(Normalized {
                frame_count: 1000,
                workload: xtask::shell_timing::WorkloadMode::Fresh,
                retained_resources: None,
                source,
                transferred_bytes: 1000 * 184320 + probe,
                uploaded_bytes: 1000 * 184320 + probe,
                presentation_bytes: 1000 * 184320,
                transfer_probe_bytes: probe,
                cpu_ns_per_uploaded_mib: bounds(value.0, value.1),
                cpu_ns_per_frame: bounds(1, 2),
                allocation_operations_per_frame: allocations,
            }),
        },
    }
}

const ROLES: [(&str, &str, ImageSource); 3] = [
    ("a", "ipc", ImageSource::Bytes),
    ("b", "files", ImageSource::Bytes),
    ("c", "files", ImageSource::Isb),
];

/// One complete pair. `slow` adds latency to the given role's arms and
/// `costly` sets every cost value of the given role.
fn pair(index: usize, slow: Option<(&str, u64)>, costly: Option<(&str, (u64, u64))>) -> PairInput {
    let mut latency_arms = Vec::new();
    let mut cost_arms = Vec::new();
    for workload in [Workload::Panel, Workload::Bulk] {
        for (role, wire, source) in ROLES {
            let extra = slow.filter(|(r, _)| *r == role).map_or(0, |(_, n)| n);
            let timing = match extra {
                0 => quick(),
                1001 => Timing {
                    present: |_| 2_001,
                    ..quick()
                },
                2001 => Timing {
                    present: |_| 3_001,
                    ..quick()
                },
                _ => unreachable!(),
            };
            latency_arms.push(latency(
                &format!("p{index}-{role}-{workload:?}-latency"),
                wire,
                source,
                workload,
                timing,
            ));
            for instrumentation in ["cpu-only", "rust-global"] {
                let value = costly
                    .filter(|(r, _)| *r == role)
                    .map_or((100, 100), |(_, v)| v);
                cost_arms.push(cost(
                    &format!("p{index}-{role}-{workload:?}-{instrumentation}"),
                    wire,
                    source,
                    workload,
                    instrumentation,
                    value,
                ));
            }
        }
    }
    let order = latency_arms
        .iter()
        .map(|a| a.label.clone())
        .chain(cost_arms.iter().map(|a| a.label.clone()))
        .collect();
    PairInput {
        index,
        order,
        latency: latency_arms,
        cost: cost_arms,
    }
}

#[test]
fn conservative_cost_rule_boundaries() {
    let v = |lower, upper| Value { lower, upper };
    assert_eq!(cost_outcome(v(90, 110), v(100, 105)), Outcome::Pass);
    assert_eq!(cost_outcome(v(90, 111), v(100, 105)), Outcome::Inconclusive);
    assert_eq!(cost_outcome(v(116, 120), v(100, 105)), Outcome::Fail);
    assert_eq!(
        cost_outcome(v(115, 120), v(100, 105)),
        Outcome::Inconclusive
    );
}

#[test]
fn latency_gates_use_per_pair_points_and_label_bar_measures_in_bulk_arms() {
    let a = latency("a", "ipc", ImageSource::Bytes, Workload::Bulk, quick());
    let exact = latency(
        "c",
        "files",
        ImageSource::Isb,
        Workload::Bulk,
        Timing {
            present: |_| 2_000,
            probe: Some(|_| 22_000),
            ..quick()
        },
    );
    let results = compare_latency(&exact, &a).unwrap();
    let gates: Vec<_> = results
        .iter()
        .map(|r| (r.gate, r.measure, r.outcome))
        .collect();
    assert!(gates.contains(&(
        "present_p95",
        "bulk_arm_candidate_to_native_presented",
        Outcome::Pass
    )));
    assert!(gates.contains(&(
        "panel_accept_p95",
        "bulk_arm_bar_upload_to_accepted",
        Outcome::Pass
    )));
    assert!(gates.contains(&("bulk_accept_p95", "probe_upload_to_accepted", Outcome::Pass)));
    let over = latency(
        "c",
        "files",
        ImageSource::Isb,
        Workload::Bulk,
        Timing {
            present: |_| 2_001,
            probe: Some(|_| 22_001),
            ..quick()
        },
    );
    let results = compare_latency(&over, &a).unwrap();
    for gate in ["present_p95", "bulk_accept_p95"] {
        assert_eq!(
            results.iter().find(|r| r.gate == gate).unwrap().outcome,
            Outcome::Fail
        );
    }
    assert_eq!(
        results
            .iter()
            .find(|r| r.gate == "present_p99")
            .unwrap()
            .outcome,
        Outcome::Pass
    );
    // Baselines must be IPC bytes, and workloads must match.
    assert!(compare_latency(&a, &over).is_err());
    let panel = latency("c", "files", ImageSource::Isb, Workload::Panel, quick());
    assert!(compare_latency(&panel, &a).is_err());
}

#[test]
fn absolute_gates_include_warmup_transfer_bound_and_permit_limit() {
    let late = Timing {
        bar: |g| if g == 1 { 2_000_000 } else { 100 },
        ..quick()
    };
    let arm = latency("a", "ipc", ImageSource::Bytes, Workload::Panel, late);
    let results = absolute_gates(&arm).unwrap();
    assert_eq!(
        results
            .iter()
            .find(|r| r.gate == "transfer_bound")
            .unwrap()
            .outcome,
        Outcome::Fail
    );
    assert_eq!(
        results
            .iter()
            .find(|r| r.gate == "permit_p99")
            .unwrap()
            .outcome,
        Outcome::Pass
    );
    let slow_permit = Timing {
        permit: |_| 125_000,
        ..quick()
    };
    let arm = latency("a", "ipc", ImageSource::Bytes, Workload::Panel, slow_permit);
    let results = absolute_gates(&arm).unwrap();
    assert_eq!(
        results
            .iter()
            .find(|r| r.gate == "permit_p99")
            .unwrap()
            .outcome,
        Outcome::Fail
    );
}

#[test]
fn cost_and_source_comparisons_refuse_mismatched_identity() {
    let a = cost(
        "a",
        "ipc",
        ImageSource::Bytes,
        Workload::Panel,
        "cpu-only",
        (100, 100),
    );
    let c = cost(
        "c",
        "files",
        ImageSource::Isb,
        Workload::Panel,
        "cpu-only",
        (110, 110),
    );
    assert_eq!(compare_cost(&c, &a).unwrap().outcome, Outcome::Pass);
    let alloc = cost(
        "c",
        "files",
        ImageSource::Isb,
        Workload::Panel,
        "rust-global",
        (110, 110),
    );
    assert!(compare_cost(&alloc, &a).is_err());
    let bulk = cost(
        "c",
        "files",
        ImageSource::Isb,
        Workload::Bulk,
        "cpu-only",
        (110, 110),
    );
    assert!(compare_cost(&bulk, &a).is_err());
    let mut fewer = cost(
        "c",
        "files",
        ImageSource::Isb,
        Workload::Panel,
        "cpu-only",
        (110, 110),
    );
    fewer.report.normalized.as_mut().unwrap().presentation_bytes -= 4;
    assert!(compare_cost(&fewer, &a).is_err());
    let mut mislabelled = cost(
        "c",
        "files",
        ImageSource::Isb,
        Workload::Panel,
        "cpu-only",
        (1, 1),
    );
    mislabelled.source = ImageSource::Bytes;
    assert!(compare_cost(&mislabelled, &a).is_err());
    assert!(compare_cost(&a, &c).is_err());
    // Retained arms are CPU/progress qualification only: no B9 comparison
    // against fresh arms (or each other) until a comparison is designed.
    let mut retained = cost(
        "c",
        "files",
        ImageSource::Isb,
        Workload::Panel,
        "cpu-only",
        (110, 110),
    );
    let normalized = retained.report.normalized.as_mut().unwrap();
    normalized.workload = xtask::shell_timing::WorkloadMode::Retained;
    normalized.retained_resources = Some(4);
    assert!(compare_cost(&retained, &a).is_err());
    assert!(compare_source_cost(&retained, &c).is_err());
    let b = cost(
        "b",
        "files",
        ImageSource::Bytes,
        Workload::Panel,
        "cpu-only",
        (100, 100),
    );
    let entry = compare_source_cost(&c, &b).unwrap();
    assert_eq!(
        (entry.outcome, entry.difference_lower),
        ("reported_only", 10)
    );
    assert!(compare_source_cost(&b, &c).is_err());
    assert!(compare_source_cost(&c, &a).is_err());
    let b_alloc = cost(
        "b",
        "files",
        ImageSource::Bytes,
        Workload::Panel,
        "rust-global",
        (1, 1),
    );
    assert!(compare_source_cost(&c, &b_alloc).is_err());
    let bl = latency("b", "files", ImageSource::Bytes, Workload::Panel, quick());
    let cl = latency("c", "files", ImageSource::Isb, Workload::Panel, quick());
    assert!(
        compare_source_latency(&cl, &bl)
            .unwrap()
            .iter()
            .all(|e| e.outcome == "reported_only")
    );
    let cb = latency("c", "files", ImageSource::Isb, Workload::Bulk, quick());
    assert!(compare_source_latency(&cb, &bl).is_err());
}

#[test]
fn condition_requires_three_complete_unique_pairs() {
    let report = condition_report(&[
        pair(1, None, None),
        pair(2, None, None),
        pair(3, None, None),
    ])
    .unwrap();
    assert_eq!(report.verdict, Outcome::Pass);
    assert!(report.scope.contains("not a B9 verdict"));
    assert_eq!(
        report.pairs[0]
            .sealed_vs_ipc
            .iter()
            .filter(|r| r.gate.starts_with("cost_"))
            .count(),
        4
    );
    assert!(condition_report(&[pair(1, None, None), pair(2, None, None)]).is_err());
    assert!(condition_report(&[]).is_err());
    assert!(
        condition_report(&[
            pair(1, None, None),
            pair(2, None, None),
            pair(2, None, None)
        ])
        .is_err()
    );
    let mut duplicate = pair(3, None, None);
    duplicate.latency[0].label = "p1-a-Panel-latency".into();
    duplicate.order[0] = "p1-a-Panel-latency".into();
    assert!(condition_report(&[pair(1, None, None), pair(2, None, None), duplicate]).is_err());
    let mut partial = pair(3, None, None);
    partial.cost.pop();
    partial.order.pop();
    assert!(condition_report(&[pair(1, None, None), pair(2, None, None), partial]).is_err());
    let mut unordered = pair(3, None, None);
    unordered.order.pop();
    assert!(condition_report(&[pair(1, None, None), pair(2, None, None), unordered]).is_err());
}

fn without_retired_bytes(mut pair: PairInput) -> PairInput {
    pair.latency
        .retain(|a| a.wire != "files" || a.source != ImageSource::Bytes);
    pair.cost
        .retain(|a| a.wire != "files" || a.source != ImageSource::Bytes);
    let labels: std::collections::BTreeSet<_> = pair
        .latency
        .iter()
        .map(|a| a.label.clone())
        .chain(pair.cost.iter().map(|a| a.label.clone()))
        .collect();
    pair.order.retain(|id| labels.contains(id));
    pair
}

#[test]
fn isb_only_condition_keeps_every_gate_and_refuses_missing_or_extra_arms() {
    let pairs = [1, 2, 3].map(|i| without_retired_bytes(pair(i, None, None)));
    let report = isb_condition_report(&pairs).unwrap();
    assert_eq!(report.schema, 2);
    assert_eq!(report.verdict, Outcome::Pass);
    assert_eq!(report.pooled.len(), 4);
    assert_eq!(report.bytes_vs_ipc_outcome, Outcome::Inconclusive);
    assert!(report.bytes_vs_ipc_scope.starts_with("not run"));
    for row in &report.pairs {
        assert_eq!(
            row.sealed_vs_ipc
                .iter()
                .filter(|g| g.gate.starts_with("cost_"))
                .count(),
            4
        );
        assert!(row.bytes_vs_ipc.is_empty());
        assert!(row.sealed_vs_bytes.is_empty());
    }
    assert!(isb_condition_report(&pairs[..2]).is_err());
    assert!(condition_report(&pairs).is_err());
    assert!(
        isb_condition_report(&[
            pair(1, None, None),
            pair(2, None, None),
            pair(3, None, None)
        ])
        .is_err()
    );
    let failed = [
        without_retired_bytes(pair(1, None, None)),
        without_retired_bytes(pair(2, Some(("c", 1001)), None)),
        without_retired_bytes(pair(3, None, None)),
    ];
    assert_eq!(
        isb_condition_report(&failed).unwrap().verdict,
        Outcome::Fail
    );
    let uncertain = [
        without_retired_bytes(pair(1, None, Some(("c", (100, 111))))),
        without_retired_bytes(pair(2, None, None)),
        without_retired_bytes(pair(3, None, None)),
    ];
    assert_eq!(
        isb_condition_report(&uncertain).unwrap().verdict,
        Outcome::Inconclusive
    );
}

#[test]
fn one_failing_pair_fails_the_condition_and_bytes_results_stay_separate() {
    // Pair 2's sealed arms present 1001 us later: a p95 failure in one pair.
    let report = condition_report(&[
        pair(1, None, None),
        pair(2, Some(("c", 1001)), None),
        pair(3, None, None),
    ])
    .unwrap();
    assert_eq!(report.verdict, Outcome::Fail);
    assert_eq!(report.pairs[1].sealed_vs_ipc_outcome, Outcome::Fail);
    assert_eq!(report.pairs[0].sealed_vs_ipc_outcome, Outcome::Pass);
    // Inconclusive cost in one pair makes the condition inconclusive.
    let report = condition_report(&[
        pair(1, None, Some(("c", (100, 111)))),
        pair(2, None, None),
        pair(3, None, None),
    ])
    .unwrap();
    assert_eq!(report.verdict, Outcome::Inconclusive);
    // A failing bytes arm is reported but does not decide the C against A verdict.
    let report = condition_report(&[
        pair(1, Some(("b", 2001)), Some(("b", (200, 200)))),
        pair(2, None, None),
        pair(3, None, None),
    ])
    .unwrap();
    assert_eq!(report.verdict, Outcome::Pass);
    assert_eq!(report.bytes_vs_ipc_outcome, Outcome::Fail);
    assert!(
        report
            .bytes_vs_ipc_scope
            .contains("not the condition verdict")
    );
    assert_eq!(report.pooled.len(), 6);
}

const BAR: &str = "\"width\":1920,\"height\":24,\"total_bytes\":184320";
const NARROW: &str = "\"width\":64,\"height\":24,\"total_bytes\":6144";

#[test]
fn latency_comparisons_refuse_different_upload_shapes() {
    let narrow = |journal: String| journal.replace(BAR, NARROW);
    // Only the first upload, a warmup frame, differs.
    let first = |journal: String| journal.replacen(BAR, NARROW, 1);
    for workload in [Workload::Panel, Workload::Bulk] {
        let a = latency("a", "ipc", ImageSource::Bytes, workload, quick());
        let b = latency("b", "files", ImageSource::Bytes, workload, quick());
        let c = latency("c", "files", ImageSource::Isb, workload, quick());
        assert!(compare_latency(&c, &a).is_ok());
        assert!(compare_source_latency(&c, &b).is_ok());
        for edit in [&narrow as &dyn Fn(String) -> String, &first] {
            let c2 = shaped("c", "files", ImageSource::Isb, workload, quick(), edit);
            assert_eq!(c2.latency.measured_samples, c.latency.measured_samples);
            assert!(compare_latency(&c2, &a).is_err());
            assert!(compare_source_latency(&c2, &b).is_err());
        }
    }
    // Same dimensions, different bytes in one generation.
    let mut c = latency("c", "files", ImageSource::Isb, Workload::Panel, quick());
    c.latency.journal_metrics.uploads[3].upload.total_bytes += 4;
    let a = latency("a", "ipc", ImageSource::Bytes, Workload::Panel, quick());
    let b = latency("b", "files", ImageSource::Bytes, Workload::Panel, quick());
    assert!(compare_latency(&c, &a).is_err());
    assert!(compare_source_latency(&c, &b).is_err());
    // Epochs and timestamps may differ.
    let mut c = latency("c", "files", ImageSource::Isb, Workload::Panel, quick());
    for upload in &mut c.latency.journal_metrics.uploads {
        upload.upload.connection_epoch += 7;
        upload.upload.begin_usec += 11;
        upload.accepted_usec += 11;
    }
    assert!(compare_latency(&c, &a).is_ok());
    assert!(compare_source_latency(&c, &b).is_ok());
}

#[test]
fn bulk_arms_require_the_exact_four_mib_probe() {
    let a = latency("a", "ipc", ImageSource::Bytes, Workload::Bulk, quick());
    let mut c = latency("c", "files", ImageSource::Isb, Workload::Bulk, quick());
    assert!(compare_latency(&c, &a).is_ok());
    c.latency.probe_bytes = 4096;
    assert!(compare_latency(&c, &a).is_err());
    let mut a2 = latency("a", "ipc", ImageSource::Bytes, Workload::Bulk, quick());
    a2.latency.probe_bytes = 4096;
    c.latency.probe_bytes = 4096;
    assert!(compare_latency(&c, &a2).is_err());
    assert!(absolute_gates(&a2).is_err());
    let a = cost(
        "a",
        "ipc",
        ImageSource::Bytes,
        Workload::Bulk,
        "cpu-only",
        (100, 100),
    );
    let mut c = cost(
        "c",
        "files",
        ImageSource::Isb,
        Workload::Bulk,
        "cpu-only",
        (100, 100),
    );
    assert!(compare_cost(&c, &a).is_ok());
    for probe in [1, 999 * 4194304, 1000 * 4194304 - 1] {
        let mut a2 = cost(
            "a",
            "ipc",
            ImageSource::Bytes,
            Workload::Bulk,
            "cpu-only",
            (100, 100),
        );
        a2.report.normalized.as_mut().unwrap().transfer_probe_bytes = probe;
        c.report.normalized.as_mut().unwrap().transfer_probe_bytes = probe;
        assert!(compare_cost(&c, &a2).is_err());
    }
}

#[test]
fn every_pair_runs_the_same_workload() {
    // Each change applies to all three roles, so every pair still matches
    // internally and only the cross-pair check can refuse it.
    let presentation = |pair: &mut PairInput| {
        for arm in pair
            .cost
            .iter_mut()
            .filter(|a| a.workload == Workload::Panel && a.report.instrumentation == "cpu-only")
        {
            arm.report.normalized.as_mut().unwrap().presentation_bytes -= 4;
        }
    };
    let frames =
        |pair: &mut PairInput| {
            for arm in pair.cost.iter_mut().filter(|a| {
                a.workload == Workload::Bulk && a.report.instrumentation == "rust-global"
            }) {
                let normalized = arm.report.normalized.as_mut().unwrap();
                normalized.frame_count = 999;
                normalized.transfer_probe_bytes = 999 * 4194304;
            }
        };
    let shape = |pair: &mut PairInput| {
        for arm in pair
            .latency
            .iter_mut()
            .filter(|a| a.workload == Workload::Bulk)
        {
            arm.latency.journal_metrics.uploads[1].upload.width = 64;
            arm.latency.journal_metrics.uploads[1].upload.total_bytes = 6144;
        }
    };
    for change in [&presentation as &dyn Fn(&mut PairInput), &frames, &shape] {
        let mut odd = pair(2, None, None);
        change(&mut odd);
        assert!(condition_report(&[pair(1, None, None), odd, pair(3, None, None)]).is_err());
        let mut all = [
            pair(1, None, None),
            pair(2, None, None),
            pair(3, None, None),
        ];
        all.iter_mut().for_each(change);
        assert_eq!(condition_report(&all).unwrap().verdict, Outcome::Pass);
    }
}
