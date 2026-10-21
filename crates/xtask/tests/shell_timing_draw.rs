use serde_json::{Value, json};
use xtask::shell_timing::{DrawSource, DrawWorkload, ImageSource, collect_metrics};

fn encode(rows: &[Value]) -> String {
    rows.iter()
        .map(|row| {
            let mut row = row.clone();
            row["schema"] = 6.into();
            format!("{row}\n")
        })
        .collect()
}

fn permit(frame: u64, base: u64) -> Value {
    json!({"event":"permit","generation":frame,"demand_usec":base,"received_usec":base+1})
}
fn offer(frame: u64, base: u64) -> Value {
    json!({"event":"offer","connection_epoch":1,"content_grant_epoch":1,"output":7,"candidate_generation":frame,"submitted_monotonic_usec":base+2})
}
fn terminal(frame: u64, base: u64) -> Value {
    json!({"event":"terminal","connection_epoch":1,"content_grant_epoch":1,"output":7,"candidate_generation":frame,"outcome":"presented","kind":2,"reason":0,"observed_monotonic_usec":base+3})
}
fn draw_pair(tx: u64, id: u64, generation: u64, w: u32, h: u32, base: u64) -> Vec<Value> {
    vec![
        json!({"event":"draw_submitted","transaction":tx,"resource_id":id,"resource_generation":generation,"width":w,"height":h,"bytes":100,"submitted_usec":base}),
        json!({"event":"draw_settled","transaction":tx,"resource_id":id,"resource_generation":generation,"status":2,"reason":0,"settled_usec":base+1}),
    ]
}

/// Mixed retained run: panel by ring (one 1920 tile), value by draw,
/// updated at frames 1 and 9 only; 9 frames so frame 9 is exercised.
fn mixed_retained() -> Vec<Value> {
    let mut rows = vec![
        json!({"event":"draw_ready","content_grant_epoch":1,"ready_usec":10}),
        json!({"event":"ring_ready","connection_epoch":1,"content_grant_epoch":1,"ready_usec":11}),
        json!({"event":"start","frames":9,"wire":"files","source":"mixed","workload":"retained","output_width":1920,"output_height":1080,"panel_tiles":1,"panel_height":24,"tile_width":96,"tile_height":24}),
        json!({"event":"ring_submitted","offer":1,"submitted_usec":20,"upload":{"connection_epoch":1,"content_grant_epoch":1,"output":7,"candidate_generation":0,"resource_id":1,"resource_generation":1,"width":1920,"height":24,"total_bytes":184320,"begin_usec":20}}),
        json!({"event":"ring_settled","offer":1,"resource_id":1,"resource_generation":1,"settled_usec":21}),
    ];
    for f in 1..=9u64 {
        let base = 100 + f * 100;
        if f == 1 || f == 9 {
            let generation = if f == 1 { 1 } else { 2 };
            rows.extend(draw_pair(100 + generation, 900, generation, 96, 24, base));
        }
        rows.push(permit(f, base + 10));
        rows.push(offer(f, base + 10));
        rows.push(terminal(f, base + 10));
    }
    rows.push(json!({"event":"settled_end","settled_usec":1013,"held_until_usec":1_001_013}));
    rows.push(json!({"event":"end","offered":9,"completed":9}));
    rows
}

#[test]
fn mixed_retained_settles_panel_and_value_before_permits() {
    let metrics = collect_metrics(&encode(&mixed_retained()), 9).unwrap();
    assert_eq!(metrics.schema, 6);
    assert_eq!(metrics.source, ImageSource::Mixed);
    assert_eq!(metrics.offers.len(), 9);
    let draw = metrics.draw.unwrap();
    assert_eq!(draw.source, DrawSource::Mixed);
    assert_eq!(draw.workload, DrawWorkload::Retained);
    assert_eq!(draw.batches.len(), 3);
    assert!(!draw.batches[0].via_draw);
    assert!(draw.batches[1].via_draw);
    assert_eq!(draw.hold_usec, 1_000_000);
}

#[test]
fn update_on_wrong_retained_frame_is_refused() {
    let mut rows = mixed_retained();
    // Insert a value update at frame 2 (not a multiple-of-8 boundary).
    let at = rows
        .iter()
        .position(|r| r["event"] == "permit" && r["generation"] == 2)
        .unwrap();
    for (i, row) in draw_pair(777, 900, 9, 96, 24, 290).into_iter().enumerate() {
        rows.insert(at + i, row);
    }
    assert!(collect_metrics(&encode(&rows), 9).is_err());
}

