//! Explicit untimed diagnostic binary; never a timed comparator peer.
#[path = "main.rs"]
#[allow(dead_code)]
mod peer;

fn main() {
    peer::diagnostic_main();
}
