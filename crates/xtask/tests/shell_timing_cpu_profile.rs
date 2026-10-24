//! Device-free profile policy and retained sample controls; no native launch.
use std::path::Path;
use xtask::shell_timing_cpu_profile::*;
use xtask::shell_timing_environment::Interval;

#[test]
fn recording_is_user_only_inherited_and_uses_monotonic_dwarf_stacks() {
    let args = record_arguments(
        Path::new("/tmp/data"),
        Path::new("/bin/runner"),
        Path::new("/tmp/profile.json"),
        "sha",
        Path::new("/tmp/run"),
    );
    assert_eq!(
        args,
        [
            "record",
            "--event=cpu-clock:u",
            "--freq=199",
            "--call-graph=dwarf,8192",
            "--clockid=mono",
            "--timestamp",
            "--output",
            "/tmp/data",
            "--",
            "/bin/runner",
            "shell-timing",
            "capture-cpu-profile",
            "/tmp/profile.json",
            "sha",
            "/tmp/run"
        ]
    );
}

#[test]
fn monotonic_filter_preserves_microseconds_without_float_rounding() {
    assert_eq!(
        time_filter(&Interval {
            start_usec: 1_000_001,
            end_usec: 9_999_999
        })
        .unwrap(),
        "1.000001,9.999999"
    );
    for (start, end) in [(0, 0), (2, 1)] {
        assert!(
            time_filter(&Interval {
                start_usec: start,
                end_usec: end
            })
            .is_err()
        );
    }
}

#[test]
fn samples_require_the_outer_process_event_and_measured_time() {
    let interval = Interval {
        start_usec: 1_000_001,
        end_usec: 2_000_000,
    };
    let good = "  42 1.000001000: cpu-clock:u:  abc symbol (/binary)\n42 2.000000000: cpu-clock:u: def [unknown] (/binary)\n";
    assert_eq!(sample_counts(good, 42, &interval).unwrap(), (2, 1));
    for bad in [
        String::new(),
        good.replace("42", "1"),
        good.replace("cpu-clock:u:", "cpu-clock:"),
        good.replace("1.000001000:", "1.000000999:"),
        good.replace("2.000000000:", "2.000000001:"),
        good.replace("1.000001000:", "1.1:"),
        good.replace("1.000001000:", "+1.000001000:"),
    ] {
        assert!(sample_counts(&bad, 42, &interval).is_err(), "{bad}");
    }
}

#[test]
fn loss_evidence_must_be_present_unique_and_zero() {
    assert!(check_losses("#\n# Total Lost Samples: 0\n# Samples: 20\n").is_ok());
    for bad in [
        "",
        "# Total Lost Samples: 1\n",
        "# Total Lost Samples: unknown\n",
        "# Total Lost Samples: 0\n# Total Lost Samples: 0\n",
    ] {
        assert!(check_losses(bad).is_err());
    }
}

#[test]
fn report_filters_require_a_process_and_a_nonempty_interval() {
    let interval = Interval {
        start_usec: 1_000_001,
        end_usec: 2_000_000,
    };
    let args = report_arguments(Path::new("/tmp/perf.data"), 42, &interval).unwrap();
    for required in [
        "--pid=42",
        "--time=1.000001,2.000000",
        "--sort=pid,dso,symbol",
        "--percentage=relative",
        "--call-graph=fractal,0,callee",
        "--no-inline",
        "--percent-limit=0",
    ] {
        assert!(args.iter().any(|arg| arg == required));
    }
    assert!(report_arguments(Path::new("/unused"), 0, &interval).is_err());
    assert!(
        report_arguments(
            Path::new("/unused"),
            42,
            &Interval {
                start_usec: 2,
                end_usec: 1,
            },
        )
        .is_err()
    );
}

#[test]
fn shared_symbols_keep_thread_rows_and_cannot_add_foreign_samples() {
    let good = "# Samples: 3  of event 'cpu-clock:u'\n\
        66.67% 2 42:session libc.so.6 [.] memcpy\n\
          |\n\
          ---memcpy\n\
             session_caller\n\
        33.33% 1 43:worker libc.so.6 [.] memcpy\n";
    check_report_samples(good, 3).unwrap();
    // A foreign shared-symbol row cannot be hidden by a correct-looking header.
    let mixed = format!("{good}33.33% 1 99:peer libc.so.6 [.] memcpy\n");
    assert_eq!(
        check_report_samples(&mixed, 3).unwrap_err(),
        "perf histogram differs from the selected process's raw sample count"
    );
    assert!(check_report_samples(&good.replace("33.33% 1", "33.33% 0"), 3).is_err());
    assert!(check_report_samples(&good.replace("42:session", "libc.so.6"), 3).is_err());
    assert!(check_report_samples("", 0).is_err());
    assert!(check_report_samples("", 3).is_err());
    assert!(check_report_samples("100.00% broken 42:session", 3).is_err());
}

fn clock_usec() -> u64 {
    let t = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    t.tv_sec as u64 * 1_000_000 + t.tv_nsec as u64 / 1000
}

#[inline(never)]
fn profile_shared_cpu_work() -> u64 {
    let end = std::time::Instant::now() + std::time::Duration::from_millis(500);
    let mut x = 1u64;
    while std::time::Instant::now() < end {
        x = std::hint::black_box(x.wrapping_mul(6364136223846793005).wrapping_add(1));
    }
    x
}

#[test]
fn profile_cpu_child_payload() {
    let Ok(path) = std::env::var("T331_PROFILE_CHILD_CONTROL") else {
        return;
    };
    let start = clock_usec();
    std::hint::black_box(profile_shared_cpu_work());
    std::fs::write(
        path,
        format!("{} {start} {}", std::process::id(), clock_usec()),
    )
    .unwrap();
}

