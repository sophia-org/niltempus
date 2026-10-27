//! Place an exit after the live-chain checks, without scheduling sleeps.
use super::*;
use std::io::Write;
use xtask::shell_timing_process_binding::ProcessBinding;

fn at_executable_probe(
    change: impl FnOnce(&Fixture, &ProcessBinding) + Send,
) -> Result<BindingRecheck, String> {
    let fx = fixture();
    let binding = fx.bound();
    assert_eq!(fx.binder.recheck(&binding).unwrap(), BindingRecheck::Bound);
    let cmdline = fx.proc_root.join(format!("{HOST}/cmdline"));
    let argv = fs::read(&cmdline).unwrap();
    fs::remove_file(&cmdline).unwrap();
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &cmdline,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    std::thread::scope(|scope| {
        let writer = scope.spawn(|| {
            // Opening the write end requires recheck's cmdline reader. That
            // reader runs after chain/stat and exact-position checks, and
            // cannot finish read_to_end until this writer sends argv and EOF.
            let mut pipe = fs::OpenOptions::new().write(true).open(&cmdline).unwrap();
            change(&fx, &binding);
            pipe.write_all(&argv).unwrap();
        });
        let result = fx.binder.recheck(&binding);
        // If a future recheck returns before the barrier, unblock the writer
        // so this test reports its wrong result instead of hanging on join.
        let _unblock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&cmdline)
            .unwrap();
        writer.join().unwrap();
        result
    })
}

#[test]
fn same_identity_supervisor_exit_during_recheck_is_ended() {
    let result = at_executable_probe(|fx, binding| {
        write(
            &fx.proc_root.join(format!("{SUPERVISOR}/stat")),
            &stat(SUPERVISOR, HOST, binding.supervisor.start_time_ticks, "Z"),
        );
        fs::remove_file(fx.proc_root.join(format!("{SUPERVISOR}/exe"))).unwrap();
    });
    assert_eq!(result, Ok(BindingRecheck::Ended));
}

#[test]
fn supervisor_disappearing_during_recheck_is_ended() {
    let result = at_executable_probe(|fx, _| {
        fs::remove_file(fx.proc_root.join(format!("{SUPERVISOR}/exe"))).unwrap();
        fs::remove_file(fx.proc_root.join(format!("{SUPERVISOR}/stat"))).unwrap();
    });
    assert_eq!(result, Ok(BindingRecheck::Ended));
}

#[test]
fn executable_probe_race_controls_stay_fail_closed() {
    assert_eq!(at_executable_probe(|_, _| {}), Ok(BindingRecheck::Bound));
    for fault in [
        "absent_live",
        "deleted_live",
        "deleted_ended",
        "wrong_live",
        "wrong_ended",
        "reused_ended",
        "opaque_ended",
    ] {
        let result = at_executable_probe(|fx, binding| {
            let exe = fx.proc_root.join(format!("{SUPERVISOR}/exe"));
            match fault {
                "absent_live" => fs::remove_file(exe).unwrap(),
                "deleted_live" | "deleted_ended" => link(
                    &PathBuf::from(format!("{} (deleted)", fx.supervisor_file.display())),
                    &exe,
                ),
                "wrong_live" | "wrong_ended" => link(&other_or(fx), &exe),
                "reused_ended" => fs::remove_file(exe).unwrap(),
                "opaque_ended" => fx.opaque(SUPERVISOR),
                _ => unreachable!(),
            }
            if fault.ends_with("_ended") {
                let start =
                    binding.supervisor.start_time_ticks + u64::from(fault == "reused_ended");
                write(
                    &fx.proc_root.join(format!("{SUPERVISOR}/stat")),
                    &stat(SUPERVISOR, HOST, start, "Z"),
                );
            }
        });
        assert!(result.is_err(), "{fault}: {result:?}");
    }
}
