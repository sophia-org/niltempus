use xtask::shell_timing::{collect, collect_metrics};

#[test]
fn bursty_schema_preserves_upload_custody_without_claiming_legacy_acceptance() {
    let old = upload_journal();
    let bursty = old.replace("\"schema\":2", "\"schema\":4").replacen(
        "\"event\":\"start\"",
        "\"event\":\"start\",\"source\":\"sealed\",\"interval_msec\":250",
        1,
    );
    let metrics = collect_metrics(&bursty, 1).unwrap();
    assert_eq!(metrics.schema, 4);
    assert_eq!(metrics.interval_msec, Some(250));
    assert_eq!(metrics.uploads.len(), 1);
    let joined = xtask::shell_timing::join(&native_host(140), &metrics.offers, 60000, 1).unwrap();
    assert_eq!(joined.len(), 1);
    assert!(
        xtask::shell_timing::join(
            &native_host(140).replace("timestamp_source=kernel", "timestamp_source=monotonic"),
            &metrics.offers,
            60000,
            1,
        )
        .is_err()
    );
    assert!(xtask::shell_timing::latency_report(&native_host(140), &bursty, 60000, 1).is_err());
    for bad in [
        bursty.replace("\"interval_msec\":250", "\"interval_msec\":0"),
        bursty.replace("\"interval_msec\":250", "\"interval_msec\":10001"),
        bursty.replace(",\"interval_msec\":250", ""),
        bursty.replace("\"source\":\"sealed\"", "\"source\":\"bytes\""),
        bursty.replace("\"wire\":\"files\"", "\"wire\":\"ipc\""),
        bursty.replace("\"schema\":4", "\"schema\":2"),
        bursty.replacen("\"schema\":4", "\"schema\":3", 1),
        bursty.replace("\"resource_id\":1", "\"resource_id\":99"),
    ] {
        assert!(collect_metrics(&bad, 1).is_err(), "accepted {bad}");
    }
    // No new field is emitted for historical fresh evidence.
    let historical = serde_json::to_value(collect_metrics(&old, 1).unwrap()).unwrap();
    assert!(historical.get("interval_msec").is_none());
}

#[test]
fn sealed_evidence_is_explicit_and_cannot_claim_the_ipc_or_legacy_path() {
    use xtask::shell_timing::ImageSource;
    let bytes = upload_journal();
    assert_eq!(
        collect_metrics(&bytes, 1).unwrap().source,
        ImageSource::Bytes
    );
    let sealed = bytes.replacen(
        "\"event\":\"start\"",
        "\"event\":\"start\",\"source\":\"sealed\"",
        1,
    );
    assert_eq!(
        collect_metrics(&sealed, 1).unwrap().source,
        ImageSource::Isb
    );
    assert!(collect_metrics(&sealed.replace("files", "ipc"), 1).is_err());
    assert!(collect_metrics(&sealed.replace("sealed", "unknown"), 1).is_err());
    let legacy = journal().replacen(
        "\"event\":\"start\"",
        "\"event\":\"start\",\"source\":\"sealed\"",
        1,
    );
    assert!(collect_metrics(&legacy, 1).is_err());
}

#[test]
fn nearest_rank_preserves_tail_and_distinguishes_missing_measurements() {
    use xtask::shell_timing::Distribution;
    assert!(Distribution::from_samples(&[]).is_none());
    let samples: Vec<_> = (1..=100).rev().collect();
    let result = Distribution::from_samples(&samples).unwrap();
    assert_eq!(
        (
            result.count,
            result.p50_usec,
            result.p95_usec,
            result.p99_usec,
            result.max_usec
        ),
        (100, 50, 95, 99, 100)
    );
    assert_eq!(Distribution::from_samples(&[0]).unwrap().p99_usec, 0);
}

fn native_host(time: u64) -> String {
    format!(
        "sophia_shell_native_binding schema=1 connection_epoch=1 content_grant_epoch=2 output=3 candidate_generation=1 native_owner=5 native_frame=6 head=7 target_generation=8 heads=1 mode_refresh_millihz=60000\nsophia_shell_native_completion schema=1 output=3 native_owner=5 native_frame=6 heads=1 monotonic_usec={time} timestamp_source=kernel missing_kernel_timestamp=0\n"
    )
}

