use super::*;
use sophia_shell_gpu::StageTiming;

#[test]
fn counted_window_requires_every_stage_and_monotonic_totals() {
    let stage = StageTiming {
        count: 2000,
        thread_cpu_ns: 20,
        wall_ns: 30,
    };
    let mut end = DmaTiming {
        setup: stage,
        draw: stage,
        release: stage,
        export: stage,
        destroy: stage,
    };
    let zero = DmaTiming::default();
    let row = record(zero, end).unwrap();
    assert!(row.contains("setup_count=2000 setup_cpu_ns=20 setup_wall_ns=30"));
    assert!(row.contains("destroy_count=2000"));
    end.destroy.count -= 1;
    assert!(record(zero, end).unwrap_err().contains("destroy"));
    end.destroy.count += 1;
    let mut start = zero;
    start.export.thread_cpu_ns = 21;
    assert!(
        record(start, end)
            .unwrap_err()
            .contains("CPU timing regressed")
    );
    start = zero;
    start.draw.wall_ns = 31;
    assert!(
        record(start, end)
            .unwrap_err()
            .contains("wall timing regressed")
    );
}
