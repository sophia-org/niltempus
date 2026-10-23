//! Producer timing is attribution only, outside the image journal.
use sophia_shell_gpu::{DmaTiming, StageTiming};
use std::fmt::Write;

const FRESH: [&str; 5] = ["setup", "draw", "release", "export", "destroy"];

fn fresh_stages(timing: DmaTiming) -> [StageTiming; 5] {
    [
        timing.setup,
        timing.draw,
        timing.release,
        timing.export,
        timing.destroy,
    ]
}

/// One stage's counted-window change: exactly `count` completions, with
/// neither total regressing. A stage expected idle must not change at all.
fn delta(
    name: &str,
    before: StageTiming,
    after: StageTiming,
    count: u64,
) -> Result<(u64, u64), String> {
    if after.count.checked_sub(before.count) != Some(count) {
        return Err(format!("{name}: expected {count} completed timing stages"));
    }
    let cpu = after
        .thread_cpu_ns
        .checked_sub(before.thread_cpu_ns)
        .ok_or_else(|| format!("{name}: CPU timing regressed"))?;
    let wall = after
        .wall_ns
        .checked_sub(before.wall_ns)
        .ok_or_else(|| format!("{name}: wall timing regressed"))?;
    if count == 0 && (cpu, wall) != (0, 0) {
        return Err(format!("{name}: idle stage accumulated time"));
    }
    Ok((cpu, wall))
}

/// Fresh exports: 2000 of every export stage and no reuse.
pub(super) fn record(start: DmaTiming, end: DmaTiming) -> Result<String, String> {
    let mut record =
        String::from("gpu_bar_dma_timing schema=1 first_generation=121 last_generation=2120");
    for ((name, before), after) in FRESH
        .into_iter()
        .zip(fresh_stages(start))
        .zip(fresh_stages(end))
    {
        let (cpu, wall) = delta(name, before, after, 2000)?;
        write!(
            record,
            " {name}_count=2000 {name}_cpu_ns={cpu} {name}_wall_ns={wall}"
        )
        .expect("writing to a String");
    }
    delta("reuse", start.reuse, end.reuse, 0)?;
    Ok(record)
}

/// One retained export: 2000 reuses and no fresh stage. Its allocation is
/// in warmup and its destruction after the window.
pub(super) fn reuse_record(start: DmaTiming, end: DmaTiming) -> Result<String, String> {
    let (cpu, wall) = delta("reuse", start.reuse, end.reuse, 2000)?;
    let mut record = format!(
        "gpu_bar_dma_timing schema=2 mode=reuse first_generation=121 last_generation=2120 \
         reuse_count=2000 reuse_cpu_ns={cpu} reuse_wall_ns={wall}"
    );
    for ((name, before), after) in FRESH
        .into_iter()
        .zip(fresh_stages(start))
        .zip(fresh_stages(end))
    {
        delta(name, before, after, 0)?;
        write!(record, " {name}_count=0").expect("writing to a String");
    }
    Ok(record)
}

#[cfg(test)]
#[path = "../tests/support/profile.rs"]
mod tests;
