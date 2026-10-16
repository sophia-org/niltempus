use xtask::shell_progress::{NAMES, collect};
fn row(process: &str, time: u64, count: u64) -> String {
    let mut row = format!(
        "sophia_comparator_progress schema=1 mode=progress process={process} pid=1 sequence={time} before_usec={time} after_usec={time}"
    );
    for name in NAMES {
        row.push_str(&format!(" {name}={count}"));
    }
    row.push('\n');
    row
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
