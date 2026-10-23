use serde_json::{Value, json};
use xtask::shell_timing::{ImageSource, collect_metrics};

fn rows() -> Vec<Value> {
    let mut rows = vec![
        json!({"event":"start","frames":3,"wire":"files","source":"owned_ring","probe_bytes":0,"workload":"fresh","ring_slots":2}),
        json!({"event":"ring_ready","connection_epoch":1,"content_grant_epoch":2,"channel":3,"ring":4,"ring_generation":1,"slots":2,"pixel_bytes":184320,"admitted_usec":1,"ready_usec":2}),
    ];
    for f in 1..=3 {
        let base = f * 100;
        let id = (f - 1) % 2 + 1;
        let generation = (f - 1) / 2 + 1;
        rows.extend([
            json!({"event":"ring_offer","offer":f,"slot":(f-1)%2,"slot_generation":2*f,"fill_start_usec":base,
                "upload":{"connection_epoch":1,"content_grant_epoch":2,"output":7,"candidate_generation":f,
                    "resource_id":id,"resource_generation":generation,"width":1920,"height":24,"total_bytes":184320,"begin_usec":base+1}}),
            json!({"event":"ring_accepted","offer":f,"resource_id":id,"resource_generation":generation,"accepted_usec":base+2}),
            json!({"event":"slot_released","offer":f,"slot":(f-1)%2,"slot_generation":2*f,
                "resource_id":id,"resource_generation":generation,"released_usec":base+3}),
            json!({"event":"permit","generation":f,"demand_usec":base+4,"received_usec":base+5}),
            json!({"event":"offer","connection_epoch":1,"content_grant_epoch":2,"output":7,"candidate_generation":f,"submitted_monotonic_usec":base+6}),
            json!({"event":"terminal","connection_epoch":1,"content_grant_epoch":2,"output":7,"candidate_generation":f,"outcome":"presented","kind":2,"reason":0,"observed_monotonic_usec":base+8}),
        ]);
    }
    rows.push(json!({"event":"end","offered":3,"completed":3}));
    for row in &mut rows {
        row["schema"] = 5.into();
    }
    rows
}

fn encode(rows: &[Value]) -> String {
    rows.iter().map(|row| format!("{row}\n")).collect()
}

fn sdk_rows() -> Vec<Value> {
    let mut rows = rows();
    for row in &mut rows {
        row["schema"] = 7.into();
        match row["event"].as_str().unwrap() {
            "ring_ready" => row["admitted_usec"] = Value::Null,
            "ring_offer" | "slot_released" => {
                row["slot"] = 0.into();
                row["slot_generation"] = Value::Null;
            }
            _ => {}
        }
    }
    rows
}

#[test]
fn sdk_ring_records_observed_custody_without_private_sdk_fields() {
    let mut rows = sdk_rows();
    // Acceptance and settlement may be observed in the same poll.
    rows[4]["released_usec"] = rows[3]["accepted_usec"].clone();
    let metrics = collect_metrics(&encode(&rows), 3).unwrap();
    assert_eq!(metrics.schema, 7);
    let ring = metrics.ring.unwrap();
    assert!(ring.registration.admitted_usec.is_none());
    assert!(
        ring.copies
            .iter()
            .all(|c| c.slot == 0 && c.slot_generation.is_none())
    );

    for (row, key, value) in [
        (1, "admitted_usec", json!(1)),
        (2, "slot_generation", json!(2)),
        (2, "slot", json!(2)),
        (4, "slot", json!(1)),
        (4, "slot_generation", json!(2)),
        (4, "released_usec", json!(101)),
        (3, "resource_generation", json!(2)),
        (0, "interval_msec", json!(100)),
        (0, "workload", json!("retained")),
        (3, "schema", json!(5)),
    ] {
        let mut bad = sdk_rows();
        bad[row][key] = value;
        assert!(collect_metrics(&encode(&bad), 3).is_err(), "{row}:{key}");
    }
    // SDK observations cannot be relabeled as raw-protocol evidence.
    for row in &mut rows {
        row["schema"] = 5.into();
    }
    assert!(collect_metrics(&encode(&rows), 3).is_err());
}

