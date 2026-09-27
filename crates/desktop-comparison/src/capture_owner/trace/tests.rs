// Provenance: moved from Sophia crates/sophia-conformance/src/desktop_comparison/capture_owner/trace/tests.rs at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
#![cfg(test)]

use super::parse_tracefs_probe;
use std::path::PathBuf;

#[test]
fn tracefs_probe_admits_only_exact_known_records() {
    for root in ["/sys/kernel/tracing", "/sys/kernel/debug/tracing"] {
        let record = format!(
            "desktop_comparison_tracefs_probe schema=1 status=ready tracefs={root} event=drm_vblank_event_delivered\n"
        );
        assert_eq!(
            parse_tracefs_probe(&record).expect("known tracefs root should pass"),
            PathBuf::from(root)
        );
    }
    assert!(parse_tracefs_probe(
        "desktop_comparison_tracefs_probe schema=1 status=ready tracefs=/tmp event=drm_vblank_event_delivered"
    )
    .is_err());
}
