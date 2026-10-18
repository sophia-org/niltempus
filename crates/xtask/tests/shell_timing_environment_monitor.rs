//! Environment monitor over fixture proc/sys roots and a real local
//! datagram socket. The fixture host pid is this test process, so the
//! sender path needs no spawned host; one bounded child covers the
//! wrong-pid and exited-sender senders. No live `/proc`, no hardware.
use std::{
    fs,
    os::unix::{
        fs::{DirBuilderExt, symlink},
        net::UnixDatagram,
    },
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use xtask::shell_timing_environment_capture::{CaptureRoots, Capturer};
use xtask::shell_timing_environment_monitor::{
    EnvironmentMonitor, MonitorConfig, MonitorOutcome, MonitorTurn,
};
use xtask::shell_timing_process_binding::Binder;

const REF: u32 = 1;
const WRAPPER: u32 = 100;
const SHELL: u32 = 110;
const PEER: u32 = 300;
const SUPERVISOR: u32 = 250;
const CAPTURER: u32 = 900;
const NS: u64 = 0xEFFFFFFA;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

fn host() -> u32 {
    std::process::id()
}

fn host_argv() -> Vec<String> {
    vec![
        std::env::current_exe().unwrap().display().to_string(),
        "session".into(),
        "run".into(),
    ]
}

fn stat(pid: u32, ppid: u32, start: u64, state: &str) -> String {
    let mut fields: Vec<String> = (3..=52).map(|_| "0".to_string()).collect();
    fields[0] = state.into();
    fields[1] = ppid.to_string();
    fields[22 - 3] = start.to_string();
    format!("{pid} (proc-{pid}) {}\n", fields.join(" "))
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn link(target: &Path, path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let _ = fs::remove_file(path);
    symlink(target, path).unwrap();
}

struct Fixture {
    root: PathBuf,
    proc_root: PathBuf,
    socket: PathBuf,
    peer_file: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    fn process(&self, pid: u32, ppid: u32, start: u64, exe: Option<&Path>, children: &[u32]) {
        let dir = self.proc_root.join(pid.to_string());
        write(&dir.join("stat"), &stat(pid, ppid, start, "S"));
        let text = children
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        write(&dir.join("task/1/children"), &format!("{text}\n"));
        if let Some(exe) = exe {
            link(exe, &dir.join("exe"));
        }
        link(&PathBuf::from(format!("time:[{NS}]")), &dir.join("ns/time"));
    }

    fn link_host_exe(&self) {
        link(
            &std::env::current_exe().unwrap(),
            &self.proc_root.join(format!("{}/exe", host())),
        );
    }

    fn monitor(&self) -> EnvironmentMonitor {
        let roots = CaptureRoots::new(self.proc_root.clone(), self.root.join("sys"));
        EnvironmentMonitor::new(
            MonitorConfig {
                host_executable: std::env::current_exe().unwrap(),
                host_argv: host_argv(),
                supervisor_executable: PathBuf::from("/usr/bin/bwrap"),
                peer_executable: self.peer_file.clone(),
                card: "card1".into(),
                environment_socket: self.socket.clone(),
            },
            Binder::new(&roots, CAPTURER),
            Capturer::new(roots),
        )
        .unwrap()
    }
}

fn fixture(host_ready: bool) -> Fixture {
    let id = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
    let root =
        std::env::temp_dir().join(format!("shell-timing-monitor-{}-{id}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let proc_root = root.join("proc");
    let sys_root = root.join("sys");
    let run_dir = root.join("run");
    fs::create_dir_all(&root).unwrap();
    fs::DirBuilder::new().mode(0o700).create(&run_dir).unwrap();
    let peer_file = root.join("bin/peer-bin");
    write(&peer_file, "peer executable fixture\n");
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
    let device = sys_root.join("class/drm/card1/device");
    fs::create_dir_all(&device).unwrap();
    fs::create_dir_all(sys_root.join("drivers/amdgpu")).unwrap();
    symlink("../../../drivers/amdgpu", device.join("driver")).unwrap();
    let fx = Fixture {
        root,
        proc_root,
        socket: run_dir.join("environment.sock"),
        peer_file,
    };
    fx.process(REF, 0, 10, None, &[]);
    fx.process(CAPTURER, REF, 9000, None, &[]);
    write(
        &fx.proc_root.join(format!("{CAPTURER}/timens_offsets")),
        "monotonic 0 0\nboottime 0 0\n",
    );
    fx.process(WRAPPER, CAPTURER, 1000, None, &[SHELL]);
    fx.process(SHELL, WRAPPER, 1100, None, &[host()]);
    let host_exe = std::env::current_exe().unwrap();
    fx.process(
        host(),
        SHELL,
        4242,
        if host_ready { Some(&host_exe) } else { None },
        &[SUPERVISOR],
    );
    write(
        &fx.proc_root.join(format!("{}/cmdline", host())),
        &(host_argv().join("\0") + "\0"),
    );
    fx.process(
        SUPERVISOR,
        host(),
        2500,
        Some(Path::new("/usr/bin/bwrap")),
        &[PEER],
    );
    fx.process(PEER, SUPERVISOR, 3000, Some(&fx.peer_file.clone()), &[]);
    let uid = rustix::process::getuid().as_raw();
    let gid = rustix::process::getgid().as_raw();
    write(
        &fx.proc_root.join(format!("{}/status", host())),
        &format!(
            "Name:\thost\nPid:\t{}\nUid:\t{uid} {uid} {uid} {uid}\nGid:\t{gid} {gid} {gid} {gid}\n\
             Cpus_allowed_list:\t0-1\n",
            host()
        ),
    );
    write(
        &fx.proc_root.join(format!("{PEER}/status")),
        "Name:\tpeer\nPid:\t300\nUid:\t1000 1000 1000 1000\nGid:\t1000 1000 1000 1000\n\
         Cpus_allowed_list:\t0-1\n",
    );
    fx
}

fn send_record(path: &Path, bytes: &[u8]) {
    use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg};
    use std::os::fd::AsFd;
    let sender = UnixDatagram::unbound().unwrap();
    sender.connect(path).unwrap();
    let pidfd = rustix::process::pidfd_open(
        rustix::process::getpid(),
        rustix::process::PidfdFlags::empty(),
    )
    .unwrap();
    let rights = [pidfd.as_fd()];
    let mut space = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    assert!(control.push(SendAncillaryMessage::ScmRights(&rights)));
    sendmsg(
        &sender,
        &[std::io::IoSlice::new(bytes)],
        &mut control,
        SendFlags::NOSIGNAL,
    )
    .unwrap();
}

#[test]
fn monitor_sender_child() {
    let Ok(path) = std::env::var("T338_MONITOR_SOCKET") else {
        return;
    };
    send_record(
        Path::new(&path),
        std::env::var("T338_MONITOR_RECORD").unwrap().as_bytes(),
    );
    fs::write(std::env::var("T338_MONITOR_SENT").unwrap(), b"sent").unwrap();
    let release = PathBuf::from(std::env::var("T338_MONITOR_RELEASE").unwrap());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !release.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_sender(fx: &Fixture) -> ChildGuard {
    ChildGuard(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "monitor_sender_child", "--test-threads=1"])
            .env("T338_MONITOR_SOCKET", &fx.socket)
            .env(
                "T338_MONITOR_RECORD",
                "sophia_shell_environment_output schema=1",
            )
            .env("T338_MONITOR_SENT", fx.root.join("sent"))
            .env("T338_MONITOR_RELEASE", fx.root.join("release"))
            .spawn()
            .unwrap(),
    )
}

fn wait_exited(child: &mut ChildGuard) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn pending_then_bound_batch_is_accepted_inside_its_bracket() {
    let fx = fixture(false);
    let mut monitor = fx.monitor();
    assert_eq!(monitor.observe(WRAPPER).unwrap(), MonitorTurn::Pending);
    fx.link_host_exe();
    send_record(&fx.socket, b"sophia_shell_environment_output schema=1");
    assert_eq!(monitor.observe(WRAPPER).unwrap(), MonitorTurn::Bound);
    // An empty poll turn adds no batch.
    assert_eq!(monitor.observe(WRAPPER).unwrap(), MonitorTurn::Bound);
    let evidence = monitor.finish();
    assert_eq!(evidence.outcome, MonitorOutcome::Bound);
    assert_eq!(evidence.receipts.len(), 1);
    assert_eq!(evidence.batches.len(), 1);
    let batch = &evidence.batches[0];
    assert_eq!((batch.first_receipt, batch.receipt_count), (0, 1));
    assert!(
        (batch.interval.start_usec..=batch.interval.end_usec)
            .contains(&evidence.receipts[0].received_usec)
    );
    assert_eq!(evidence.binding.as_ref().unwrap().host.pid, host());
    assert_eq!(evidence.host_uid, Some(rustix::process::getuid().as_raw()));
    // Start and end captures are retained as separate source sets.
    let start = evidence.start.as_ref().unwrap();
    assert!(!start.sources.is_empty());
    assert_eq!(start.closing_stats.len(), 2);
    assert!(matches!(
        evidence.end.static_sources,
        Ok(ref sources) if !sources.is_empty()
    ));
    let capture_window = evidence.capture_window.unwrap();
    assert!(capture_window.start_usec <= capture_window.end_usec);
    assert!(evidence.finish_error.is_none());
    assert!(evidence.end.window.is_some());
    let json = serde_json::to_string(&evidence).unwrap();
    let parsed: xtask::shell_timing_environment_monitor::MonitorEvidence =
        serde_json::from_str(&json).unwrap();
    assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
}

#[test]
fn wrong_pid_batch_is_discarded_and_failure_latches() {
    let fx = fixture(true);
    let mut monitor = fx.monitor();
    let mut child = spawn_sender(&fx);
    let sent = fx.root.join("sent");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sent.exists() {
        assert!(Instant::now() < deadline, "sender child never sent");
        std::thread::sleep(Duration::from_millis(1));
    }
    let error = monitor.observe(WRAPPER).unwrap_err();
    assert!(!error.is_empty());
    assert_eq!(monitor.observe(WRAPPER).unwrap(), MonitorTurn::Ended);
    fs::write(fx.root.join("release"), b"done").unwrap();
    wait_exited(&mut child);
    let evidence = monitor.finish();
    assert_eq!(evidence.outcome, MonitorOutcome::Failed);
    assert!(evidence.receipts.is_empty());
    assert!(evidence.batches.is_empty());
}

#[test]
fn queued_record_from_an_exited_sender_is_refused_at_first_drain() {
    let fx = fixture(true);
    let mut monitor = fx.monitor();
    fs::write(fx.root.join("release"), b"exit after send").unwrap();
    let mut child = spawn_sender(&fx);
    wait_exited(&mut child);
    assert!(monitor.observe(WRAPPER).is_err());
    let evidence = monitor.finish();
    assert_eq!(evidence.outcome, MonitorOutcome::Failed);
    assert!(evidence.receipts.is_empty());
}

#[test]
fn host_ending_before_a_batch_discards_it() {
    let fx = fixture(true);
    let mut monitor = fx.monitor();
    assert_eq!(monitor.observe(WRAPPER).unwrap(), MonitorTurn::Bound);
    write(
        &fx.proc_root.join(format!("{}/stat", host())),
        &stat(host(), SHELL, 4242, "Z"),
    );
    send_record(&fx.socket, b"sophia_shell_environment_output schema=1");
    assert_eq!(monitor.observe(WRAPPER).unwrap(), MonitorTurn::Ended);
    let evidence = monitor.finish();
    assert_eq!(evidence.outcome, MonitorOutcome::EndedBeforeAccept);
    assert!(evidence.receipts.is_empty());
    assert!(matches!(
        evidence.end.host_recheck,
        Some(xtask::shell_timing_environment_monitor::RecheckRecord::Exited)
    ));
}

#[test]
fn normal_end_after_evidence_qualifies_with_proven_alive_bound() {
    let fx = fixture(true);
    let mut monitor = fx.monitor();
    send_record(&fx.socket, b"sophia_shell_environment_output schema=1");
    assert_eq!(monitor.observe(WRAPPER).unwrap(), MonitorTurn::Bound);
    // An empty successful turn still refreshes the proven-alive bracket.
    assert_eq!(monitor.observe(WRAPPER).unwrap(), MonitorTurn::Bound);
    // The normal lifecycle ends the host before finish.
    write(
        &fx.proc_root.join(format!("{}/stat", host())),
        &stat(host(), SHELL, 4242, "Z"),
    );
    assert_eq!(monitor.observe(WRAPPER).unwrap(), MonitorTurn::Ended);
    let evidence = monitor.finish();
    assert_eq!(evidence.outcome, MonitorOutcome::EndedAfterEvidence);
    assert_eq!(evidence.receipts.len(), 1);
    assert_eq!(evidence.batches.len(), 1);
    let proven = evidence.last_proven_alive.unwrap();
    assert!(
        proven.interval.start_usec >= evidence.batches[0].interval.start_usec,
        "empty turns refresh the proven-alive opening"
    );
    assert_eq!(proven.host_start_time_ticks, 4242);
    assert!(matches!(
        evidence.end.host_recheck,
        Some(xtask::shell_timing_environment_monitor::RecheckRecord::Exited)
    ));
}

#[test]
fn sticky_receiver_failure_latches_and_finish_retains_accepted_receipts() {
    let fx = fixture(true);
    let mut monitor = fx.monitor();
    send_record(&fx.socket, b"sophia_shell_environment_output schema=1");
    assert_eq!(monitor.observe(WRAPPER).unwrap(), MonitorTurn::Bound);
    UnixDatagram::unbound()
        .unwrap()
        .send_to(b"sophia_shell_environment_output schema=1", &fx.socket)
        .unwrap();
    assert!(monitor.observe(WRAPPER).is_err());
    assert_eq!(monitor.observe(WRAPPER).unwrap(), MonitorTurn::Ended);
    let evidence = monitor.finish();
    assert_eq!(evidence.outcome, MonitorOutcome::Failed);
    assert_eq!(evidence.receipts.len(), 1);
    assert_eq!(evidence.batches.len(), 1);
    assert!(
        evidence.end.static_sources.is_ok(),
        "end capture after failure"
    );
}

fn monotonic_now_usec() -> u64 {
    let clock = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    u64::try_from(i128::from(clock.tv_sec) * 1_000_000 + i128::from(clock.tv_nsec) / 1_000).unwrap()
}

fn campaign_manifest() -> xtask::shell_timing_campaign::Manifest {
    use xtask::shell_timing_campaign::{
        Arm, Condition, EnvironmentTarget, Manifest, Measurement, Variant, Workload,
    };
    let mut arms = Vec::new();
    for condition in [Condition::Idle, Condition::Load] {
        for pair in 1..=3u8 {
            for workload in [Workload::Panel, Workload::Bulk] {
                for measurement in [
                    Measurement::Latency,
                    Measurement::CpuOnly,
                    Measurement::RustGlobal,
                ] {
                    let order = match pair {
                        1 => [Variant::C, Variant::B, Variant::A],
                        2 => [Variant::A, Variant::B, Variant::C],
                        _ => [Variant::B, Variant::C, Variant::A],
                    };
                    for variant in order {
                        let index = arms.len();
                        let (frames, warmup) = if measurement == Measurement::Latency {
                            (1100, 100)
                        } else {
                            (1000, 0)
                        };
                        arms.push(Arm {
                            id: format!("arm-{index}"),
                            condition,
                            pair,
                            workload,
                            measurement,
                            variant,
                            plan_sha256: "a".repeat(64),
                            peer_config_sha256: "b".repeat(64),
                            frames,
                            warmup,
                            runtime_ms: 60_000,
                        });
                    }
                }
            }
        }
    }
    Manifest {
        schema: 2,
        id: "campaign".into(),
        connector: "DP-2".into(),
        width: 1920,
        height: 1080,
        mode_millihz: 120_000,
        vrr: false,
        environment: EnvironmentTarget {
            card: "card1".into(),
            output: 3,
            scale_milli: 1000,
        },
        load_workers: 16,
        quiet_window: "declared quiet window".into(),
        arms,
    }
}

/// Monitor -> provider schema composition with a real monitor, real
/// pidfd datagrams and the fixture proc/sys tree. The fixture
/// bindings and static facts are supplied; the sender credentials
/// and pidfd are real kernel facts about this test process. This is
/// not a hardware or live-host claim.
#[test]
fn monitor_evidence_composes_into_the_m5_record() {
    use xtask::shell_timing_campaign_import::{ArmFiles, ArmProvenance, check_measured_interval};
    use xtask::shell_timing_campaign_provenance::TrustedRunnerProvenance;
    use xtask::shell_timing_environment::{Interval, environment_record};

    let fx = fixture(true);
    let mut monitor = fx.monitor();
    let stamp = monotonic_now_usec();
    send_record(
        &fx.socket,
        format!(
            "sophia_shell_environment_output schema=1 pid={} output=3 card=card1 \
             connector=DP-2 width=1920 height=1080 refresh_millihz=120000 \
             timing_refresh_millihz=119980 scale_milli=1000 vrr=off monotonic_usec={stamp}",
            host()
        )
        .as_bytes(),
    );
    send_record(
        &fx.socket,
        format!(
            "sophia_shell_environment_renderer schema=1 pid={} output=3 card=card1 \
             driver=amdgpu vendor=Mesa renderer=AMD%20Radeon version=4.6 monotonic_usec={stamp}",
            host()
        )
        .as_bytes(),
    );
    assert_eq!(monitor.observe(WRAPPER).unwrap(), MonitorTurn::Bound);
    // The evidence comes only from monitor.finish(); no supplied
    // MonitorEvidence constructor is involved.
    let evidence = monitor.finish();
    assert_eq!(evidence.outcome, MonitorOutcome::Bound);
    assert_eq!(evidence.receipts.len(), 2);

    let manifest = campaign_manifest();
    let arm = manifest.arms[0].clone();
    let provider = TrustedRunnerProvenance::new(manifest).unwrap();
    let current_exe = std::env::current_exe().unwrap();
    let artifact = |path: &Path| serde_json::json!({"path": path.display().to_string(), "sha256": "c".repeat(64)});
    let mut artifacts = std::collections::BTreeMap::new();
    artifacts.insert("sophia".to_string(), artifact(&current_exe));
    artifacts.insert("peer".to_string(), artifact(&fx.peer_file));
    artifacts.insert("runner".to_string(), artifact(&current_exe));
    let plan = serde_json::to_vec(&serde_json::json!({
        "schema": 1,
        "request": {
            "sophia_source": {"path": "/fixture/sophia-source", "commit": "d".repeat(40)},
            "integration_source": {"path": "/fixture/integration", "commit": "e".repeat(40)},
            "artifacts": artifacts,
            "tty": "tty3",
            "display": 1,
            "input_seat": "seat0",
            "runtime_ms": 60000,
            "wire": "files",
            "output": 3,
            "frames": 1100,
            "mode_millihz": 120000,
            "instrumentation": "none",
            "environment_readback": true
        },
        "argv": host_argv()
    }))
    .unwrap();
    let environment_bytes = serde_json::to_vec(&evidence).unwrap();
    let receipt_bytes = serde_json::to_vec(&evidence.receipts).unwrap();
    let files = ArmFiles {
        arm: &arm,
        plan: &plan,
        environment: &environment_bytes,
        environment_receipts: &receipt_bytes,
        load: None,
    };
    let verified = provider.environment(&files).unwrap();
    let record = environment_record(&verified.capture, &verified.expect).unwrap();
    assert_eq!(record.verdict, "environment_complete");
    // The declared measurement is the proven lifetime: a raw span
    // covering it passes, one ending past the proven-alive opening
    // refuses.
    let measured = verified.expect.measurement;
    check_measured_interval(&verified, measured).unwrap();
    assert!(
        check_measured_interval(
            &verified,
            Interval {
                start_usec: measured.start_usec,
                end_usec: measured.end_usec + 1,
            },
        )
        .is_err()
    );
    // A tampered sidecar receipt no longer matches the monitor
    // evidence and cannot qualify.
    let mut tampered = evidence.receipts.clone();
    tampered[0].record.push_str(" forged=1");
    let tampered_bytes = serde_json::to_vec(&tampered).unwrap();
    let files = ArmFiles {
        environment_receipts: &tampered_bytes,
        ..files
    };
    assert!(provider.environment(&files).is_err());
}

#[test]
fn never_bound_cannot_qualify_but_finish_still_captures_the_end() {
    let fx = fixture(false);
    let mut monitor = fx.monitor();
    assert_eq!(monitor.observe(WRAPPER).unwrap(), MonitorTurn::Pending);
    assert_eq!(monitor.observe(WRAPPER).unwrap(), MonitorTurn::Pending);
    let evidence = monitor.finish();
    assert_eq!(evidence.outcome, MonitorOutcome::NeverBound);
    assert!(evidence.start.is_none());
    assert!(evidence.receipts.is_empty());
    assert!(evidence.end.static_sources.is_ok());
}
