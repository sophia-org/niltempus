//! One real source release applied after a bounded diagnostic delay.
#[path = "main.rs"]
#[allow(dead_code)]
mod peer;

fn main() {
    peer::release_delay_main();
}
