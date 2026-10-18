//! Only a two-millisecond single-worker control. No campaign load is launched.
use xtask::shell_timing_cpu_worker::{MAX_DURATION_MS, run, work};

#[test]
fn one_worker_exits_after_its_monotonic_duration() {
    let report = work(2).unwrap();
    assert_eq!(report.pid, std::process::id());
    assert_eq!(report.requested_ms, 2);
    assert!(report.elapsed_usec >= 2000);
    // Scheduling can delay return, so a wall-time ceiling is not asserted.
    let encoded = serde_json::to_value(report).unwrap();
    assert_eq!(encoded["schema"], 1);
}

#[test]
fn invalid_worker_requests_never_start_a_load() {
    assert!(work(0).is_err());
    assert!(work(MAX_DURATION_MS + 1).is_err());
    for value in [
        "",
        "0",
        "01",
        "+1",
        "-1",
        " 1",
        "600001",
        "18446744073709551616",
    ] {
        assert!(run(&[value.into()]).is_err(), "{value}");
    }
    assert!(run(&[]).is_err());
    assert!(run(&["1".into(), "extra".into()]).is_err());
}
