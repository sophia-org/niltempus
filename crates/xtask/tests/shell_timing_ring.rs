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
    assert_eq!(ring.copies[2].slot_generation, 6);
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
        (1, "pixel_bytes", json!(184324)),
        (1, "connection_epoch", json!(0)),
        (2, "event", json!("upload_offer")),
        (2, "slot", json!(1)),
        (2, "slot_generation", json!(1)),
        (2, "fill_start_usec", json!(0)),
        (3, "offer", json!(2)),
        (3, "resource_generation", json!(2)),
        (4, "slot_generation", json!(4)),
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
