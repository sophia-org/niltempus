//! Preparation receipt shared by the legacy and SFT paths.
use std::io::{self, Write};

pub(super) fn write(
    output: &mut impl Write,
    generation: u64,
    resource_id: u64,
    start_usec: u64,
    end_usec: u64,
) -> io::Result<()> {
    // stderr is unbuffered and shares the capture file with the host.
    let line = format!(
        "gpu_bar_prepare schema=1 arm=owned_ring generation={generation} resource_id={resource_id} start_usec={start_usec} end_usec={end_usec}\n"
    );
    output.write_all(line.as_bytes())
}

#[cfg(test)]
#[path = "../tests/support/prepare_log.rs"]
mod tests;
