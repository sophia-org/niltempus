//! Producer timing is attribution only, outside the image journal.
use sophia_shell_gpu::DmaTiming;

pub(super) fn record(start: DmaTiming, end: DmaTiming) -> Result<String, String> {
    let mut record =
        String::from("gpu_bar_dma_timing schema=1 first_generation=121 last_generation=2120");
    for (name, before, after) in [
        ("setup", start.setup, end.setup),
        ("draw", start.draw, end.draw),
        ("release", start.release, end.release),
        ("export", start.export, end.export),
        ("destroy", start.destroy, end.destroy),
    ] {
        let count = after
            .count
            .checked_sub(before.count)
            .filter(|count| *count == 2000)
            .ok_or_else(|| format!("{name}: expected 2000 completed timing stages"))?;
        let cpu = after
            .thread_cpu_ns
            .checked_sub(before.thread_cpu_ns)
            .ok_or_else(|| format!("{name}: CPU timing regressed"))?;
        let wall = after
            .wall_ns
            .checked_sub(before.wall_ns)
            .ok_or_else(|| format!("{name}: wall timing regressed"))?;
        use std::fmt::Write;
        write!(
            record,
            " {name}_count={count} {name}_cpu_ns={cpu} {name}_wall_ns={wall}"
        )
        .expect("writing to a String");
    }
    Ok(record)
}

#[cfg(test)]
#[path = "../tests/support/profile.rs"]
mod tests;
