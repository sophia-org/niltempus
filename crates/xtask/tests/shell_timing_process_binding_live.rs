//! Kernel readability control, not native Session or binding-uniqueness proof.
//! Uses Sophia's protection flags with a sleep payload and no device grants.
use std::{
    fs, io,
    os::unix::{
        fs::{DirBuilderExt, MetadataExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

// Both controls inspect descendants of this test binary or its cargo parent.
// Serializing avoids mistaking the other control's sandbox for a candidate.
static LIVE_TREE: Mutex<()> = Mutex::new(());

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct WrapperGuard(Child);
impl Drop for WrapperGuard {
    fn drop(&mut self) {
        if let Some(pid) = rustix::process::Pid::from_raw(self.0.id() as i32) {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
        }
        let _ = self.0.wait();
    }
}

fn helper(name: &str) -> ChildGuard {
    ChildGuard(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

fn hold(dir: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !dir.join("release").exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn wrapper_helper() {
    let Some(dir) = std::env::var_os("T338_WRAPPER_FIXTURE") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let _guard = helper("guard_payload");
    let _host = helper("host_payload");
    hold(&dir);
}

#[test]
fn guard_payload() {
    let Some(dir) = std::env::var_os("T338_WRAPPER_FIXTURE") else {
        return;
    };
    let dir = PathBuf::from(dir);
    fs::write(dir.join("guard"), std::process::id().to_string()).unwrap();
    hold(&dir);
}

#[test]
fn host_payload() {
    let Some(dir) = std::env::var_os("T338_WRAPPER_FIXTURE") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let _peer = protected_child();
    fs::write(dir.join("host"), std::process::id().to_string()).unwrap();
    hold(&dir);
}

#[test]
#[ignore = "explicit device-free live wrapper and same-image helper control"]
fn same_image_guard_sibling_is_not_the_measured_session() {
    let _tree = LIVE_TREE.lock().unwrap();
    use xtask::shell_timing_environment_capture::CaptureRoots;
    use xtask::shell_timing_process_binding::{
        Binder, BindingRecheck, Executables, ResolveOutcome,
    };
    let dir =
        Scratch(std::env::temp_dir().join(format!("t338-binding-wrapper-{}", std::process::id())));
    fs::DirBuilder::new().mode(0o700).create(&dir.0).unwrap();
    let executable = std::env::current_exe().unwrap();
    let mut wrapper = WrapperGuard(
        Command::new(&executable)
            .args(["--exact", "wrapper_helper", "--nocapture"])
            .env("T338_WRAPPER_FIXTURE", &dir.0)
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let argv = vec![
        executable.display().to_string(),
        "--exact".into(),
        "host_payload".into(),
        "--nocapture".into(),
    ];
    let roots = CaptureRoots::system();
    let binder = Binder::new(&roots, std::process::id());
    let deadline = Instant::now() + Duration::from_secs(5);
    let binding = loop {
        assert!(Instant::now() < deadline, "wrapper did not bind");
        assert!(wrapper.0.try_wait().unwrap().is_none(), "wrapper exited");
        if dir.0.join("guard").exists() && dir.0.join("host").exists() {
            match binder
                .resolve(
                    wrapper.0.id(),
                    Executables {
                        host: &executable,
                        host_argv: &argv,
                        supervisor: Path::new("/usr/bin/bwrap"),
                        peer: Path::new("/usr/bin/sleep"),
                    },
                )
                .unwrap()
            {
                ResolveOutcome::Bound(binding) => break binding,
                ResolveOutcome::Pending { .. } => {}
            }
        }
        thread::sleep(Duration::from_millis(5));
    };
    let guard: u32 = fs::read_to_string(dir.0.join("guard"))
        .unwrap()
        .parse()
        .unwrap();
    let host: u32 = fs::read_to_string(dir.0.join("host"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(binding.host.pid, host);
    assert_ne!(host, guard);
    assert_eq!(
        identity(Path::new(&format!("/proc/{guard}/exe"))).unwrap(),
        identity(&executable).unwrap()
    );
    let guard_stat = fs::read_to_string(format!("/proc/{guard}/stat")).unwrap();
    let guard_parent: u32 = guard_stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(guard_parent, wrapper.0.id());
    assert_eq!(
        binding
            .chain
            .iter()
            .find(|node| node.identity.pid == host)
            .unwrap()
            .parent_pid,
        wrapper.0.id()
    );
    assert_eq!(binding.host_argv, argv);
    assert_eq!(binder.recheck(&binding).unwrap(), BindingRecheck::Bound);
    fs::write(dir.0.join("release"), b"done").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = wrapper.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "wrapper cleanup timed out");
        thread::sleep(Duration::from_millis(5));
    }
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn identity(path: &Path) -> io::Result<(u64, u64)> {
    let meta = fs::metadata(path)?;
    Ok((meta.dev(), meta.ino()))
}

fn protected_child() -> ChildGuard {
    ChildGuard(
        Command::new("/usr/bin/bwrap")
            .args([
                "--unshare-user",
                "--unshare-ipc",
                "--unshare-pid",
                "--unshare-net",
                "--unshare-uts",
                "--unshare-cgroup",
                "--disable-userns",
                "--assert-userns-disabled",
                "--new-session",
                "--die-with-parent",
                "--as-pid-1",
                "--cap-drop",
                "ALL",
                "--hostname",
                "sophia-client",
                "--clearenv",
                "--ro-bind",
                "/usr",
                "/usr",
                "--symlink",
                "usr/bin",
                "/bin",
                "--symlink",
                "usr/lib",
                "/lib",
                "--symlink",
                "usr/lib",
                "/lib64",
                "--dir",
                "/etc",
                "--ro-bind",
                "/etc/ld.so.cache",
                "/etc/ld.so.cache",
                "--proc",
                "/proc",
                "--dev",
                "/dev",
                "--tmpfs",
                "/tmp",
                "--dir",
                "/run",
                "--dir",
                "/home",
                "--",
                "/usr/bin/sleep",
                "10",
            ])
            .stdin(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

#[test]
fn nondumpable_child() {
    let Some(ready) = std::env::var_os("T338_BINDING_READY") else {
        return;
    };
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable).unwrap();
    fs::write(ready, b"ready").unwrap();
    // Independent lifetime bound even if the controlling test exits unexpectedly.
    thread::sleep(Duration::from_secs(10));
}

#[test]
#[ignore = "explicit device-free control requiring unprivileged bubblewrap namespaces"]
fn protected_peer_is_readable_but_plain_nondumpable_child_is_not() {
    let _tree = LIVE_TREE.lock().unwrap();
    let scratch = Scratch(
        std::env::temp_dir().join(format!("t338-binding-readability-{}", std::process::id())),
    );
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&scratch.0)
        .unwrap();
    let ready = scratch.0.join("ready");
    let mut opaque = ChildGuard(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "nondumpable_child", "--nocapture"])
            .env("T338_BINDING_READY", &ready)
            .stdin(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut supervisor = protected_child();
    let expected_peer = identity(Path::new("/usr/bin/sleep")).unwrap();
    let supervisor_pid = supervisor.0.id();
    let children = format!("/proc/{supervisor_pid}/task/{supervisor_pid}/children");
    let deadline = Instant::now() + Duration::from_secs(5);
    let peer = loop {
        assert!(
            Instant::now() < deadline,
            "children did not reach exec/readiness"
        );
        assert!(
            supervisor.0.try_wait().unwrap().is_none(),
            "bubblewrap exited"
        );
        assert!(
            opaque.0.try_wait().unwrap().is_none(),
            "opaque child exited"
        );
        let pids = fs::read_to_string(&children).unwrap();
        let peers: Vec<u32> = pids
            .split_whitespace()
            .map(|pid| pid.parse::<u32>().unwrap())
            .filter(|pid| {
                identity(Path::new(&format!("/proc/{pid}/exe"))).is_ok_and(|id| id == expected_peer)
            })
            .collect();
        if ready.exists() && peers.len() == 1 {
            break peers[0];
        }
        thread::sleep(Duration::from_millis(5));
    };

    let time = fs::read_link("/proc/self/ns/time").unwrap();
    assert_eq!(time, PathBuf::from("time:[4026531834]"));
    // PID 1 may be an inaccessible host init, or our readable namespace init.
    // Neither case is used as the clock-domain authority.
    match fs::read_link("/proc/1/ns/time") {
        Ok(reference) => {
            assert_eq!(reference, time);
            eprintln!("PID 1 time namespace readable (not used as reference)");
        }
        Err(error) => {
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            eprintln!("PID 1 time namespace denied (not used as reference)");
        }
    }
    for (pid, program) in [(supervisor_pid, "/usr/bin/bwrap"), (peer, "/usr/bin/sleep")] {
        let exe = format!("/proc/{pid}/exe");
        let link = fs::read_link(&exe).unwrap();
        assert_eq!(
            identity(Path::new(&exe)).unwrap(),
            identity(Path::new(program)).unwrap()
        );
        assert_eq!(fs::read_link(format!("/proc/{pid}/ns/time")).unwrap(), time);
        eprintln!(
            "readable pid={pid} exe={} time={}",
            link.display(),
            time.display()
        );
    }
    for suffix in ["exe", "ns/time"] {
        let path = format!("/proc/{}/{suffix}", opaque.0.id());
        assert_eq!(
            fs::read_link(&path).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        eprintln!("denied {path}");
    }
    assert!(
        !fs::read_to_string(format!("/proc/{}/stat", opaque.0.id()))
            .unwrap()
            .is_empty()
    );
    // Cargo is the supplied wrapper for this control; the test binary is the
    // host. This exercises live proc reads, not a native Session or launch grant.
    use xtask::shell_timing_environment_capture::CaptureRoots;
    use xtask::shell_timing_process_binding::{
        Binder, BindingRecheck, Executables, ResolveOutcome,
    };
    let roots = CaptureRoots::system();
    let binder = Binder::new(&roots, std::process::id());
    let host_executable = std::env::current_exe().unwrap();
    let wrapper = rustix::process::getppid().unwrap().as_raw_nonzero().get() as u32;
    let ResolveOutcome::Bound(binding) = binder
        .resolve(
            wrapper,
            Executables {
                host: &host_executable,
                host_argv: &std::env::args().collect::<Vec<_>>(),
                supervisor: Path::new("/usr/bin/bwrap"),
                peer: Path::new("/usr/bin/sleep"),
            },
        )
        .unwrap()
    else {
        panic!("ready protected peer did not bind");
    };
    assert_eq!(binding.host.pid, std::process::id());
    assert_eq!(binding.supervisor.pid, supervisor_pid);
    assert_eq!(binding.peer.pid, peer);
    assert!(
        binding
            .unexamined
            .iter()
            .any(|node| node.identity.pid == opaque.0.id())
    );
    assert_eq!(binder.recheck(&binding).unwrap(), BindingRecheck::Bound);
}
