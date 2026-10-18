//! Kernel readability control, not native Session or binding-uniqueness proof.
//! Uses Sophia's protection flags with a sleep payload and no device grants.
use std::{
    fs, io,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
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
    let mut supervisor = ChildGuard(
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
    );
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