#[test]
fn profile_cpu_payload() {
    let Ok(path) = std::env::var("T331_PROFILE_CONTROL") else {
        return;
    };
    let start = clock_usec();
    // The same executable and hot function appear in two processes. Omitting
    // pid from perf's sort key used to merge them despite the --pid filter.
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "profile_cpu_child_payload", "--nocapture"])
        .env_remove("T331_PROFILE_CONTROL")
        .env(
            "T331_PROFILE_CHILD_CONTROL",
            Path::new(&path).with_extension("child"),
        )
        .spawn()
        .unwrap();
    let worker = std::thread::spawn(profile_shared_cpu_work);
    std::hint::black_box(worker.join().unwrap());
    assert!(child.wait().unwrap().success());
    std::fs::write(
        path,
        format!("{} {start} {}", std::process::id(), clock_usec()),
    )
    .unwrap();
}

#[test]
#[ignore = "device-free local perf permission/inheritance control; invoke explicitly"]
fn perf_records_worker_threads_and_filters_the_outer_tgid_and_clock() {
    let root = std::env::temp_dir().join(format!("t331-profile-control-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(root.clone());
    let data = root.join("perf.data");
    let identity = root.join("identity");
    let perf = "/usr/bin/perf";
    let captured = std::process::Command::new(perf)
        .args([
            "record",
            "--event=cpu-clock:u",
            "--freq=199",
            "--call-graph=dwarf,8192",
            "--clockid=mono",
            "--timestamp",
            "--output",
        ])
        .arg(&data)
        .arg("--")
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", "profile_cpu_payload", "--nocapture"])
        .env("T331_PROFILE_CONTROL", &identity)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        captured.status.success(),
        "{}",
        String::from_utf8_lossy(&captured.stderr)
    );
    let raw = std::fs::read_to_string(&identity).unwrap();
    let fields = raw
        .split_whitespace()
        .map(|s| s.parse::<u64>().unwrap())
        .collect::<Vec<_>>();
    let interval = Interval {
        start_usec: fields[1],
        end_usec: fields[2],
    };
    let child = std::fs::read_to_string(identity.with_extension("child")).unwrap();
    let child_pid = child
        .split_whitespace()
        .next()
        .unwrap()
        .parse::<u32>()
        .unwrap();
    assert_ne!(fields[0] as u32, child_pid);
    for pid in [fields[0] as u32, child_pid] {
        let filters = [
            format!("--pid={pid}"),
            format!("--time={}", time_filter(&interval).unwrap()),
        ];
        let script = std::process::Command::new(perf)
            .args([
                "script",
                "-G",
                "--ns",
                "--fields=pid,time,event,ip,sym,dso",
                "--input",
            ])
            .arg(&data)
            .args(&filters)
            .env("LC_ALL", "C")
            .output()
            .unwrap();
        assert!(
            script.status.success(),
            "{}",
            String::from_utf8_lossy(&script.stderr)
        );
        let (count, _) =
            sample_counts(std::str::from_utf8(&script.stdout).unwrap(), pid, &interval).unwrap();
        assert!(count > 0);
        let report = std::process::Command::new(perf)
            .args(report_arguments(&data, pid, &interval).unwrap())
            .env("LC_ALL", "C")
            .output()
            .unwrap();
        assert!(report.status.success());
        let output = std::str::from_utf8(&report.stdout).unwrap();
        check_losses(output).unwrap();
        check_report_samples(output, count).unwrap();
        assert!(
            output.contains(&format!("# Samples: {count}  of event 'cpu-clock:u'")),
            "report must count only this TGID, matching the independently checked raw samples: {output}"
        );
    }
}

#[test]
fn only_observed_cpu_plans_can_be_profiled() {
    use std::collections::BTreeMap;
    use xtask::shell_timing_native::{Artifact, Request, Source};
    let mut request = Request {
        sophia_source: Source {
            path: "/unused".into(),
            commit: "a".repeat(40),
        },
        integration_source: Source {
            path: "/unused".into(),
            commit: "b".repeat(40),
        },
        artifacts: BTreeMap::from([(
            "peer-config".into(),
            Artifact {
                path: "/unused".into(),
                sha256: "c".repeat(64),
            },
        )]),
        tty: "/dev/tty4".into(),
        display: 94,
        input_seat: "seat0".into(),
        runtime_ms: 60_000,
        wire: "ipc".into(),
        source: xtask::shell_timing::ImageSource::Bytes,
        output: 1,
        frames: 1000,
        mode_millihz: 120_000,
        instrumentation: "cpu-only".into(),
        environment_readback: true,
        external_timing: None,
        reference_workload: None,
        producer: Default::default(),
        managed_destination_reuse: false,
    };
    assert!(projected(&request, 0, "sha").is_ok());
    assert!(projected(&request, 4_194_304, "sha").is_ok());
    assert!(projected(&request, 1, "sha").is_err());
    request.instrumentation = "rust-global".into();
    assert!(projected(&request, 0, "sha").is_err());
    request.instrumentation = "cpu-only".into();
    request.environment_readback = false;
    assert!(projected(&request, 0, "sha").is_err());
    request.environment_readback = true;
    request.source = xtask::shell_timing::ImageSource::Isb;
    assert!(projected(&request, 0, "sha").is_err());
    request.wire = "files".into();
    assert!(projected(&request, 0, "sha").is_ok());
    request.frames = 999;
    assert!(projected(&request, 0, "sha").is_err());
}
