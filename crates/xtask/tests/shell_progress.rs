use xtask::shell_progress::{NAMES, collect};

/// The comparator's service ledger, in emission order after the 30 base
/// counters. Schema 4 must carry exactly these names.
const LEDGER: [&str; 17] = [
    "wire_owner_input",
    "wire_standalone_input",
    "wire_flush_useful",
    "wire_flush_no_progress",
    "wire_flush_failed",
    "wire_flush_blocked",
    "wire_flush_deferred",
    "wire_no_publication",
    "retry_wake_ready",
    "retry_wake_error",
    "retry_wake_pending",
    "retry_write_ready",
    "retry_write_error",
    "retry_write_pending",
    "retry_other_ready",
    "retry_other_error",
    "retry_other_pending",
];

/// Counters each schema carries: a prefix of the full name list.
fn width(schema: u8) -> usize {
    match schema {
        1 => 12,
        2 => 20,
        3 => 30,
        4 => 47,
        _ => unreachable!(),
    }
}

fn row_at(schema: u8, process: &str, time: u64, count: u64) -> String {
    let mut row = format!(
        "sophia_comparator_progress schema={schema} mode=progress process={process} pid=1 sequence={time} before_usec={time} after_usec={time}"
    );
    for name in &NAMES[..width(schema)] {
        row.push_str(&format!(" {name}={count}"));
    }
    row.push('\n');
    row
}

fn row(process: &str, time: u64, count: u64) -> String {
    row_at(4, process, time, count)
}

#[test]
fn accepts_separate_process_totals_and_refuses_ambiguous_evidence() {
    let good = row("session", 1, 0) + &row("peer", 1, 0) + &row("session", 2, 3);
    assert_eq!(collect(&good).unwrap().len(), 3);
    assert!(collect(&row("session", 1, 0)).is_err());
    assert!(collect(&(good.clone() + &row("session", 3, 2))).is_err());
    assert!(collect(&(good.clone() + &row("peer", 0, 2))).is_err());
    assert!(collect(&(good.clone() + "sophia_comparator_counters mode=cpu-only\n")).is_err());
    assert!(collect(&good.replace("peer_wait=0", "peer_wait=0 peer_wait=0")).is_err());
    assert!(collect(&good.replace("owner_pass=0", "owner_pass=no")).is_err());
}

#[test]
fn schema_four_names_are_the_base_counters_then_the_whole_ledger() {
    assert_eq!(NAMES.len(), 47);
    assert_eq!(&NAMES[30..], &LEDGER);
    assert!(NAMES[..30].iter().all(|name| !LEDGER.contains(name)));
}

#[test]
fn every_schema_replays_with_its_exact_counter_width() {
    for schema in 1..=4 {
        let text = row_at(schema, "session", 1, 0) + &row_at(schema, "peer", 1, 2);
        let samples = collect(&text).unwrap();
        assert_eq!(samples.len(), 2);
        assert!(
            samples
                .iter()
                .all(|sample| sample.counts.len() == width(schema))
        );
        assert!(
            collect(&text.replace("owner_pass=0", "owner_pass=no")).is_err(),
            "schema {schema} keeps numeric validation"
        );
    }
}

#[test]
fn historical_schema_replays_but_cannot_mix_with_new_counts() {
    let modern = row("session", 1, 0) + &row("peer", 1, 0);
    for old in 1..=3 {
        let previous = row_at(old, "session", 1, 0) + &row_at(old, "peer", 1, 0);
        assert!(collect(&(previous + &row("peer", 2, 1))).is_err());
        assert!(collect(&(modern.clone() + &row_at(old, "peer", 2, 1))).is_err());
    }
}

#[test]
fn schema_four_requires_exactly_the_service_ledger() {
    let good = row("session", 1, 0) + &row("peer", 1, 0);
    assert_eq!(collect(&good).unwrap()[0].counts.len(), 47);
    // Schema 3 rows carrying the ledger, or schema 4 rows without it.
    assert!(collect(&good.replace("schema=4", "schema=3")).is_err());
    let unledgered =
        (row_at(3, "session", 1, 0) + &row_at(3, "peer", 1, 0)).replace("schema=3", "schema=4");
    assert!(collect(&unledgered).is_err());
    assert!(collect(&good.replace("schema=4", "schema=5")).is_err());
    for name in LEDGER {
        let field = format!(" {name}=0");
        assert!(
            collect(&good.replace(&field, "")).is_err(),
            "missing {name}"
        );
        assert!(
            collect(&good.replace(&field, &format!(" {name}_renamed=0"))).is_err(),
            "renamed {name}"
        );
        assert!(
            collect(&good.replace(&field, &format!("{field}{field}"))).is_err(),
            "duplicated {name}"
        );
        assert!(
            collect(&good.replace(&field, &format!("{field} wire_extra=0"))).is_err(),
            "extra beside {name}"
        );
        assert!(
            collect(&good.replace(&field, &format!(" {name}=-1"))).is_err(),
            "invalid {name}"
        );
    }
}

#[test]
fn a_reset_service_ledger_counter_refuses_the_capture() {
    let grown = row("session", 1, 5) + &row("peer", 1, 5) + &row("session", 2, 6);
    assert_eq!(collect(&grown).unwrap().len(), 3);
    // Only one ledger counter goes backwards; every other counter grows.
    for name in LEDGER {
        let reset = row("session", 1, 5)
            + &row("peer", 1, 5)
            + &row("session", 2, 6).replace(&format!(" {name}=6"), &format!(" {name}=4"));
        assert!(collect(&reset).is_err(), "reset {name}");
    }
}
