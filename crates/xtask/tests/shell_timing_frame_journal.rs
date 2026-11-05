use serde_json::{Value, json};
use xtask::shell_timing::collect_metrics;
const SFT: &str = include_str!("fixtures/sft/schema8-h24.ndjson");
fn encode(rows: &[Value]) -> String {
    rows.iter().map(|r| r.to_string() + "\n").collect()
}

#[test]
fn real_owner_records_keep_sft_separate_from_legacy() {
    for text in [SFT, include_str!("fixtures/sft/schema8-h512.ndjson")] {
        let result = collect_metrics(text, 4).unwrap();
        assert_eq!(result.schema, 8);
        assert_eq!(
            (
                result.offers.len(),
                result.uploads.len(),
                result.terminal_usec.len()
            ),
            (4, 4, 4)
        );
        assert!(result.permits.is_empty());
        let frame = result.frame.unwrap();
        assert_eq!(
            (
                frame.prepared_usec.len(),
                frame.predecessor_released_usec.len()
            ),
            (4, 3)
        );
        assert!(
            result.uploads[..3]
                .iter()
                .all(|u| u.released_usec.is_some())
        );
        assert!(result.uploads[3].released_usec.is_none());
        assert_eq!(result.retire_counters.unwrap().explicit_retires_sent, 0);
    }
    for text in [
        include_str!("fixtures/sft/schema7-h24.ndjson"),
        include_str!("fixtures/sft/schema7-h512.ndjson"),
    ] {
        let result = collect_metrics(text, 4).unwrap();
        assert_eq!(result.permits.len(), 4);
        assert!(result.frame.is_none());
        assert!(
            serde_json::to_value(&result)
                .unwrap()
                .get("frame")
                .is_none()
        );
    }
}

#[test]
fn identity_order_credit_and_source_custody_mutations_refuse() {
    let rows: Vec<Value> = SFT
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    for (event, field, bad) in [
        ("start", "frames", json!(5)),
        ("start", "ring_slots", json!(3)),
        ("ring_ready", "pixel_bytes", json!(4)),
        ("ring_ready", "connection_epoch", json!(0)),
        ("frame_opened", "stream_id", json!(0)),
        ("frame_opened", "observed_monotonic_usec", json!(0)),
        ("ring_offer", "slot", json!(2)),
        ("ring_offer", "slot_generation", json!(1)),
        ("ring_offer", "fill_start_usec", json!(0)),
        ("offer", "candidate_generation", json!(2)),
        ("offer", "content_grant_epoch", json!(99)),
        ("offer", "output", json!(99)),
        ("ring_accepted", "resource_generation", json!(99)),
        ("ring_accepted", "offer", json!(99)),
        ("frame_prepared", "candidate_generation", json!(99)),
        ("terminal", "kind", json!(3)),
        ("terminal", "reason", json!(1)),
        ("terminal", "observed_monotonic_usec", json!(0)),
        ("slot_released", "resource_id", json!(99)),
        ("slot_released", "slot", json!(1)),
        ("frame_retired", "resource_generation", json!(99)),
        ("frame_retired", "candidate_generation", json!(1)),
        ("frame_closed", "stream_id", json!(99)),
        ("frame_closed", "observed_monotonic_usec", json!(0)),
        ("end", "completed", json!(3)),
        ("end", "explicit_retires_sent", json!(3)),
        ("end", "predecessor_releases_received", json!(2)),
    ] {
        let mut bad_rows = rows.clone();
        let row = bad_rows.iter_mut().find(|r| r["event"] == event).unwrap();
        row[field] = bad;
        assert!(
            collect_metrics(&encode(&bad_rows), 4).is_err(),
            "{event}.{field}"
        );
    }
    for event in [
        "frame_opened",
        "ring_accepted",
        "frame_prepared",
        "slot_released",
        "terminal",
        "frame_retired",
        "frame_closed",
        "end",
    ] {
        let at = rows.iter().position(|r| r["event"] == event).unwrap();
        let mut missing = rows.clone();
        missing.remove(at);
        assert!(
            collect_metrics(&encode(&missing), 4).is_err(),
            "missing {event}"
        );
        let mut duplicate = rows.clone();
        duplicate.insert(at, rows[at].clone());
        assert!(
            collect_metrics(&encode(&duplicate), 4).is_err(),
            "duplicate {event}"
        );
    }
    let mut mixed = rows.clone();
    mixed[3]["schema"] = json!(7);
    assert!(collect_metrics(&encode(&mixed), 4).is_err());
    let mut early = rows.clone();
    let prepared = early
        .iter()
        .position(|r| r["event"] == "frame_prepared")
        .unwrap();
    let terminal = early.iter().position(|r| r["event"] == "terminal").unwrap();
    early.swap(prepared, terminal);
    assert!(collect_metrics(&encode(&early), 4).is_err());
    let mut invented = rows;
    invented.insert(
        3,
        json!({"schema":8,"event":"permit","generation":1,"demand_usec":1,"received_usec":1}),
    );
    assert!(collect_metrics(&encode(&invented), 4).is_err());
}
