//! Device-free kernel-syscalls profile policy; the perf control is ignored
//! by default and launches no native session.
use std::path::Path;
use xtask::shell_timing_cpu_profile::*;
use xtask::shell_timing_environment::Interval;

const CPU_LINE: &str = "cpu-clock/freq=997,call-graph=dwarf,stack-size=8192/: type: 1 (PERF_TYPE_SOFTWARE), size: 136, config: 0 (PERF_COUNT_SW_CPU_CLOCK), { sample_period, sample_freq }: 997, sample_type: IP|TID|TIME|ADDR|CALLCHAIN|PERIOD|REGS_USER|STACK_USER|IDENTIFIER|DATA_SRC, read_format: ID|LOST, disabled: 1, inherit: 1, mmap: 1, comm: 1, freq: 1, enable_on_exec: 1, task: 1, mmap_data: 1, sample_id_all: 1, exclude_guest: 1, exclude_callchain_user: 1, mmap2: 1, comm_exec: 1, use_clockid: 1, ksymbol: 1, bpf_event: 1, sample_regs_user: 0xff0fff, sample_stack_user: 8192, clockid: 1";

/// Shape of perf 6.5 `evlist -v` output from the combined local control.
fn syscall_line(name: &str) -> String {
    format!(
        "syscalls:sys_enter_{name}: type: 2 (PERF_TYPE_TRACEPOINT), size: 136, config: 0x329 (syscalls:sys_enter_{name}), {{ sample_period, sample_freq }}: 1, sample_type: IP|TID|TIME|CPU|PERIOD|RAW|IDENTIFIER, read_format: ID|LOST, disabled: 1, inherit: 1, enable_on_exec: 1, sample_id_all: 1, exclude_guest: 1, use_clockid: 1, clockid: 1"
    )
}

fn evlist() -> String {
    let mut lines = vec![CPU_LINE.to_string()];
    lines.extend(SYSCALLS.iter().map(|name| syscall_line(name)));
    lines.push(
        "# Tip: use 'perf evlist --trace-fields' to show fields for tracepoint events".into(),
    );
    lines.join("\n")
}

