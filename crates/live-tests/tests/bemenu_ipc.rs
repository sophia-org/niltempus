//! LIVE gate, IPC twin of bemenu_files: the same prepared Bemenu artifact,
//! sandbox, owners, phases and assertions over the current `sophia_shell_v1`
//! socket wire (today's rollback path). The application sees exactly PATH
//! and SOPHIA_SHELL_SOCKET, must announce `status=negotiated revision=7`
//! without `wire=9p`, and runs the full open, raster, edit, Accept, close,
//! held-lease retirement, reopen and stop flow beside a neighbouring bar.
//!
//!   SOPHIA_BEMENU_ARTIFACT=<output-dir> SOPHIA_BEMENU_SHA256=<binary sha256> \
//!   SOPHIA_BEMENU_COMMIT=<signed commit> nice -n 19 \
//!   cargo test -p live-tests --test bemenu_ipc -- --ignored --nocapture
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
fn bemenu_executable_serves_the_native_launcher_over_the_current_ipc_socket() {
    gate::run(fixture::Wire::Ipc);
}
