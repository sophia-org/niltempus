//! Supervised process binding over fixture proc roots. No live `/proc`
//! reads, no processes launched, no hardware.
use std::{
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use xtask::shell_timing_environment_capture::CaptureRoots;
use xtask::shell_timing_process_binding::{
    Binder, BindingRecheck, MAX_CHILDREN_PER_PROCESS, MAX_TASKS_PER_PROCESS, PendingStage,
    ResolveOutcome,
};

const REF: u32 = 1;
const WRAPPER: u32 = 100;
const SHELL: u32 = 110;
const HOST: u32 = 200;
const PEER: u32 = 300;
const CAPTURER: u32 = 900;
const NS: u64 = 4_026_531_840;
const OFFSETS: &str = "monotonic 0 0\nboottime 0 0\n";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    proc_root: PathBuf,
    binder: Binder,
    host_file: PathBuf,
    peer_file: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A 52-field stat line with explicit ppid, state and start time.
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

impl Fixture {
    fn process(&self, pid: u32, ppid: u32, start: u64, exe: Option<&Path>, children: &[u32]) {
        self.process_state(pid, ppid, start, "S", exe, children);
    }

    fn process_state(
        &self,
        pid: u32,
        ppid: u32,
        start: u64,
        state: &str,
        exe: Option<&Path>,
        children: &[u32],
    ) {
        let dir = self.proc_root.join(pid.to_string());
        write(&dir.join("stat"), &stat(pid, ppid, start, state));
        self.set_children(pid, children);
        if let Some(exe) = exe {
            link(exe, &dir.join("exe"));
        }
        self.set_ns(pid, NS);
    }

    fn set_children(&self, pid: u32, children: &[u32]) {
        let text = children
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        write(
            &self.proc_root.join(format!("{pid}/task/1/children")),
            &format!("{text}\n"),
        );
    }

    fn set_ns(&self, pid: u32, inode: u64) {
        let target = PathBuf::from(format!("time:[{inode}]"));
        link(&target, &self.proc_root.join(format!("{pid}/ns/time")));
    }

    fn set_stat(&self, pid: u32, ppid: u32, start: u64) {
        write(
            &self.proc_root.join(format!("{pid}/stat")),
            &stat(pid, ppid, start, "S"),
        );
    }

    fn bound(&self) -> xtask::shell_timing_process_binding::ProcessBinding {
        match self
            .binder
            .resolve(WRAPPER, &self.host_file, &self.peer_file)
            .unwrap()
        {
            ResolveOutcome::Bound(binding) => *binding,
            other => panic!("expected Bound, got {other:?}"),
        }
    }
}

fn fixture() -> Fixture {
    let id = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
    let root =
        std::env::temp_dir().join(format!("shell-timing-binding-{}-{id}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let proc_root = root.join("proc");
    let host_file = root.join("bin/host-bin");
    let peer_file = root.join("bin/peer-bin");
    write(&host_file, "host executable fixture\n");
    write(&peer_file, "peer executable fixture\n");
    let binder = Binder::new(
        &CaptureRoots::new(proc_root.clone(), root.join("sys")),
        CAPTURER,
    );
    let fx = Fixture {
        root,
        proc_root,
        binder,
        host_file,
        peer_file,
    };
    // Reference and capturer live outside the wrapper tree.
    fx.process(REF, 0, 10, None, &[]);
    fx.process(CAPTURER, REF, 9000, None, &[]);
    write(
        &fx.proc_root.join(format!("{CAPTURER}/timens_offsets")),
        OFFSETS,
    );
    // wrapper -> shell -> host -> peer.
    fx.process(WRAPPER, CAPTURER, 1000, None, &[SHELL]);
    fx.process(SHELL, WRAPPER, 1100, None, &[HOST]);
    fx.process(HOST, SHELL, 2000, Some(&fx.host_file.clone()), &[PEER]);
    fx.process(PEER, HOST, 3000, Some(&fx.peer_file.clone()), &[]);
    fx
}

#[test]
fn binds_host_below_wrapper_and_peer_below_host_with_proof() {
    let fx = fixture();
    let binding = fx.bound();
    assert_eq!(
        (binding.wrapper.pid, binding.host.pid, binding.peer.pid),
        (WRAPPER, HOST, PEER)
    );
    assert_eq!(
        binding
            .chain
            .iter()
            .map(|node| node.identity.pid)
            .collect::<Vec<_>>(),
        [WRAPPER, SHELL, HOST, PEER]
    );
    assert_eq!(binding.host.start_time_ticks, 2000);
    assert_eq!(binding.peer.start_time_ticks, 3000);
    // Executable identity is the prepared file's dev+ino; the link
    // text is retained evidence, not the match key.
    let host_id =
        xtask::shell_timing_process_binding::ExecutableIdentity::of_file(&fx.host_file).unwrap();
    assert_eq!(binding.host_executable.identity, host_id);
    assert_eq!(binding.host_executable.observed_link, fx.host_file);
    assert_eq!(binding.host_executable.prepared_path, fx.host_file);
    // Namespace proof: all four inodes equal and nonzero, raw offsets
    // retained, monotonic offset exactly zero, provenance present.
    let time = &binding.time;
    for evidence in [&time.reference, &time.host, &time.peer, &time.capturer] {
        assert_eq!(evidence.time_namespace_inode, NS);
        assert_eq!(evidence.observed_target, format!("time:[{NS}]"));
        assert!(
            evidence
                .origin
                .ends_with(&format!("{}/ns/time", evidence.pid))
        );
        assert!(evidence.captured_usec > 0);
    }
    assert_eq!(time.reference.pid, REF);
    assert_eq!(time.capturer.pid, CAPTURER);
    assert_eq!(time.capturer_timens_offsets, OFFSETS);
    assert!(
        time.capturer_timens_offsets_origin
            .ends_with(&format!("{CAPTURER}/timens_offsets"))
    );
    assert_eq!(time.monotonic_offset, (0, 0));
    assert_eq!(fx.binder.recheck(&binding).unwrap(), BindingRecheck::Bound);
}

#[test]
fn unrelated_same_executable_outside_the_tree_is_ignored() {
    let fx = fixture();
    // Same host executable, but not a descendant of the wrapper: the
    // traversal never visits it, so it cannot create ambiguity.
    fx.process(555, REF, 5550, Some(&fx.host_file.clone()), &[]);
    let binding = fx.bound();
    assert_eq!(binding.host.pid, HOST);
}

#[test]
fn zero_matches_are_pending_not_errors() {
    // No host yet: the host process runs a different file.
    let fx = fixture();
    let other = fx.root.join("bin/other");
    write(&other, "other\n");
    link(&other, &fx.proc_root.join(format!("{HOST}/exe")));
    assert_eq!(
        fx.binder
            .resolve(WRAPPER, &fx.host_file, &fx.peer_file)
            .unwrap(),
        ResolveOutcome::Pending {
            stage: PendingStage::Host,
            host: None
        }
    );
    // Host present, peer not yet: Pending names the bound host.
    let fx = fixture();
    link(&other_or(&fx), &fx.proc_root.join(format!("{PEER}/exe")));
    assert_eq!(
        fx.binder
            .resolve(WRAPPER, &fx.host_file, &fx.peer_file)
            .unwrap(),
        ResolveOutcome::Pending {
            stage: PendingStage::Peer,
            host: Some(xtask::shell_timing_environment_capture::ProcessIdentity {
                pid: HOST,
                start_time_ticks: 2000
            })
        }
    );
}

fn other_or(fx: &Fixture) -> PathBuf {
    let other = fx.root.join("bin/other");
    write(&other, "other\n");
    other
}

#[test]
fn peer_outside_the_host_subtree_does_not_bind() {
    let fx = fixture();
    // A peer-executable process under the shell but not under the host,
    // while the real peer slot runs something else.
    let other = other_or(&fx);
    link(&other, &fx.proc_root.join(format!("{PEER}/exe")));
    fx.process(310, SHELL, 3100, Some(&fx.peer_file.clone()), &[]);
    fx.set_children(SHELL, &[HOST, 310]);
    match fx
        .binder
        .resolve(WRAPPER, &fx.host_file, &fx.peer_file)
        .unwrap()
    {
        ResolveOutcome::Pending {
            stage: PendingStage::Peer,
            ..
        } => {}
        other => panic!("expected Pending Peer, got {other:?}"),
    }
}

#[test]
fn ambiguous_host_is_an_error() {
    let fx = fixture();
    fx.process(210, SHELL, 2100, Some(&fx.host_file.clone()), &[]);
    fx.set_children(SHELL, &[HOST, 210]);
    let err = fx
        .binder
        .resolve(WRAPPER, &fx.host_file, &fx.peer_file)
        .unwrap_err();
    assert!(err.contains("ambiguous"), "{err}");
}

#[test]
fn children_and_stat_parent_disagreement_is_an_error() {
    let fx = fixture();
    // Listed under the shell, but its stat names another parent.
    fx.set_stat(HOST, 999, 2000);
    let err = fx
        .binder
        .resolve(WRAPPER, &fx.host_file, &fx.peer_file)
        .unwrap_err();
    assert!(err.contains("stat names parent"), "{err}");
}

#[test]
fn deleted_executable_cannot_bind_even_though_stat_can_succeed() {
    let fx = fixture();
    // The link target carries the kernel's deleted marker. The binder
    // rejects it from the link text explicitly and never reaches the
    // dev+ino comparison for this candidate.
    let deleted = PathBuf::from("/nonexistent/host-bin (deleted)");
    link(&deleted, &fx.proc_root.join(format!("{HOST}/exe")));
    assert_eq!(
        fx.binder
            .resolve(WRAPPER, &fx.host_file, &fx.peer_file)
            .unwrap(),
        ResolveOutcome::Pending {
            stage: PendingStage::Host,
            host: None
        }
    );
}

#[test]
fn zombie_host_does_not_bind() {
    let fx = fixture();
    write(
        &fx.proc_root.join(format!("{HOST}/stat")),
        &stat(HOST, SHELL, 2000, "Z"),
    );
    assert_eq!(
        fx.binder
            .resolve(WRAPPER, &fx.host_file, &fx.peer_file)
            .unwrap(),
        ResolveOutcome::Pending {
            stage: PendingStage::Host,
            host: None
        }
    );
}

#[test]
fn namespace_mismatch_and_nonzero_offset_are_errors() {
    let fx = fixture();
    fx.set_ns(PEER, NS + 1);
    let err = fx
        .binder
        .resolve(WRAPPER, &fx.host_file, &fx.peer_file)
        .unwrap_err();
    assert!(err.contains("inodes differ"), "{err}");

    let fx = fixture();
    fx.set_ns(HOST, NS + 159);
    let err = fx
        .binder
        .resolve(WRAPPER, &fx.host_file, &fx.peer_file)
        .unwrap_err();
    assert!(err.contains("inodes differ"), "{err}");

    let fx = fixture();
    write(
        &fx.proc_root.join(format!("{CAPTURER}/timens_offsets")),
        "monotonic 1 0\nboottime 0 0\n",
    );
    let err = fx
        .binder
        .resolve(WRAPPER, &fx.host_file, &fx.peer_file)
        .unwrap_err();
    assert!(err.contains("exactly zero"), "{err}");
}

#[test]
fn duplicate_and_trailing_monotonic_offsets_are_refused() {
    let fx = fixture();
    write(
        &fx.proc_root.join(format!("{CAPTURER}/timens_offsets")),
        "monotonic 0 0\nmonotonic 0 0\nboottime 0 0\n",
    );
    let err = fx
        .binder
        .resolve(WRAPPER, &fx.host_file, &fx.peer_file)
        .unwrap_err();
    assert!(err.contains("duplicate monotonic"), "{err}");

    let fx = fixture();
    write(
        &fx.proc_root.join(format!("{CAPTURER}/timens_offsets")),
        "monotonic 0 0 0\nboottime 0 0\n",
    );
    let err = fx
        .binder
        .resolve(WRAPPER, &fx.host_file, &fx.peer_file)
        .unwrap_err();
    assert!(err.contains("trailing fields"), "{err}");
}

#[test]
fn task_and_child_enumeration_are_bounded() {
    // Child union past the bound is refused during enumeration, before
    // any of the listed pids is stat-read.
    let fx = fixture();
    let children = (10_000..10_000 + MAX_CHILDREN_PER_PROCESS as u32 + 1)
        .map(|pid| pid.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    write(
        &fx.proc_root.join(format!("{WRAPPER}/task/1/children")),
        &format!("{children}\n"),
    );
    let err = fx
        .binder
        .resolve(WRAPPER, &fx.host_file, &fx.peer_file)
        .unwrap_err();
    assert!(err.contains("child count exceeds"), "{err}");

    // Task entries past the bound are refused while streaming the
    // directory, not after collecting every path.
    let fx = fixture();
    for task in 2..=MAX_TASKS_PER_PROCESS + 1 {
        fs::create_dir_all(fx.proc_root.join(format!("{WRAPPER}/task/{task}"))).unwrap();
    }
    let err = fx
        .binder
        .resolve(WRAPPER, &fx.host_file, &fx.peer_file)
        .unwrap_err();
    assert!(err.contains("task count exceeds"), "{err}");
}

#[test]
fn zero_namespace_inode_is_refused() {
    let fx = fixture();
    let target = PathBuf::from("time:[0]");
    link(&target, &fx.proc_root.join(format!("{PEER}/ns/time")));
    let err = fx
        .binder
        .resolve(WRAPPER, &fx.host_file, &fx.peer_file)
        .unwrap_err();
    assert!(err.contains("nonzero"), "{err}");
}

#[test]
fn recheck_detects_exit_reuse_and_broken_links() {
    // Ancestor gone: Ended.
    let fx = fixture();
    let binding = fx.bound();
    fs::remove_file(fx.proc_root.join(format!("{SHELL}/stat"))).unwrap();
    assert_eq!(fx.binder.recheck(&binding).unwrap(), BindingRecheck::Ended);
    // Ancestor in an ended state: also Ended.
    let fx = fixture();
    let binding = fx.bound();
    write(
        &fx.proc_root.join(format!("{SHELL}/stat")),
        &stat(SHELL, WRAPPER, 1100, "Z"),
    );
    assert_eq!(fx.binder.recheck(&binding).unwrap(), BindingRecheck::Ended);
    // Reused pid: an error, never Ended.
    let fx = fixture();
    let binding = fx.bound();
    fx.set_stat(HOST, SHELL, 2001);
    let err = fx.binder.recheck(&binding).unwrap_err();
    assert!(err.contains("reused"), "{err}");
    // Parent link changed.
    let fx = fixture();
    let binding = fx.bound();
    fx.set_stat(PEER, SHELL, 3000);
    let err = fx.binder.recheck(&binding).unwrap_err();
    assert!(err.contains("parent changed"), "{err}");
    // Parent no longer lists the child.
    let fx = fixture();
    let binding = fx.bound();
    fx.set_children(HOST, &[]);
    let err = fx.binder.recheck(&binding).unwrap_err();
    assert!(err.contains("no longer lists"), "{err}");
    // Executable swapped after binding.
    let fx = fixture();
    let binding = fx.bound();
    let other = other_or(&fx);
    link(&other, &fx.proc_root.join(format!("{PEER}/exe")));
    let err = fx.binder.recheck(&binding).unwrap_err();
    assert!(err.contains("no longer runs"), "{err}");
    // Namespace changed after binding.
    let fx = fixture();
    let binding = fx.bound();
    fx.set_ns(PEER, NS + 7);
    let err = fx.binder.recheck(&binding).unwrap_err();
    assert!(
        err.contains("inodes differ") || err.contains("proof changed"),
        "{err}"
    );
}
