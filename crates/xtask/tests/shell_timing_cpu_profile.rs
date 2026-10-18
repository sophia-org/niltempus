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
fn profile_cpu_payload() {
    let Ok(path) = std::env::var("T331_PROFILE_CONTROL") else {
        return;
    };
    let clock = || {
        let t = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
        t.tv_sec as u64 * 1_000_000 + t.tv_nsec as u64 / 1000
    };
    let start = clock();
    let worker = std::thread::spawn(|| {
        let end = std::time::Instant::now() + std::time::Duration::from_millis(500);
        let mut x = 1u64;
        while std::time::Instant::now() < end {
            x = std::hint::black_box(x.wrapping_mul(6364136223846793005).wrapping_add(1));
        }
        x
    });
    std::hint::black_box(worker.join().unwrap());
    std::fs::write(path, format!("{} {start} {}", std::process::id(), clock())).unwrap();
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
    let raw = std::fs::read_to_string(identity).unwrap();
    let fields = raw
        .split_whitespace()
        .map(|s| s.parse::<u64>().unwrap())
        .collect::<Vec<_>>();
    let interval = Interval {
        start_usec: fields[1],
        end_usec: fields[2],
    };
    let filters = [
        format!("--pid={}", fields[0]),
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
    let (count, _) = sample_counts(
        std::str::from_utf8(&script.stdout).unwrap(),
        fields[0] as u32,
        &interval,
    )
    .unwrap();
    assert!(count > 0);
    let report = std::process::Command::new(perf)
        .args(["report", "--stdio", "--stdio-color=never", "--input"])
        .arg(&data)
        .args(&filters)
        .env("LC_ALL", "C")
        .output()
        .unwrap();
    assert!(report.status.success());
    check_losses(std::str::from_utf8(&report.stdout).unwrap()).unwrap();
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
    request.source = xtask::shell_timing::ImageSource::Sealed;
    assert!(projected(&request, 0, "sha").is_err());
    request.wire = "files".into();
    assert!(projected(&request, 0, "sha").is_ok());
    request.frames = 999;
    assert!(projected(&request, 0, "sha").is_err());
}
