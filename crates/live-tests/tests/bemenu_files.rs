// Provenance: moved from Sophia
// crates/sophia-runtime/tests/shell_bemenu_files.rs at
// 9fcaec782ce4fe9978568c0466ee17a78b3d4571 (Sophia rule 13).
//! LIVE gate: the real `bemenu-sophia --serve` executable, from a prepared,
//! signed-revision artifact, over Sophia's native launcher 9P file contract,
//! against the production file export and native launcher owners, launched by
//! the production protected-process supervisor (Bubblewrap, Shell role).
//!
//! The body lives in support/bemenu_files/gate.rs, with a neighbouring bar on
//! a second transport, allocation and candidate shape checks, and a held
//! renderer lease across close. Bemenu is 9P-only (536d6b2 onward): the gate
//! requires exactly SOPHIA_SHELL_9P_SOCKET in its environment, never the
//! retired SOPHIA_SHELL_SOCKET, and a negotiated line carrying wire=9p. The
//! former IPC twins (bemenu_ipc, bemenu_session_ipc) are retired with product
//! IPC.
//!
//! Opt-in: ordinary `cargo test` has no application artifact, so this test is
//! #[ignore]d. Its dedicated invocation fails closed on any absent or
//! mismatched input; it never passes or skips silently:
//!
//!   cargo xtask prepare-bemenu-artifact <source-repo> <signed-commit> <new-output-dir>
//!   SOPHIA_BEMENU_ARTIFACT=<output-dir> SOPHIA_BEMENU_SHA256=<binary sha256> \
//!   SOPHIA_BEMENU_COMMIT=<signed commit> \
//!   cargo test -p live-tests --test bemenu_files -- --ignored --nocapture
//!
//! (After tools/provision.sh, every cargo command here runs with the
//! provisioned private `CARGO_HOME` and `--offline --locked`.)
//!
//! Covered: negotiation; catalog and output-fact objects; opening; the peer's
//! allocation request; the actual Cairo raster upload; Prepared/Presented;
//! exact focus; a text edit whose next candidate changes rows and pixels;
//! exactly one keyboard Accept admission; close, allocation invalidation and
//! peer resource retirement to owner settlement; reopen with a reset query and
//! a second close; graceful SIGTERM stop. Not covered: pointer/ContentAction
//! activation, focus loss, scale or facts change mid-opening, reconnect, and
//! every expiry path (see the fixture's clock note). Session decisions are
//! scripted; no launch-policy or physical-rendering claim is made.
#[path = "support/bemenu_files/artifact.rs"]
mod artifact;
#[path = "support/bemenu_files/fixture.rs"]
mod fixture;
#[path = "support/bemenu_files/gate.rs"]
mod gate;
#[path = "support/bemenu_files/neighbour.rs"]
mod neighbour;
#[path = "support/bemenu_files/sandbox.rs"]
mod sandbox;

#[test]
#[ignore = "live Bemenu gate: needs SOPHIA_BEMENU_ARTIFACT, SOPHIA_BEMENU_SHA256, SOPHIA_BEMENU_COMMIT"]
fn bemenu_executable_serves_the_native_launcher_over_the_production_file_export() {
    gate::run(fixture::Wire::Files);
}