#[test]
fn bursty_requires_grid_dues_and_idle_refuses_updates() {
    // Bursty: dues at 250ms grid; frame 2 update without due refused by
    // the same journal with the due removed.
    let mut rows = vec![
        json!({"event":"draw_ready","content_grant_epoch":1,"ready_usec":10}),
        json!({"event":"start","frames":2,"wire":"files","source":"draw","workload":"bursty","output_width":96,"output_height":24,"panel_tiles":1,"panel_height":24,"tile_width":96,"tile_height":24}),
    ];
    rows.extend(draw_pair(100, 1, 1, 96, 24, 20));
    rows.extend(draw_pair(101, 900, 1, 96, 24, 100));
    rows.push(permit(1, 110));
    rows.push(offer(1, 110));
    rows.push(terminal(1, 110));
    rows.push(json!({"event":"value_due","candidate_generation":2,"due_usec":250_000,"observed_usec":250_100}));
    rows.extend(draw_pair(102, 900, 2, 96, 24, 250_200));
    rows.push(permit(2, 250_210));
    rows.push(offer(2, 250_210));
    rows.push(terminal(2, 250_210));
    rows.push(json!({"event":"settled_end","settled_usec":250_213,"held_until_usec":1_250_213}));
    rows.push(json!({"event":"end","offered":2,"completed":2}));
    let metrics = collect_metrics(&encode(&rows), 2).unwrap();
    assert_eq!(metrics.draw.as_ref().unwrap().value_dues.len(), 1);

    // Removing the due must refuse the frame-2 update.
    let rows: Vec<Value> = rows
        .into_iter()
        .filter(|r| r["event"] != "value_due")
        .collect();
    assert!(collect_metrics(&encode(&rows), 2).is_err());
}

#[test]
fn resource_id_reuse_needs_increasing_generation() {
    let mut rows = mixed_retained();
    // Reuse panel id 1 for the frame-9 value update at generation 1:
    // generation does not increase for that id.
    for row in &mut rows {
        if row["event"] == "draw_submitted" && row["resource_id"] == 900 {
            row["resource_id"] = 1.into();
        }
        if row["event"] == "draw_settled" && row["resource_id"] == 900 {
            row["resource_id"] = 1.into();
        }
    }
    assert!(collect_metrics(&encode(&rows), 9).is_err());
}

#[test]
fn ring_journal_spelling_normalizes_to_owned_ring() {
    // Journal spells the source "ring"; the normalized journal and
    // plan vocabulary spell it owned_ring (ImageSource::OwnedRing).
    let mut rows = vec![
        json!({"event":"ring_ready","connection_epoch":1,"content_grant_epoch":1,"ready_usec":10}),
        json!({"event":"start","frames":1,"wire":"files","source":"ring","workload":"idle","output_width":1920,"output_height":1080,"panel_tiles":1,"panel_height":24,"tile_width":96,"tile_height":24}),
        json!({"event":"ring_submitted","offer":1,"submitted_usec":20,"upload":{"connection_epoch":1,"content_grant_epoch":1,"output":7,"candidate_generation":0,"resource_id":1,"resource_generation":1,"width":1920,"height":24,"total_bytes":184320,"begin_usec":20}}),
        json!({"event":"ring_settled","offer":1,"resource_id":1,"resource_generation":1,"settled_usec":21}),
        json!({"event":"ring_submitted","offer":2,"submitted_usec":30,"upload":{"connection_epoch":1,"content_grant_epoch":1,"output":7,"candidate_generation":0,"resource_id":900,"resource_generation":1,"width":96,"height":24,"total_bytes":9216,"begin_usec":30}}),
        json!({"event":"ring_settled","offer":2,"resource_id":900,"resource_generation":1,"settled_usec":31}),
    ];
    rows.push(permit(1, 40));
    rows.push(offer(1, 40));
    rows.push(terminal(1, 40));
    rows.push(json!({"event":"settled_end","settled_usec":43,"held_until_usec":1_000_043}));
    rows.push(json!({"event":"end","offered":1,"completed":1}));
    let metrics = collect_metrics(&encode(&rows), 1).unwrap();
    assert_eq!(metrics.source, ImageSource::OwnedRing);
    let draw = metrics.draw.unwrap();
    assert_eq!(draw.source, DrawSource::Ring);
    assert!(!draw.source.uses_draw_helper());
    assert_eq!(draw.workload, DrawWorkload::Idle);
}

#[test]
fn short_post_terminal_hold_is_refused() {
    let mut rows = mixed_retained();
    for row in &mut rows {
        if row["event"] == "settled_end" {
            row["held_until_usec"] = 999.into();
        }
    }
    assert!(collect_metrics(&encode(&rows), 9).is_err());
}
