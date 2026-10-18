//! Environment capture over fixture filesystem roots. No live `/proc` or
//! `/sys` reads, no devices, no processes launched.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use xtask::shell_timing_environment::{
    Capture, Expect, Interval, SourceKind, SourceText, Vrr, environment_record,
};
use xtask::shell_timing_environment_capture::{CaptureRoots, Capturer, ProcessRecheck};

const HOST: u32 = 4100;
const PEER: u32 = 4200;
const HOST_START: u64 = 111_111;
const PEER_START: u64 = 222_222;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    capturer: Capturer,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A 52-field stat line; field 22 (start_time) set to `start`.
fn stat(pid: u32, comm: &str, start: u64) -> String {
    stat_with_state(pid, comm, start, "S")
}

fn stat_with_state(pid: u32, comm: &str, start: u64, state: &str) -> String {
    let mut fields: Vec<String> = (3..=52).map(|_| "0".to_string()).collect();
    fields[0] = state.into();
    fields[22 - 3] = start.to_string();
    format!("{pid} ({comm}) {}\n", fields.join(" "))
}

fn status(pid: u32, cpus: &str) -> String {
    format!(
        "Name:\tx\nUmask:\t0022\nState:\tS (sleeping)\nTgid:\t{pid}\nPid:\t{pid}\n\
         Cpus_allowed:\t3\nCpus_allowed_list:\t{cpus}\n"
    )
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn fixture() -> Fixture {
    let id = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
    let root =
        std::env::temp_dir().join(format!("shell-timing-capture-{}-{id}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let proc_root = root.join("proc");
    let sys_root = root.join("sys");
    write(&proc_root.join("sys/kernel/osrelease"), "6.18.55_1\n");
    write(
        &proc_root.join("version"),
        "Linux version 6.18.55_1 (builder) (gcc 14.2) #1 SMP\n",
    );
    write(&sys_root.join("devices/system/cpu/online"), "0-1\n");
    for cpu in 0..2 {
        write(
            &sys_root.join(format!(
                "devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor"
            )),
            "performance\n",
        );
    }
    // Driver symlink whose basename is the kernel driver name.
    let device = sys_root.join("class/drm/card1/device");
    fs::create_dir_all(&device).unwrap();
    fs::create_dir_all(sys_root.join("drivers/amdgpu")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("../../../drivers/amdgpu", device.join("driver")).unwrap();
    write(
        &proc_root.join(format!("{HOST}/stat")),
        &stat(HOST, "sophia-host", HOST_START),
    );
    write(
        &proc_root.join(format!("{HOST}/status")),
        &status(HOST, "0-1"),
    );
    write(
        &proc_root.join(format!("{PEER}/stat")),
        &stat(PEER, "timing peer", PEER_START),
    );
    write(
        &proc_root.join(format!("{PEER}/status")),
        &status(PEER, "0-1"),
    );
    let capturer = Capturer::new(CaptureRoots::new(proc_root, sys_root));
    Fixture { root, capturer }
}

fn kinds(sources: &[SourceText]) -> Vec<SourceKind> {
    sources.iter().map(|s| s.kind.clone()).collect()
}

#[test]
fn static_capture_covers_kernel_cpus_governors_and_driver() {
    let fx = fixture();
    let sources = fx.capturer.capture_static("card1").unwrap();
    assert_eq!(
        kinds(&sources),
        [
            SourceKind::Osrelease,
            SourceKind::ProcVersion,
            SourceKind::CpuOnline,
            SourceKind::Governor { cpu: 0 },
            SourceKind::Governor { cpu: 1 },
            SourceKind::DrmDriver {
                card: "card1".into()
            },
        ]
    );
    let driver = sources.last().unwrap();
    assert_eq!(driver.text, "amdgpu\n");
    assert!(driver.origin.ends_with("class/drm/card1/device/driver"));
    assert!(
        sources
            .windows(2)
            .all(|pair| pair[0].captured_usec <= pair[1].captured_usec),
        "captured_usec must not go backwards"
    );
    assert!(sources.iter().all(|s| s.captured_usec > 0));
}

#[test]
fn static_samples_can_be_compared_across_a_change() {
    let fx = fixture();
    let before = fx.capturer.capture_static("card1").unwrap();
    write(
        &fx.root
            .join("sys/devices/system/cpu/cpu1/cpufreq/scaling_governor"),
        "powersave\n",
    );
    let after = fx.capturer.capture_static("card1").unwrap();
    assert_eq!(before[4].text, "performance\n");
    assert_eq!(after[4].text, "powersave\n");
    // Two samples disagreeing is for the runner to judge; nothing here
    // claims the interval between them was observed.
}

#[test]
fn measurement_capture_binds_host_and_peer_identities() {
    let fx = fixture();
    let measured = fx
        .capturer
        .capture_measurement("card1", HOST, PEER)
        .unwrap();
    assert_eq!(
        (measured.host.pid, measured.host.start_time_ticks),
        (HOST, HOST_START)
    );
    assert_eq!(
        (measured.peer.pid, measured.peer.start_time_ticks),
        (PEER, PEER_START)
    );
    assert!(kinds(&measured.sources).contains(&SourceKind::ProcStat { pid: PEER }));
    // Bracket evidence is preserved as a sidecar; the M5 source list keeps
    // exactly one stat (and one status) source per pid.
    assert_eq!(measured.closing_stats.len(), 2);
    assert_eq!(
        measured.closing_stats[0].kind,
        SourceKind::ProcStat { pid: HOST }
    );
    assert!(measured.closing_stats.iter().all(|s| s.captured_usec > 0));
    for pid in [HOST, PEER] {
        let stats = measured
            .sources
            .iter()
            .filter(|s| s.kind == SourceKind::ProcStat { pid })
            .count();
        assert_eq!(stats, 1, "one stat source per pid in the source list");
    }
    let captured = fx.capturer.capture_process(HOST).unwrap();
    assert_eq!(
        captured.closing_stat.kind,
        SourceKind::ProcStat { pid: HOST }
    );
    assert!(captured.closing_stat.text.contains(&HOST_START.to_string()));
}

#[test]
fn zombie_and_dead_processes_do_not_qualify_as_alive() {
    // Same identity, zombie state: capture refuses, recheck reports ended.
    let fx = fixture();
    let captured = fx.capturer.capture_process(HOST).unwrap();
    write(
        &fx.root.join(format!("proc/{HOST}/stat")),
        &stat_with_state(HOST, "sophia-host", HOST_START, "Z"),
    );
    let err = fx.capturer.capture_process(HOST).unwrap_err();
    assert!(err.contains("ended state"), "{err}");
    assert_eq!(
        fx.capturer.recheck(&captured.identity).unwrap(),
        ProcessRecheck::Exited
    );
    // Reused pid in a zombie state is reuse, never the original's exit.
    write(
        &fx.root.join(format!("proc/{HOST}/stat")),
        &stat_with_state(HOST, "sophia-host", HOST_START + 1, "Z"),
    );
    let err = fx.capturer.recheck(&captured.identity).unwrap_err();
    assert!(err.contains("reused"), "{err}");
    // Dead state at capture is refused too.
    let fx = fixture();
    write(
        &fx.root.join(format!("proc/{HOST}/stat")),
        &stat_with_state(HOST, "sophia-host", HOST_START, "X"),
    );
    assert!(fx.capturer.capture_process(HOST).is_err());
}

#[test]
fn captured_sources_satisfy_the_environment_record() {
    let fx = fixture();
    let measured = fx
        .capturer
        .capture_measurement("card1", HOST, PEER)
        .unwrap();
    let stamp = measured.sources[0].captured_usec;
    let log = format!(
        "sophia_shell_environment_output schema=1 pid={HOST} output=3 card=card1 \
         connector=DP-2 width=2560 height=1440 refresh_millihz=120000 timing_refresh_millihz=120014 scale_milli=1000 \
         vrr=off monotonic_usec={stamp}\n\
         sophia_shell_environment_renderer schema=1 pid={HOST} output=3 card=card1 \
         driver=amdgpu vendor=Mesa renderer=AMD%20Radeon version=4.6 monotonic_usec={stamp}\n"
    );
    let mut capture: Capture = measured.into_capture();
    capture.sources.push(SourceText {
        kind: SourceKind::HostLog,
        origin: "host.log".into(),
        captured_usec: stamp,
        text: log,
    });
    let expect = Expect {
        output: 3,
        card: "card1".into(),
        connector: "DP-2".into(),
        width: 2560,
        height: 1440,
        refresh_millihz: 120000,
        scale_milli: 1000,
        vrr: Vrr::Off,
        host_pid: HOST,
        peer_pid: PEER,
        capture: Interval {
            start_usec: 0,
            end_usec: u64::MAX - 1,
        },
        measurement: Interval {
            start_usec: 0,
            end_usec: u64::MAX - 1,
        },
    };
    let env = environment_record(&capture, &expect).unwrap();
    assert_eq!(env.verdict, "environment_complete");
    assert_eq!(env.drm_driver.value, "amdgpu");
    assert_eq!(env.governors.len(), 2);
}

#[test]
fn recheck_distinguishes_alive_exited_reused_and_malformed() {
    let fx = fixture();
    let captured = fx.capturer.capture_process(HOST).unwrap();
    assert_eq!(
        fx.capturer.recheck(&captured.identity).unwrap(),
        ProcessRecheck::Alive(captured.identity)
    );
    // Reused pid: same path, different start time.
    write(
        &fx.root.join(format!("proc/{HOST}/stat")),
        &stat(HOST, "sophia-host", HOST_START + 1),
    );
    let err = fx.capturer.recheck(&captured.identity).unwrap_err();
    assert!(err.contains("reused"), "{err}");
    // Malformed is an error, never a clean exit.
    write(&fx.root.join(format!("proc/{HOST}/stat")), "junk\n");
    let err = fx.capturer.recheck(&captured.identity).unwrap_err();
    assert!(!err.is_empty());
    // Gone is the one clean Exited outcome.
    fs::remove_file(fx.root.join(format!("proc/{HOST}/stat"))).unwrap();
    assert_eq!(
        fx.capturer.recheck(&captured.identity).unwrap(),
        ProcessRecheck::Exited
    );
}

#[test]
fn unknown_stat_state_is_malformed_never_alive_or_exited() {
    let fx = fixture();
    let captured = fx.capturer.capture_process(HOST).unwrap();
    for bad in ["?", "1"] {
        write(
            &fx.root.join(format!("proc/{HOST}/stat")),
            &stat_with_state(HOST, "sophia-host", HOST_START, bad),
        );
        let err = fx.capturer.capture_process(HOST).unwrap_err();
        assert!(err.contains("unknown state"), "{err}");
        let err = fx.capturer.recheck(&captured.identity).unwrap_err();
        assert!(err.contains("unknown state"), "{err}");
    }
}

#[test]
fn wrong_pid_malformed_and_missing_sources_are_refused() {
    let fx = fixture();
    // Stat content names a different pid than the directory requested.
    write(
        &fx.root.join(format!("proc/{HOST}/stat")),
        &stat(HOST + 9, "sophia-host", HOST_START),
    );
    let err = fx.capturer.capture_process(HOST).unwrap_err();
    assert!(err.contains("not requested pid"), "{err}");
    // Missing governor for an online CPU.
    let fx = fixture();
    fs::remove_file(
        fx.root
            .join("sys/devices/system/cpu/cpu1/cpufreq/scaling_governor"),
    )
    .unwrap();
    assert!(fx.capturer.capture_static("card1").is_err());
    // Same host and peer pid.
    let fx = fixture();
    assert!(
        fx.capturer
            .capture_measurement("card1", HOST, HOST)
            .is_err()
    );
    assert!(fx.capturer.capture_process(0).is_err());
}

#[test]
fn oversized_sources_are_refused_not_truncated() {
    let fx = fixture();
    let big = "x".repeat(70_000);
    write(&fx.root.join(format!("proc/{HOST}/status")), &big);
    let err = fx.capturer.capture_process(HOST).unwrap_err();
    assert!(err.contains("exceeds"), "{err}");
    let fx = fixture();
    let big = "x".repeat(5_000);
    write(&fx.root.join("proc/sys/kernel/osrelease"), &big);
    assert!(fx.capturer.capture_static("card1").is_err());
}

#[test]
fn start_time_is_read_past_parenthesised_names() {
    let fx = fixture();
    write(
        &fx.root.join(format!("proc/{PEER}/stat")),
        &stat(PEER, "name with ) parens", PEER_START),
    );
    let captured = fx.capturer.capture_process(PEER).unwrap();
    assert_eq!(captured.identity.start_time_ticks, PEER_START);
}
