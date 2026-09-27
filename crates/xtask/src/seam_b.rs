// Provenance: copied from Sophia crates/sophia-conformance/src/direct_scanout.rs
// (`record_after_marker`, pub(crate)) at 9fcaec782ce4fe9978568c0466ee17a78b3d4571.
//
// WAITING ON SEAM B. The dock transcript verifier reads Sophia session logs,
// whose record grammar Sophia owns. Sophia is to publish a public record
// parser; until the director relays that API's approval this is a marked
// temporary duplicate of the one private helper dock.rs used, and nothing here
// is bound to a proposed name. When the seam lands, delete this file and call
// the public parser from dock.rs (the only caller).
//! Temporary stand-in for Sophia's session-log record reader (seam B).

/// The record text after `marker`, wherever it appears in a decorated
/// session-log line.
pub(crate) fn record_after_marker<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    line.find(marker).map(|start| &line[start + marker.len()..])
}
