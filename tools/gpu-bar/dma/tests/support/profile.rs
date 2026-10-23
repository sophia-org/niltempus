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
        reuse: StageTiming::default(),
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

#[test]
fn fresh_window_refuses_any_reuse() {
    let stage = StageTiming {
        count: 2000,
        thread_cpu_ns: 20,
        wall_ns: 30,
    };
    let fresh = DmaTiming {
        setup: stage,
        draw: stage,
        release: stage,
        export: stage,
        destroy: stage,
        reuse: StageTiming::default(),
    };
    let zero = DmaTiming::default();
    let unchanged = record(zero, fresh).unwrap();
    assert!(unchanged.starts_with("gpu_bar_dma_timing schema=1 first_generation=121"));
    assert!(!unchanged.contains("reuse"), "schema 1 output is unchanged");
    for reuse in [
        StageTiming {
            count: 1,
            ..Default::default()
        },
        StageTiming {
            thread_cpu_ns: 1,
            ..Default::default()
        },
        StageTiming {
            wall_ns: 1,
            ..Default::default()
        },
    ] {
        assert!(
            record(zero, DmaTiming { reuse, ..fresh })
                .unwrap_err()
                .contains("reuse")
        );
    }
}

#[test]
fn reuse_window_needs_2000_reuses_and_no_fresh_stage() {
    let warm = StageTiming {
        count: 1,
        thread_cpu_ns: 7,
        wall_ns: 9,
    };
    // One fresh allocation in warmup, unchanged through the window.
    let start = DmaTiming {
        setup: warm,
        draw: warm,
        release: warm,
        export: warm,
        destroy: StageTiming::default(),
        reuse: StageTiming {
            count: 119,
            thread_cpu_ns: 100,
            wall_ns: 200,
        },
    };
    let end = DmaTiming {
        reuse: StageTiming {
            count: 2119,
            thread_cpu_ns: 150,
            wall_ns: 290,
        },
        ..start
    };
    assert_eq!(
        reuse_record(start, end).unwrap(),
        "gpu_bar_dma_timing schema=2 mode=reuse first_generation=121 last_generation=2120 \
         reuse_count=2000 reuse_cpu_ns=50 reuse_wall_ns=90 setup_count=0 draw_count=0 \
         release_count=0 export_count=0 destroy_count=0"
    );
    let mut short = end;
    short.reuse.count -= 1;
    assert!(reuse_record(start, short).unwrap_err().contains("reuse"));
    let mut regressed = end;
    regressed.reuse.thread_cpu_ns = 99;
    assert!(
        reuse_record(start, regressed)
            .unwrap_err()
            .contains("CPU timing regressed")
    );
    regressed = end;
    regressed.reuse.wall_ns = 199;
    assert!(
        reuse_record(start, regressed)
            .unwrap_err()
            .contains("wall timing regressed")
    );
    // Any fresh-stage change in the window, even time without a count.
    for (name, change) in [
        (
            "setup",
            DmaTiming {
                setup: StageTiming { count: 2, ..warm },
                ..end
            },
        ),
        (
            "draw",
            DmaTiming {
                draw: StageTiming {
                    thread_cpu_ns: 8,
                    ..warm
                },
                ..end
            },
        ),
        (
            "release",
            DmaTiming {
                release: StageTiming {
                    wall_ns: 10,
                    ..warm
                },
                ..end
            },
        ),
        (
            "export",
            DmaTiming {
                export: StageTiming {
                    thread_cpu_ns: 6,
                    ..warm
                },
                ..end
            },
        ),
        (
            "destroy",
            DmaTiming {
                destroy: StageTiming {
                    count: 1,
                    ..Default::default()
                },
                ..end
            },
        ),
    ] {
        assert!(
            reuse_record(start, change).unwrap_err().contains(name),
            "{name}"
        );
    }
}