#[test]
fn kernel_recording_scopes_stacks_to_the_cpu_event_only() {
    let args = kernel_record_arguments(
        Path::new("/tmp/data"),
        Path::new("/bin/runner"),
        Path::new("/tmp/profile.json"),
        "sha",
        Path::new("/tmp/run"),
    );
    assert_eq!(
        args[..2],
        [
            "record",
            "--event=cpu-clock/freq=997,call-graph=dwarf,stack-size=8192/"
        ]
    );
    let syscalls = SYSCALLS
        .iter()
        .map(|name| format!("--event=syscalls:sys_enter_{name}/call-graph=no/"))
        .collect::<Vec<_>>();
    assert_eq!(args[2..2 + SYSCALLS.len()], syscalls[..]);
    assert_eq!(
        args[2 + SYSCALLS.len()..],
        [
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
    // No global sampling or stack option may reach the tracepoints, and no
    // user-only modifier may drop kernel samples.
    assert!(!args.iter().any(|arg| arg.starts_with("--call-graph")
        || arg.starts_with("--freq")
        || arg.starts_with("--count")
        || arg.ends_with(":u")));
}

#[test]
fn descriptors_without_a_mode_keep_the_user_only_capture() {
    let profile = |mode: &str| {
        format!(
            r#"{{"schema":1,"plan":{{"path":"/p","sha256":"a"}},"perf":{{"path":"/usr/bin/perf","sha256":"b"}},
            "target":{{"card":"card0","output":1,"scale_milli":1000,"connector":"eDP-1","width":1,"height":1,"mode_millihz":60000,"vrr":false}}{mode}}}"#
        )
    };
    let parse = |text: String| serde_json::from_str::<Profile>(&text).map(|p| p.mode);
    assert_eq!(parse(profile("")).unwrap(), CaptureMode::UserCpu);
    assert_eq!(
        parse(profile(r#","mode":"user-cpu""#)).unwrap(),
        CaptureMode::UserCpu
    );
    assert_eq!(
        parse(profile(r#","mode":"kernel-syscalls""#)).unwrap(),
        CaptureMode::KernelSyscalls
    );
    assert!(parse(profile(r#","mode":"kernel""#)).is_err());
    assert!(parse(profile(r#","extra":1"#)).is_err());
    assert_ne!(
        CaptureMode::UserCpu.scope(),
        CaptureMode::KernelSyscalls.scope()
    );
}

#[test]
fn recorded_attributes_must_match_the_mode() {
    assert!(check_evlist(&evlist()).is_ok());
    // perf may echo the configuration terms in the event name.
    assert!(
        check_evlist(&evlist().replace(
            "syscalls:sys_enter_futex:",
            "syscalls:sys_enter_futex/call-graph=no/:"
        ))
        .is_ok()
    );
    let read = syscall_line("read");
    for bad in [
        String::new(),
        evlist().replace(&format!("{read}\n"), ""),
        format!("{}\n{read}", evlist()),
        evlist().replace(
            &read,
            &read.replace("sample_freq }: 1,", "sample_freq }: 2,"),
        ),
        evlist().replace(&read, &read.replace("RAW|", "RAW|CALLCHAIN|")),
        evlist().replace(&read, &read.replace("clockid: 1", "clockid: 1, freq: 1")),
        evlist().replace(&read, &read.replace("inherit: 1", "inherit: 0")),
        evlist().replace("997, sample_type", "199, sample_type"),
        evlist().replace("sample_stack_user: 8192", "sample_stack_user: 4096"),
        evlist().replace("CALLCHAIN|PERIOD|REGS_USER|STACK_USER", "PERIOD"),
        evlist().replace(
            "exclude_guest: 1, exclude_callchain_user",
            "exclude_kernel: 1, exclude_callchain_user",
        ),
        evlist().replace("syscalls:sys_enter_read:", "syscalls:sys_enter_getpid:"),
        format!("{}\ncpu-clock:u: type: 1 (PERF_TYPE_SOFTWARE)", evlist()),
        format!("{}\nnot an event", evlist()),
    ] {
        assert!(check_evlist(&bad).is_err(), "{bad}");
    }
}

#[test]
fn cpu_histogram_excludes_syscall_sections() {
    let report = "# Total Lost Samples: 0\n#\n\
        # Samples: 3  of event 'cpu-clock/freq=997,call-graph=dwarf,stack-size=8192/'\n\
        # Event count (approx.): 3\n\
        66.67% 2 42:session [kernel.kallsyms] [k] do_syscall_64\n\
        33.33% 1 43:worker libc.so.6 [.] memcpy\n#\n\
        # Samples: 5  of event 'syscalls:sys_enter_read'\n\
        100.00% 5 42:session libc.so.6 [.] read\n";
    let section = cpu_report_section(report).unwrap();
    assert!(check_report_samples(section, 3).is_ok());
    assert!(check_report_samples(report, 3).is_err());
    assert!(
        cpu_report_section(&report.replace("cpu-clock/freq=997", "cpu-clock/freq=199")).is_err()
    );
    let doubled = format!(
        "{report}# Samples: 1  of event 'cpu-clock/freq=997,call-graph=dwarf,stack-size=8192/'\n"
    );
    assert!(cpu_report_section(&doubled).is_err());
    // perf 6.5 appends the --time filter to every section header.
    let sliced = report.replace("8192/'\n", "8192/' (time slices: 1.000001,2.000000)\n");
    assert!(check_report_samples(cpu_report_section(&sliced).unwrap(), 3).is_ok());
}

#[test]
fn lost_chunk_warnings_refuse_the_report() {
    assert!(check_lost_chunks("").is_ok());
    assert!(check_lost_chunks("Warning:\nProcessed 10 events and lost 2 chunks!\n").is_err());
}

#[test]
fn samples_split_cpu_and_syscall_entries_for_the_outer_tgid_and_interval() {
    let interval = Interval {
        start_usec: 1_000_001,
        end_usec: 2_000_000,
    };
    let good = "42 1.000001000: cpu-clock/freq=997,call-graph=dwarf,stack-size=8192/: ffffffff81000000 do_syscall_64 ([kernel.kallsyms])\n\
        42 1.500000000: syscalls:sys_enter_read: 7f00 read (/usr/lib/libc.so.6)\n\
        42 1.600000000: syscalls:sys_enter_read/call-graph=no/: 7f00 read (/usr/lib/libc.so.6)\n\
        42 2.000000000: cpu-clock/freq=997,call-graph=dwarf,stack-size=8192/: 55aa [unknown] (/binary)\n\
        42 1.700000000: syscalls:sys_enter_ppoll: 7f10 ppoll (/usr/lib/libc.so.6)\n";
    let counts = kernel_sample_counts(good, 42, &interval).unwrap();
    assert_eq!((counts.cpu_samples, counts.unknown_leaf_samples), (2, 1));
    assert_eq!(counts.syscall_entries.len(), SYSCALLS.len());
    assert_eq!(counts.syscall_entries["read"], 2);
    assert_eq!(counts.syscall_entries["ppoll"], 1);
    assert_eq!(counts.syscall_entries.values().sum::<usize>(), 3);
    let cpu = "cpu-clock/freq=997,call-graph=dwarf,stack-size=8192/:";
    for bad in [
        String::new(),
        good.replace("42 1.5", "1 1.5"),
        good.replace("1.000001000:", "1.000000999:"),
        good.replace("2.000000000:", "2.000000001:"),
        good.replace("1.5000", "+1.5000"),
        good.replace("sys_enter_ppoll", "sys_enter_getpid"),
        good.replace(cpu, "cpu-clock:u:"),
        good.replace(" 55aa [unknown] (/binary)", ""),
        good.lines()
            .filter(|l| !l.contains(cpu))
            .collect::<Vec<_>>()
            .join("\n"),
    ] {
        assert!(kernel_sample_counts(&bad, 42, &interval).is_err(), "{bad}");
    }
    assert!(kernel_sample_counts(good, 0, &interval).is_err());
    let interval = Interval {
        start_usec: 2,
        end_usec: 1,
    };
    assert!(kernel_sample_counts(good, 42, &interval).is_err());
}

#[test]
fn kernel_script_requires_a_process_and_disables_inline_expansion() {
    let interval = Interval {
        start_usec: 1_000_001,
        end_usec: 2_000_000,
    };
    let args = kernel_script_arguments(Path::new("/tmp/perf.data"), 42, &interval).unwrap();
    for required in [
        "script",
        "--no-inline",
        "-G",
        "--ns",
        "--pid=42",
        "--time=1.000001,2.000000",
    ] {
        assert!(args.iter().any(|arg| arg == required), "{required}");
    }
    assert!(kernel_script_arguments(Path::new("/unused"), 0, &interval).is_err());
}

fn clock_usec() -> u64 {
    let t = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    t.tv_sec as u64 * 1_000_000 + t.tv_nsec as u64 / 1000
}

#[test]
#[ignore = "device-free local perf kernel/tracepoint control; requires readable tracefs metadata"]
fn perf_records_kernel_cpu_and_listed_syscall_entries_for_one_tgid() {
    let root = std::env::temp_dir().join(format!(
        "t331-kernel-profile-control-{}",
        std::process::id()
    ));
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
    // The mode's own event arguments, ahead of the runner command.
    let record = kernel_record_arguments(&data, Path::new("/unused"), &identity, "sha", &root);
    let separator = record.iter().position(|arg| arg == "--").unwrap();
    let start = clock_usec();
    let captured = std::process::Command::new(perf)
        .args(&record[..=separator])
        .args(["/bin/sh", "-c"])
        .arg(format!(
            "echo $$ > {}; i=0; while [ $i -lt 300000 ]; do i=$((i+1)); done",
            identity.display()
        ))
        .output()
        .unwrap();
    let end = clock_usec();
    assert!(
        captured.status.success(),
        "{}",
        String::from_utf8_lossy(&captured.stderr)
    );
    let pid = std::fs::read_to_string(&identity)
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    let interval = Interval {
        start_usec: start,
        end_usec: end,
    };
    let run = |args: Vec<String>| {
        let output = std::process::Command::new(perf)
            .env("LC_ALL", "C")
            .args(&args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        (
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    };
    check_evlist(&run(evlist_arguments(&data)).0).unwrap();
    let (report, report_stderr) = run(report_arguments(&data, pid, &interval).unwrap());
    check_losses(&report).unwrap();
    check_lost_chunks(&report_stderr).unwrap();
    let (samples, script_stderr) = run(kernel_script_arguments(&data, pid, &interval).unwrap());
    check_lost_chunks(&script_stderr).unwrap();
    let counts = kernel_sample_counts(&samples, pid, &interval).unwrap();
    check_report_samples(cpu_report_section(&report).unwrap(), counts.cpu_samples).unwrap();
    // The shell itself opens and writes its identity file.
    assert!(counts.syscall_entries["openat"] >= 1);
    assert!(counts.syscall_entries["write"] >= 1);
}