#[test]
fn latency_report_requires_complete_native_evidence_and_retains_all_metrics() {
    use xtask::shell_timing::latency_report;
    let result = latency_report(&native_host(140), &upload_journal(), 60000, 1).unwrap();
    assert_eq!(result["candidate_to_native_presented"]["p99_usec"], 40);
    assert_eq!(result["presentation_upload_to_accepted"]["p95_usec"], 60);
    assert_eq!(result["demand_to_permit"]["p50_usec"], 10);
    assert!(result["probe_upload_to_accepted"].is_null());
    assert_eq!(result["native_samples"].as_array().unwrap().len(), 1);
    assert_eq!(
        result["journal_metrics"]["uploads"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(latency_report("", &upload_journal(), 60000, 1).is_err());
    assert!(latency_report(&native_host(140), &upload_journal(), 120000, 1).is_err());
    assert!(
        latency_report(
            &native_host(140),
            &upload_journal().replace("presented", "superseded"),
            60000,
            1
        )
        .is_err()
    );
    let old = latency_report(&native_host(140), &journal(), 60000, 1).unwrap();
    assert!(old["presentation_upload_to_accepted"].is_null());
}

#[test]
fn latency_report_keeps_uploads_that_reach_the_transfer_deadline() {
    let slow = upload_journal()
        .replace("\"begin_usec\":10", "\"begin_usec\":1")
        .replace("\"accepted_usec\":70", "\"accepted_usec\":2000001")
        .replace("\"demand_usec\":80", "\"demand_usec\":2000002")
        .replace("\"received_usec\":90", "\"received_usec\":2000003")
        .replace(
            "\"submitted_monotonic_usec\":100",
            "\"submitted_monotonic_usec\":2000004",
        )
        .replace(
            "\"observed_monotonic_usec\":150",
            "\"observed_monotonic_usec\":2000006",
        );
    let result =
        xtask::shell_timing::latency_report(&native_host(2000005), &slow, 60000, 1).unwrap();
    assert_eq!(
        result["transfer_bound_reached"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        result["presentation_upload_to_accepted"]["max_usec"],
        2000000
    );
}

fn upload_journal() -> String {
    journal().replace("\"schema\":1", "\"schema\":2").replace(
        "{\"schema\":2,\"event\":\"permit\"",
        concat!(
            "{\"schema\":2,\"event\":\"upload_offer\",\"upload\":{\"connection_epoch\":1,\"content_grant_epoch\":2,\"output\":3,\"candidate_generation\":1,\"resource_id\":1,\"resource_generation\":1,\"width\":1920,\"height\":24,\"total_bytes\":184320,\"begin_usec\":10}}\n",
            "{\"schema\":2,\"event\":\"upload_accepted\",\"resource_id\":1,\"resource_generation\":1,\"accepted_usec\":70}\n",
            "{\"schema\":2,\"event\":\"permit\""
        )
    )
}

#[test]
fn upload_and_permit_metrics_preserve_bytes_and_boundaries() {
    for wire in ["files", "ipc"] {
        let metrics = collect_metrics(&upload_journal().replace("files", wire), 1).unwrap();
        assert_eq!(metrics.schema, 2);
        assert_eq!(metrics.uploads.len(), 1);
        assert_eq!(metrics.uploads[0].upload.total_bytes, 180 * 1024);
        assert_eq!(metrics.uploads[0].upload.begin_usec, 10);
        assert_eq!(metrics.uploads[0].accepted_usec, 70);
        assert_eq!(metrics.permits[0].demand_usec, 80);
        assert_eq!(metrics.permits[0].received_usec, 90);
    }
    let old = collect_metrics(&journal(), 1).unwrap();
    assert_eq!(old.schema, 1);
    assert!(old.uploads.is_empty());
    assert_eq!(old.permits.len(), 1);
    // Preserve slow uploads for the budget verdict instead of exporting only fast ones.
    let slow = upload_journal()
        .replace("\"begin_usec\":10", "\"begin_usec\":1")
        .replace("\"accepted_usec\":70", "\"accepted_usec\":2000001")
        .replace("\"demand_usec\":80", "\"demand_usec\":2000002")
        .replace("\"received_usec\":90", "\"received_usec\":2000003")
        .replace(
            "\"submitted_monotonic_usec\":100",
            "\"submitted_monotonic_usec\":2000004",
        )
        .replace(
            "\"observed_monotonic_usec\":150",
            "\"observed_monotonic_usec\":2000005",
        );
    assert!(collect_metrics(&slow, 1).is_ok());
}

#[test]
fn incomplete_or_misattributed_uploads_invalidate_the_whole_journal() {
    let original = upload_journal();
    let lines: Vec<_> = original.lines().collect();
    for i in 0..lines.len() {
        let mut missing = lines.clone();
        missing.remove(i);
        assert!(collect(&(missing.join("\n") + "\n"), 1).is_err());
        let mut duplicate = lines.clone();
        duplicate.insert(i, lines[i]);
        assert!(collect(&(duplicate.join("\n") + "\n"), 1).is_err());
    }
    for bad in [
        original.replacen("\"schema\":2", "\"schema\":1", 1),
        original.replace("\"total_bytes\":184320", "\"total_bytes\":184321"),
        original.replace("\"width\":1920", "\"width\":0"),
        original.replace("\"resource_id\":1", "\"resource_id\":2"),
        original.replacen("\"resource_generation\":1", "\"resource_generation\":2", 1),
        original.replacen("\"connection_epoch\":1", "\"connection_epoch\":9", 1),
        original.replace("\"accepted_usec\":70", "\"accepted_usec\":9"),
        original.replace("\"accepted_usec\":70", "\"accepted_usec\":81"),
        original.replace("\"begin_usec\":10", "\"begin_usec\":0"),
        original.replace(
            "\"total_bytes\":184320",
            "\"total_bytes\":184320,\"unknown\":1",
        ),
        original.trim_end().to_owned(),
    ] {
        assert!(collect_metrics(&bad, 1).is_err(), "accepted {bad}");
    }
}

fn journal() -> String {
    [
        r#"{"schema":1,"event":"start","frames":1,"wire":"files"}"#,
        r#"{"schema":1,"event":"permit","generation":1,"demand_usec":80,"received_usec":90}"#,
        r#"{"schema":1,"event":"offer","connection_epoch":1,"content_grant_epoch":2,"output":3,"candidate_generation":1,"submitted_monotonic_usec":100}"#,
        r#"{"schema":1,"event":"terminal","connection_epoch":1,"content_grant_epoch":2,"output":3,"candidate_generation":1,"outcome":"presented","kind":2,"reason":0,"observed_monotonic_usec":150}"#,
        r#"{"schema":1,"event":"end","offered":1,"completed":1}"#,
        "",
    ].join("\n")
}

#[test]
fn bulk_probe_requires_exact_bytes_and_release_before_bar_upload() {
    let mut lines: Vec<serde_json::Value> = upload_journal()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    lines[0]["probe_bytes"] = 4194304.into();
    let mut offer = lines[1].clone();
    for (key, value) in [
        ("resource_id", 1),
        ("width", 1024),
        ("height", 1024),
        ("total_bytes", 4194304),
        ("begin_usec", 1),
    ] {
        offer["upload"][key] = value.into();
    }
    lines[1]["upload"]["resource_id"] = 2.into();
    lines[2]["resource_id"] = 2.into();
    lines.splice(1..1, [offer,
        serde_json::json!({"schema":2,"event":"upload_accepted","resource_id":1,"resource_generation":1,"accepted_usec":5}),
        serde_json::json!({"schema":2,"event":"probe_released","resource_id":1,"resource_generation":1,"released_usec":8})]);
    let encode = |lines: &[serde_json::Value]| {
        lines
            .iter()
            .map(|v| v.to_string() + "\n")
            .collect::<String>()
    };
    let good = encode(&lines);
    let result = collect_metrics(&good, 1).unwrap();
    assert_eq!(result.uploads.len(), 2);
    assert_eq!(result.uploads[0].purpose, "transfer_probe");
    assert_eq!(result.uploads[0].released_usec, Some(8));
    assert_eq!(result.uploads[1].purpose, "presentation");
    for (index, key, value) in [
        (3, "released_usec", 11),
        (3, "resource_id", 2),
        (2, "resource_generation", 2),
        (0, "probe_bytes", 0),
        (0, "probe_bytes", 4194303),
    ] {
        let mut bad = lines.clone();
        bad[index][key] = value.into();
        assert!(collect_metrics(&encode(&bad), 1).is_err());
    }
    for index in 1..=3 {
        let mut bad = lines.clone();
        bad.remove(index);
        assert!(collect_metrics(&encode(&bad), 1).is_err());
    }
    assert!(
        collect_metrics(
            &good.replace("\"total_bytes\":4194304", "\"total_bytes\":4194300"),
            1
        )
        .is_err()
    );
}

#[test]
fn complete_journal_preserves_candidate_and_submission_time() {
    for wire in ["files", "ipc"] {
        let offers = collect(&journal().replace("files", wire), 1).unwrap();
        assert_eq!(offers.len(), 1);
        assert_eq!(offers[0].candidate_generation, 1);
        assert_eq!(offers[0].submitted_monotonic_usec, 100);
        assert_eq!(offers[0].outcome, "presented");
    }
}

#[test]
fn incomplete_rejected_or_reordered_journals_never_export_survivors() {
    let original = journal();
    let lines: Vec<_> = original.lines().collect();
    for i in 0..lines.len() {
        let missing = lines
            .iter()
            .enumerate()
            .filter(|(n, _)| *n != i)
            .map(|(_, line)| *line)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        assert!(collect(&missing, 1).is_err());
        let mut duplicate = lines.clone();
        duplicate.insert(i, lines[i]);
        assert!(collect(&(duplicate.join("\n") + "\n"), 1).is_err());
    }
    for bad in [
        original.trim_end().to_string(),
        original.replace("\"kind\":2", "\"kind\":3"),
        original.replace("\"reason\":0", "\"reason\":1"),
        original.replace("presented", "rejected"),
        original.replace(
            "\"observed_monotonic_usec\":150",
            "\"observed_monotonic_usec\":99",
        ),
        original.replace("\"received_usec\":90", "\"received_usec\":101"),
        original.replace("\"candidate_generation\":1", "\"candidate_generation\":2"),
        original.replace("\"schema\":1", "\"schema\":2"),
        original.replace("\"schema\":1", "\"schema\":2,\"schema\":1"),
        original.replace("\"frames\":1", "\"frames\":0,\"frames\":1"),
        original.replace("\"completed\":1", "\"completed\":0"),
        original.replace("\"frames\":1", "\"frames\":1,\"unknown\":0"),
        original.clone() + "{\"schema\":1,\"event\":\"failed\",\"error\":\"timeout\"}\n",
    ] {
        assert!(collect(&bad, 1).is_err(), "accepted {bad}");
    }
    assert!(collect(&original, 2).is_err());
}

#[test]
fn collector_cli_exports_join_input_outside_checkout() {
    let root = std::env::temp_dir().join(format!("shell-journal-cli-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("client.ndjson");
    std::fs::write(&path, journal()).unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
        .current_dir(&root)
        .args(["shell-timing", "collect"])
        .arg(&path)
        .arg("1")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let offers: Vec<xtask::shell_timing::Offer> = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(offers.len(), 1);
    std::fs::write(&path, upload_journal()).unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
        .current_dir(&root)
        .args(["shell-timing", "metrics"])
        .arg(&path)
        .arg("1")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let metrics: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(metrics["schema"], 2);
    assert_eq!(metrics["uploads"][0]["upload"]["total_bytes"], 184320);
    std::fs::remove_dir_all(root).unwrap();
}

/// Schema 3 retained rows (shape A): N generation-0 pool uploads
/// (ids 2..=N+1, generation 1), then per frame an optional probe, exact
/// rotated placements, permit, offer and terminal. No release records:
/// the pool stays live at End.
fn retained_rows(frames: u64, count: u64, probe: bool) -> Vec<serde_json::Value> {
    let tile = 1920 / count.max(1) / 2 * 2;
    let mut t = 0u64;
    let mut next = move || {
        t += 10;
        t
    };
    let mut rows = vec![
        serde_json::json!({"schema":3,"event":"start","frames":frames,"wire":"files","source":"sealed","probe_bytes":if probe {4194304} else {0},"workload":"retained","retained_resources":count}),
    ];
    for i in 0..count {
        let begin = next();
        rows.push(serde_json::json!({"schema":3,"event":"upload_offer","upload":{"connection_epoch":1,"content_grant_epoch":2,"output":3,"candidate_generation":0,"resource_id":i+2,"resource_generation":1,"width":tile,"height":24,"total_bytes":tile*24*4,"begin_usec":begin}}));
        let accepted = next();
        rows.push(serde_json::json!({"schema":3,"event":"upload_accepted","resource_id":i+2,"resource_generation":1,"accepted_usec":accepted}));
    }
    for f in 1..=frames {
        if probe {
            let begin = next();
            rows.push(serde_json::json!({"schema":3,"event":"upload_offer","upload":{"connection_epoch":1,"content_grant_epoch":2,"output":3,"candidate_generation":f,"resource_id":1,"resource_generation":f,"width":1024,"height":1024,"total_bytes":4194304,"begin_usec":begin}}));
            let accepted = next();
            rows.push(serde_json::json!({"schema":3,"event":"upload_accepted","resource_id":1,"resource_generation":f,"accepted_usec":accepted}));
            let released = next();
            rows.push(serde_json::json!({"schema":3,"event":"probe_released","resource_id":1,"resource_generation":f,"released_usec":released}));
        }
        let demand = next();
        let received = next();
        rows.push(serde_json::json!({"schema":3,"event":"permit","generation":f,"demand_usec":demand,"received_usec":received}));
        let placements: Vec<_> = (0..count)
            .map(|i| {
                serde_json::json!({"resource_id":i+2,"resource_generation":1,"destination_x_px":(i+f-1)%count*tile,"destination_y_px":0})
            })
            .collect();
        rows.push(
            serde_json::json!({"schema":3,"event":"candidate_placements","candidate_generation":f,"placements":placements}),
        );
        let submitted = next();
        rows.push(serde_json::json!({"schema":3,"event":"offer","connection_epoch":1,"content_grant_epoch":2,"output":3,"candidate_generation":f,"submitted_monotonic_usec":submitted}));
        let observed = next();
        rows.push(serde_json::json!({"schema":3,"event":"terminal","connection_epoch":1,"content_grant_epoch":2,"output":3,"candidate_generation":f,"outcome":"presented","kind":2,"reason":0,"observed_monotonic_usec":observed}));
    }
    rows.push(serde_json::json!({"schema":3,"event":"end","offered":frames,"completed":frames}));
    rows
}

fn encode_rows(rows: &[serde_json::Value]) -> String {
    rows.iter().map(|r| r.to_string() + "\n").collect()
}

#[test]
fn retained_journal_collects_exact_pool_placements_and_live_set() {
    use xtask::shell_timing::WorkloadMode;
    let metrics = collect_metrics(&encode_rows(&retained_rows(3, 3, false)), 3).unwrap();
    assert_eq!(metrics.schema, 3);
    assert_eq!(metrics.workload, WorkloadMode::Retained);
    assert_eq!(metrics.retained_resources, Some(3));
    assert_eq!(metrics.offers.len(), 3);
    assert_eq!(metrics.permits.len(), 3);
    assert_eq!(metrics.uploads.len(), 3);
    assert!(metrics.uploads.iter().all(|u| u.purpose == "retained"
        && u.upload.candidate_generation == 0
        && u.upload.total_bytes == 640 * 24 * 4));
    // Frame 2 rotates the pool by one tile: ids stay in pool order.
    assert_eq!(
        metrics.placements[1]
            .placements
            .iter()
            .map(|p| (p.resource_id, p.destination_x_px))
            .collect::<Vec<_>>(),
        vec![(2, 640), (3, 1280), (4, 0)]
    );
    // Shape A: the exact accepted pool is reported live at End, unreleased.
    assert_eq!(
        metrics
            .live_resources
            .iter()
            .map(|r| (r.resource_id, r.resource_generation, r.released_usec))
            .collect::<Vec<_>>(),
        vec![(2, 1, None), (3, 1, None), (4, 1, None)]
    );
    // The 1000-candidate shape: offers stay 1000, uploads stay N.
    let full = collect_metrics(&encode_rows(&retained_rows(1000, 8, false)), 1000).unwrap();
    assert_eq!(full.offers.len(), 1000);
    assert_eq!(full.uploads.len(), 8);
    assert_eq!(full.placements.len(), 1000);
    // The optional per-frame probe keeps its existing transfer_probe role.
    let probed = collect_metrics(&encode_rows(&retained_rows(2, 2, true)), 2).unwrap();
    assert_eq!(
        probed
            .uploads
            .iter()
            .filter(|u| u.purpose == "transfer_probe")
            .count(),
        2
    );
    assert_eq!(
        probed
            .uploads
            .iter()
            .filter(|u| u.purpose == "retained")
            .count(),
        2
    );
    // No latency acceptance is designed for retained yet.
    assert!(
        xtask::shell_timing::latency_report(
            &native_host(140),
            &encode_rows(&retained_rows(1, 2, false)),
            60000,
            1
        )
        .is_err()
    );
}

#[test]
fn retained_journal_rejects_pool_and_reference_violations() {
    let good = retained_rows(3, 3, false);
    let find = |rows: &[serde_json::Value], event: &str| {
        rows.iter().position(|r| r["event"] == event).unwrap()
    };
    // Pool size is required and bounded at declaration time.
    for count in [0, 1, 9] {
        assert!(collect_metrics(&encode_rows(&retained_rows(1, count, false)), 1).is_err());
    }
    // Schema 3 without an explicit retained declaration is not a journal.
    let mut rows = good.clone();
    rows[0].as_object_mut().unwrap().remove("workload");
    assert!(collect_metrics(&encode_rows(&rows), 3).is_err());
    let mut rows = good.clone();
    rows[0]
        .as_object_mut()
        .unwrap()
        .remove("retained_resources");
    assert!(collect_metrics(&encode_rows(&rows), 3).is_err());
    // Pool uploads are generation 0, ids from 2, generation 1, even tiles.
    for (key, value) in [
        ("candidate_generation", 1),
        ("resource_id", 1),
        ("resource_generation", 2),
    ] {
        let mut rows = good.clone();
        rows[1]["upload"][key] = value.into();
        assert!(collect_metrics(&encode_rows(&rows), 3).is_err(), "{key}");
    }
    let mut rows = good.clone();
    rows[1]["upload"]["width"] = 641.into();
    rows[1]["upload"]["total_bytes"] = (641 * 24 * 4).into();
    assert!(collect_metrics(&encode_rows(&rows), 3).is_err());
    // A missing pool acceptance means no candidate may reference the pool.
    let mut rows = good.clone();
    rows.remove(2);
    assert!(collect_metrics(&encode_rows(&rows), 3).is_err());
    // Placements precede the offer, rotate exactly, and reference only
    // accepted pool resources at generation 1.
    let placed = find(&good, "candidate_placements");
    let mut rows = good.clone();
    rows[placed]["placements"][0]["destination_x_px"] = 640.into();
    assert!(collect_metrics(&encode_rows(&rows), 3).is_err());
    let mut rows = good.clone();
    rows[placed]["placements"][0]["resource_id"] = 99.into();
    assert!(collect_metrics(&encode_rows(&rows), 3).is_err());
    let mut rows = good.clone();
    rows[placed]["placements"][0]["resource_generation"] = 2.into();
    assert!(collect_metrics(&encode_rows(&rows), 3).is_err());
    let mut rows = good.clone();
    let offer = find(&rows, "offer");
    rows.swap(placed, offer);
    assert!(collect_metrics(&encode_rows(&rows), 3).is_err());
    let mut rows = good.clone();
    rows.remove(placed);
    assert!(collect_metrics(&encode_rows(&rows), 3).is_err());
    // Shape A rejects any retained_released record: before End, right
    // after the last terminal, or after End. Tiles stay live at End.
    let release = serde_json::json!({"schema":3,"event":"retained_released","resource_id":2,"resource_generation":1,"released_usec":9999});
    let mut rows = good.clone();
    let end = rows.len() - 1;
    rows.insert(end, release.clone());
    assert!(collect_metrics(&encode_rows(&rows), 3).is_err());
    let mut rows = good.clone();
    rows.push(release.clone());
    assert!(collect_metrics(&encode_rows(&rows), 3).is_err());
    let mut rows = good.clone();
    let last_terminal = rows.iter().rposition(|r| r["event"] == "terminal").unwrap();
    rows.insert(last_terminal + 1, release);
    assert!(collect_metrics(&encode_rows(&rows), 3).is_err());
    // Fresh journals cannot smuggle a retained declaration.
    let mut rows = good.clone();
    for row in &mut rows {
        row["schema"] = 2.into();
    }
    assert!(collect_metrics(&encode_rows(&rows), 3).is_err());
}