#[test]
fn ring_custody_is_distinct_and_slots_can_recycle_before_resource_release() {
    let metrics = collect_metrics(&encode(&rows()), 3).unwrap();
    assert_eq!(metrics.source, ImageSource::OwnedRing);
    assert_eq!(metrics.uploads.len(), 3);
    assert!(
        metrics
            .uploads
            .iter()
            .all(|copy| copy.released_usec.is_none())
    );
    let ring = metrics.ring.unwrap();
    assert_eq!(ring.registration.slots, 2);
    assert_eq!(ring.copies[2].slot, 0);
    assert_eq!(ring.copies[2].slot_generation, Some(6));
    assert_eq!(ring.copies[2].resource_generation, 2);
    assert_eq!(ring.copies[2].fill_start_usec, 300);
    assert_eq!(metrics.offers[2].candidate_generation, 3);
}

#[test]
fn ring_candidates_require_exact_kernel_completions_on_every_head() {
    let metrics = collect_metrics(&encode(&rows()), 3).unwrap();
    let mut host = String::new();
    for frame in 1..=3 {
        for head in [11, 12] {
            host.push_str(&format!("sophia_shell_native_binding schema=1 connection_epoch=1 content_grant_epoch=2 output=7 candidate_generation={frame} native_owner=5 native_frame={frame} head={head} target_generation=8 heads=2 mode_refresh_millihz=60000\n"));
        }
        host.push_str(&format!("sophia_shell_native_completion schema=1 output=7 native_owner=5 native_frame={frame} heads=2 monotonic_usec={} timestamp_source=kernel missing_kernel_timestamp=0\n",frame*100+7));
    }
    let joined = xtask::shell_timing::join(&host, &metrics.offers, 60000, 3).unwrap();
    assert_eq!(joined.len(), 3);
    assert!(
        joined
            .iter()
            .all(|sample| sample.heads == 2 && sample.latency_usec == 1)
    );
    assert!(
        xtask::shell_timing::join(
            &host.replace("timestamp_source=kernel", "timestamp_source=monotonic"),
            &metrics.offers,
            60000,
            3
        )
        .is_err()
    );
    let missing = host
        .lines()
        .filter(|line| !line.contains("head=12"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert!(xtask::shell_timing::join(&missing, &metrics.offers, 60000, 3).is_err());
}

#[test]
fn ring_journal_rejects_identity_geometry_and_lifetime_substitution() {
    for (row, key, value) in [
        (0, "source", json!("sealed")),
        (0, "ring_slots", json!(1)),
        (0, "probe_bytes", json!(4194304)),
        (0, "workload", json!("retained")),
        (1, "slots", json!(3)),
        (1, "admitted_usec", json!(3)),
        (1, "admitted_usec", Value::Null),
        (1, "pixel_bytes", json!(184324)),
        (1, "connection_epoch", json!(0)),
        (2, "event", json!("upload_offer")),
        (2, "slot", json!(1)),
        (2, "slot_generation", json!(1)),
        (2, "slot_generation", Value::Null),
        (2, "fill_start_usec", json!(0)),
        (3, "offer", json!(2)),
        (3, "resource_generation", json!(2)),
        (4, "slot_generation", json!(4)),
        (4, "slot_generation", Value::Null),
        (4, "released_usec", json!(101)),
        (5, "demand_usec", json!(102)),
        (6, "output", json!(8)),
        (7, "kind", json!(3)),
        (7, "schema", json!(2)),
        (7, "observed_monotonic_usec", json!(105)),
        (20, "completed", json!(2)),
    ] {
        let mut bad = rows();
        bad[row][key] = value;
        assert!(
            collect_metrics(&encode(&bad), 3).is_err(),
            "accepted row {row} key {key}"
        );
    }
    for key in [
        "resource_id",
        "resource_generation",
        "candidate_generation",
        "total_bytes",
        "content_grant_epoch",
    ] {
        let mut bad = rows();
        bad[14]["upload"][key] = 99.into();
        assert!(collect_metrics(&encode(&bad), 3).is_err(), "accepted {key}");
    }
    let mut bad = rows();
    bad.swap(3, 4);
    assert!(collect_metrics(&encode(&bad), 3).is_err());
    let mut bad = rows();
    bad[3]["forged"] = true.into();
    assert!(collect_metrics(&encode(&bad), 3).is_err());
    let mut bad = rows();
    bad[1]["forged"] = true.into();
    assert!(collect_metrics(&encode(&bad), 3).is_err());
    let valid = encode(&rows());
    assert!(collect_metrics(valid.trim_end(), 3).is_err());
    assert!(collect_metrics(&valid, 2).is_err());
}

/// Frames 250 ms apart with absolute due times 50 us before each fill.
fn bursty_rows() -> Vec<Value> {
    let mut rows = rows();
    rows[0]["interval_msec"] = 250.into();
    for f in 0..3u64 {
        let shift = f * 250_000;
        for row in &mut rows[2 + 6 * f as usize..8 + 6 * f as usize] {
            for key in [
                "fill_start_usec",
                "accepted_usec",
                "released_usec",
                "demand_usec",
                "received_usec",
                "submitted_monotonic_usec",
                "observed_monotonic_usec",
            ] {
                if let Some(t) = row.get(key).and_then(Value::as_u64) {
                    row[key] = (t + shift).into();
                }
            }
            if let Some(t) = row["upload"]["begin_usec"].as_u64() {
                row["upload"]["begin_usec"] = (t + shift).into();
            }
        }
        rows[2 + 6 * f as usize]["due_usec"] = (50 + shift).into();
    }
    rows
}

#[test]
fn bursty_ring_journal_binds_every_offer_to_one_absolute_schedule() {
    let metrics = collect_metrics(&encode(&bursty_rows()), 3).unwrap();
    assert_eq!(metrics.interval_msec, Some(250));
    let ring = metrics.ring.as_ref().unwrap();
    assert_eq!(ring.copies[2].due_usec, Some(500_050));
    assert_eq!(ring.copies[2].fill_start_usec, 500_300);
    // A late fill keeps later deadlines on the original schedule.
    let mut late = bursty_rows();
    for (row, key) in [(2, "fill_start_usec"), (3, "accepted_usec")] {
        late[row][key] = 240_000.into();
    }
    late[2]["upload"]["begin_usec"] = 240_000.into();
    for (row, key) in [
        (4, "released_usec"),
        (5, "demand_usec"),
        (5, "received_usec"),
        (6, "submitted_monotonic_usec"),
        (7, "observed_monotonic_usec"),
    ] {
        late[row][key] = 240_001.into();
    }
    assert!(collect_metrics(&encode(&late), 3).is_ok());
    // Historical fresh evidence keeps its exact serialized shape.
    let fresh = collect_metrics(&encode(&rows()), 3).unwrap();
    assert_eq!(fresh.interval_msec, None);
    assert!(!serde_json::to_string(&fresh).unwrap().contains("due_usec"));
}

#[test]
fn bursty_ring_journal_refuses_missing_shifted_or_unpaced_due_times() {
    for (row, key, value) in [
        (0, "interval_msec", json!(0)),
        (0, "interval_msec", json!(10_001)),
        (0, "interval_msec", json!(251)),
        (2, "due_usec", json!(1)),   // before readiness
        (2, "due_usec", json!(101)), // after its fill started
        (8, "due_usec", json!(250_049)),
        (14, "due_usec", json!(500_051)),
    ] {
        let mut bad = bursty_rows();
        bad[row][key] = value;
        assert!(
            collect_metrics(&encode(&bad), 3).is_err(),
            "accepted row {row} {key}"
        );
    }
    for row in [2, 8, 14] {
        let mut bad = bursty_rows();
        bad[row].as_object_mut().unwrap().remove("due_usec");
        assert!(collect_metrics(&encode(&bad), 3).is_err(), "row {row}");
    }
    let mut bad = bursty_rows();
    bad[0].as_object_mut().unwrap().remove("interval_msec");
    assert!(collect_metrics(&encode(&bad), 3).is_err());
    let mut bad = rows();
    bad[2]["due_usec"] = 50.into();
    assert!(collect_metrics(&encode(&bad), 3).is_err());
}

/// Three 640x24 tiles copied once through two slots, then rotated per frame.
fn retained_rows() -> Vec<Value> {
    let mut rows = vec![
        json!({"event":"start","frames":3,"wire":"files","source":"owned_ring","probe_bytes":0,"workload":"retained","retained_resources":3,"ring_slots":2}),
        json!({"event":"ring_ready","connection_epoch":1,"content_grant_epoch":2,"channel":3,"ring":4,"ring_generation":1,"slots":2,"pixel_bytes":184320,"admitted_usec":1,"ready_usec":2}),
    ];
    for tile in 1..=3u64 {
        let base = tile * 10;
        rows.extend([
            json!({"event":"ring_offer","offer":tile,"slot":(tile-1)%2,"slot_generation":2*tile,"fill_start_usec":base,
                "upload":{"connection_epoch":1,"content_grant_epoch":2,"output":7,"candidate_generation":0,
                    "resource_id":tile,"resource_generation":1,"width":640,"height":24,"total_bytes":61440,"begin_usec":base+1}}),
            json!({"event":"ring_accepted","offer":tile,"resource_id":tile,"resource_generation":1,"accepted_usec":base+2}),
            json!({"event":"slot_released","offer":tile,"slot":(tile-1)%2,"slot_generation":2*tile,
                "resource_id":tile,"resource_generation":1,"released_usec":base+3}),
        ]);
    }
    for f in 1..=3u64 {
        let base = f * 100;
        let placements: Vec<Value> = (0..3u64)
            .map(|i| {
                json!({"resource_id":i+1,"resource_generation":1,
                "destination_x_px":(i+f-1)%3*640,"destination_y_px":0})
            })
            .collect();
        rows.extend([
            json!({"event":"permit","generation":f,"demand_usec":base+4,"received_usec":base+5}),
            json!({"event":"candidate_placements","candidate_generation":f,"placements":placements}),
            json!({"event":"offer","connection_epoch":1,"content_grant_epoch":2,"output":7,"candidate_generation":f,"submitted_monotonic_usec":base+6}),
            json!({"event":"terminal","connection_epoch":1,"content_grant_epoch":2,"output":7,"candidate_generation":f,"outcome":"presented","kind":2,"reason":0,"observed_monotonic_usec":base+8}),
        ]);
    }
    rows.push(json!({"event":"end","offered":3,"completed":3}));
    for row in &mut rows {
        row["schema"] = 5.into();
    }
    rows
}

#[test]
fn retained_ring_pool_is_copied_once_and_every_frame_places_it_rotated() {
    use xtask::shell_timing::WorkloadMode;
    let metrics = collect_metrics(&encode(&retained_rows()), 3).unwrap();
    assert_eq!(metrics.workload, WorkloadMode::Retained);
    assert_eq!(metrics.retained_resources, Some(3));
    assert_eq!(metrics.uploads.len(), 3);
    assert!(metrics.uploads.iter().all(|u| u.purpose == "retained"));
    assert_eq!(metrics.live_resources.len(), 3);
    assert_eq!(metrics.placements.len(), 3);
    assert_eq!(metrics.placements[1].placements[0].destination_x_px, 640);
    assert_eq!(metrics.placements[2].placements[2].destination_x_px, 640);
    let ring = metrics.ring.unwrap();
    assert_eq!(ring.copies.len(), 3);
    assert_eq!(
        (ring.copies[2].slot, ring.copies[2].slot_generation),
        (0, Some(6))
    );
    assert_eq!(metrics.offers.len(), 3);
}

#[test]
fn retained_ring_journal_refuses_pool_and_placement_substitution() {
    for (row, key, value) in [
        (0, "retained_resources", json!(1)),
        (0, "retained_resources", json!(9)),
        (0, "interval_msec", json!(250)),
        (2, "offer", json!(2)),
        (2, "slot", json!(1)),
        (6, "resource_id", json!(1)),
        (12, "candidate_generation", json!(2)),
        (13, "candidate_generation", json!(2)),
        (11, "demand_usec", json!(32)), // before the last pool release
    ] {
        let mut bad = retained_rows();
        bad[row][key] = value;
        assert!(
            collect_metrics(&encode(&bad), 3).is_err(),
            "accepted row {row} {key}"
        );
    }
    for (key, value) in [
        ("width", json!(641)),
        ("candidate_generation", json!(1)),
        ("resource_generation", json!(2)),
        ("output", json!(8)),
    ] {
        let mut bad = retained_rows();
        bad[5]["upload"][key] = value;
        assert!(collect_metrics(&encode(&bad), 3).is_err(), "accepted {key}");
    }
    for (index, key, value) in [
        (0, "destination_x_px", json!(0)),
        (0, "destination_x_px", json!(-640)),
        (1, "destination_y_px", json!(24)),
        (2, "resource_id", json!(1)),
        (2, "resource_generation", json!(2)),
    ] {
        let mut bad = retained_rows();
        bad[16]["placements"][index][key] = value;
        assert!(collect_metrics(&encode(&bad), 3).is_err(), "accepted {key}");
    }
    let mut bad = retained_rows();
    bad[16]["placements"].as_array_mut().unwrap().pop();
    assert!(collect_metrics(&encode(&bad), 3).is_err());
    let mut bad = retained_rows();
    bad.remove(12); // first frame without placements
    assert!(collect_metrics(&encode(&bad), 3).is_err());
    // A fresh journal cannot carry a pool, nor a retained one a fresh copy.
    let mut bad = rows();
    bad[0]["retained_resources"] = 2.into();
    assert!(collect_metrics(&encode(&bad), 3).is_err());
    let mut bad = retained_rows();
    let copy = rows()[2].clone();
    bad.insert(11, copy);
    assert!(collect_metrics(&encode(&bad), 3).is_err());
}
