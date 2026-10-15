use xtask::shell_timing::{collect, collect_metrics};

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
