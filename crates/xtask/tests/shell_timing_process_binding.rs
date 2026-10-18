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
    Binder, BindingRecheck, Executables, INITIAL_TIME_NAMESPACE_INODE, MAX_CHILDREN_PER_PROCESS,
    MAX_TASKS_PER_PROCESS, PendingReason, PendingStage, ResolveOutcome, UNIQUENESS_SCOPE,
};

const REF: u32 = 1;
const WRAPPER: u32 = 100;
const SHELL: u32 = 110;
const HOST: u32 = 200;
const SUPERVISOR: u32 = 250;
const PEER: u32 = 300;
const CAPTURER: u32 = 900;
const NS: u64 = INITIAL_TIME_NAMESPACE_INODE;
const OFFSETS: &str = "monotonic 0 0\nboottime 0 0\n";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    proc_root: PathBuf,
    binder: Binder,
    host_file: PathBuf,
    supervisor_file: PathBuf,
    peer_file: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        // Sealed directories must be searchable again before removal.
        if let Ok(entries) = fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with("sealed-") {
                    let _ = fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o700));
                }
            }
        }
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

    fn exes(&self) -> Executables<'_> {
        Executables {
            host: &self.host_file,
            supervisor: &self.supervisor_file,
            peer: &self.peer_file,
        }
    }

    fn resolve(&self) -> Result<ResolveOutcome, String> {
        self.binder.resolve(WRAPPER, self.exes())
    }

    /// Make `pid`'s executable unreadable the way a non-dumpable process
    /// is: the `exe` link resolves through an unsearchable directory, so
    /// the identity read fails with EACCES while stat stays readable.
    fn opaque(&self, pid: u32) {
        use std::os::unix::fs::PermissionsExt;
        let sealed = self.root.join(format!("sealed-{pid}"));
        fs::create_dir_all(&sealed).unwrap();
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o000)).unwrap();
        link(
            &sealed.join("exe"),
            &self.proc_root.join(format!("{pid}/exe")),
        );
    }

    fn bound(&self) -> xtask::shell_timing_process_binding::ProcessBinding {
        match self.resolve().unwrap() {
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
    let supervisor_file = root.join("bin/supervisor-bin");
    let peer_file = root.join("bin/peer-bin");
    write(&host_file, "host executable fixture\n");
    write(&supervisor_file, "supervisor executable fixture\n");
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
        supervisor_file,
        peer_file,
    };
    // Reference and capturer live outside the wrapper tree.
    fx.process(REF, 0, 10, None, &[]);
    fx.process(CAPTURER, REF, 9000, None, &[]);
    write(
        &fx.proc_root.join(format!("{CAPTURER}/timens_offsets")),
        OFFSETS,
    );
    // wrapper -> shell -> host -> supervisor -> peer.
    fx.process(WRAPPER, CAPTURER, 1000, None, &[SHELL]);
    fx.process(SHELL, WRAPPER, 1100, None, &[HOST]);
    fx.process(
        HOST,
        SHELL,
        2000,
        Some(&fx.host_file.clone()),
        &[SUPERVISOR],
    );
    fx.process(
        SUPERVISOR,
        HOST,
        2500,
        Some(&fx.supervisor_file.clone()),
        &[PEER],
    );
    fx.process(PEER, SUPERVISOR, 3000, Some(&fx.peer_file.clone()), &[]);
    fx
}

#[test]
fn binds_host_below_wrapper_and_supervised_peer_below_host_with_proof() {
    let fx = fixture();
    let binding = fx.bound();
    assert_eq!(
        (
            binding.wrapper.pid,
            binding.host.pid,
            binding.supervisor.pid,
            binding.peer.pid
        ),
        (WRAPPER, HOST, SUPERVISOR, PEER)
    );
    assert_eq!(
        binding.supervisor_executable.prepared_path,
        fx.supervisor_file
    );
    assert!(binding.unexamined.is_empty());
    assert_eq!(binding.uniqueness, UNIQUENESS_SCOPE);
    assert_eq!(
        binding
            .chain
            .iter()
            .map(|node| node.identity.pid)
            .collect::<Vec<_>>(),
        [WRAPPER, SHELL, HOST, SUPERVISOR, PEER]
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
    // Namespace proof: host, peer and capturer in the initial time
    // namespace, raw offsets retained, monotonic offset exactly zero.
    let time = &binding.time;
    assert_eq!(time.reference.inode, INITIAL_TIME_NAMESPACE_INODE);
    for evidence in [&time.host, &time.peer, &time.capturer] {
        assert_eq!(evidence.time_namespace_inode, NS);
        assert_eq!(evidence.observed_target, format!("time:[{NS}]"));
        assert!(
            evidence
                .origin
                .ends_with(&format!("{}/ns/time", evidence.pid))
        );
        assert!(evidence.captured_usec > 0);
    }
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
        fx.resolve().unwrap(),
        ResolveOutcome::Pending {
            stage: PendingStage::Host,
            host: None,
            reason: PendingReason::NoMatch,
        }
    );
    // Host present, peer not yet: Pending names the bound host.
    let fx = fixture();
    link(&other_or(&fx), &fx.proc_root.join(format!("{PEER}/exe")));
    assert_eq!(
        fx.resolve().unwrap(),
        ResolveOutcome::Pending {
            stage: PendingStage::Peer,
            host: Some(xtask::shell_timing_environment_capture::ProcessIdentity {
                pid: HOST,
                start_time_ticks: 2000
            }),
            reason: PendingReason::NoMatch,
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
    match fx.resolve().unwrap() {
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
    let err = fx.resolve().unwrap_err();
    assert!(err.contains("ambiguous"), "{err}");
}

#[test]
fn children_and_stat_parent_disagreement_is_an_error() {
    let fx = fixture();
    // Listed under the shell, but its stat names another parent.
    fx.set_stat(HOST, 999, 2000);
    let err = fx.resolve().unwrap_err();
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
        fx.resolve().unwrap(),
        ResolveOutcome::Pending {
            stage: PendingStage::Host,
            host: None,
            reason: PendingReason::NoMatch,
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
        fx.resolve().unwrap(),
        ResolveOutcome::Pending {
            stage: PendingStage::Host,
            host: None,
            reason: PendingReason::NoMatch,
        }
    );
}

#[test]
fn namespace_mismatch_and_nonzero_offset_are_errors() {
    let fx = fixture();
    fx.set_ns(PEER, NS + 1);
    let err = fx.resolve().unwrap_err();
    assert!(err.contains("inodes differ"), "{err}");

    let fx = fixture();
    fx.set_ns(HOST, NS + 159);
    let err = fx.resolve().unwrap_err();
    assert!(err.contains("inodes differ"), "{err}");

    let fx = fixture();
    write(
        &fx.proc_root.join(format!("{CAPTURER}/timens_offsets")),
        "monotonic 1 0\nboottime 0 0\n",
    );
    let err = fx.resolve().unwrap_err();
    assert!(err.contains("exactly zero"), "{err}");
}

#[test]
fn duplicate_and_trailing_monotonic_offsets_are_refused() {
    let fx = fixture();
    write(
        &fx.proc_root.join(format!("{CAPTURER}/timens_offsets")),
        "monotonic 0 0\nmonotonic 0 0\nboottime 0 0\n",
    );
    let err = fx.resolve().unwrap_err();
    assert!(err.contains("duplicate monotonic"), "{err}");

    let fx = fixture();
    write(
        &fx.proc_root.join(format!("{CAPTURER}/timens_offsets")),
        "monotonic 0 0 0\nboottime 0 0\n",
    );
    let err = fx.resolve().unwrap_err();
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
    let err = fx.resolve().unwrap_err();
    assert!(err.contains("child count exceeds"), "{err}");

    // Task entries past the bound are refused while streaming the
    // directory, not after collecting every path.
    let fx = fixture();
    for task in 2..=MAX_TASKS_PER_PROCESS + 1 {
        fs::create_dir_all(fx.proc_root.join(format!("{WRAPPER}/task/{task}"))).unwrap();
    }
    let err = fx.resolve().unwrap_err();
    assert!(err.contains("task count exceeds"), "{err}");
}

#[test]
fn zero_namespace_inode_is_refused() {
    let fx = fixture();
    let target = PathBuf::from("time:[0]");
    link(&target, &fx.proc_root.join(format!("{PEER}/ns/time")));
    let err = fx.resolve().unwrap_err();
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

fn pending(fx: &Fixture) -> (PendingStage, PendingReason) {
    match fx.resolve().unwrap() {
        ResolveOutcome::Pending { stage, reason, .. } => (stage, reason),
        other => panic!("expected Pending, got {other:?}"),
    }
}

#[test]
fn an_opaque_process_below_the_host_is_listed_not_compared() {
    // A Factotum-like child of the host and its helper child.
    let fx = fixture();
    fx.process(260, HOST, 2600, None, &[261]);
    fx.process(261, 260, 2610, None, &[]);
    fx.set_children(HOST, &[SUPERVISOR, 260]);
    fx.opaque(260);
    fx.opaque(261);
    let binding = fx.bound();
    assert_eq!(
        binding
            .unexamined
            .iter()
            .map(|node| (node.identity.pid, node.parent_pid))
            .collect::<Vec<_>>(),
        [(260, HOST), (261, 260)]
    );
    assert_eq!(binding.uniqueness, UNIQUENESS_SCOPE);
    assert_eq!(fx.binder.recheck(&binding).unwrap(), BindingRecheck::Bound);
}

#[test]
fn an_opaque_process_outside_the_host_or_on_its_lineage_is_pending() {
    // Beside the host, under the shell.
    let fx = fixture();
    fx.process(120, SHELL, 1200, None, &[]);
    fx.set_children(SHELL, &[HOST, 120]);
    fx.opaque(120);
    assert_eq!(
        pending(&fx),
        (PendingStage::Host, PendingReason::UnexaminedOutsideHost)
    );
    // On the wrapper-to-host lineage.
    let fx = fixture();
    fx.opaque(SHELL);
    assert_eq!(
        pending(&fx),
        (PendingStage::Host, PendingReason::UnexaminedOutsideHost)
    );
}

#[test]
fn an_opaque_process_in_the_selected_supervisor_subtree_is_pending() {
    // Below the peer.
    let fx = fixture();
    fx.process(310, PEER, 3100, None, &[]);
    fx.set_children(PEER, &[310]);
    fx.opaque(310);
    assert_eq!(
        pending(&fx),
        (
            PendingStage::Peer,
            PendingReason::UnexaminedInSelectedSubtree
        )
    );
    // The supervisor itself.
    let fx = fixture();
    fx.opaque(SUPERVISOR);
    assert_eq!(
        pending(&fx),
        (
            PendingStage::Peer,
            PendingReason::UnexaminedInSelectedSubtree
        )
    );
    // The peer itself: no supervisor is selected without an examined
    // peer, so this is the ordinary no-match wait.
    let fx = fixture();
    fx.opaque(PEER);
    assert_eq!(pending(&fx), (PendingStage::Peer, PendingReason::NoMatch));
}

#[test]
fn a_supervisor_child_before_exec_is_pending() {
    // Bubblewrap's child still runs the supervisor file.
    let fx = fixture();
    link(
        &fx.supervisor_file.clone(),
        &fx.proc_root.join(format!("{PEER}/exe")),
    );
    assert_eq!(pending(&fx), (PendingStage::Peer, PendingReason::NoMatch));
}

#[test]
fn a_host_fork_before_exec_is_pending_and_a_third_host_is_an_error() {
    let fx = fixture();
    fx.process(270, HOST, 2700, Some(&fx.host_file.clone()), &[]);
    fx.set_children(HOST, &[SUPERVISOR, 270]);
    assert_eq!(
        pending(&fx),
        (PendingStage::Host, PendingReason::ForkBeforeExec)
    );
    // After exec the child runs the supervisor file and is not a host.
    link(&fx.supervisor_file.clone(), &fx.proc_root.join("270/exe"));
    assert_eq!(fx.bound().host.pid, HOST);
    // Two host matches that are not parent and child.
    let fx = fixture();
    fx.process(271, SUPERVISOR, 2710, Some(&fx.host_file.clone()), &[]);
    fx.set_children(SUPERVISOR, &[PEER, 271]);
    let err = fx.resolve().unwrap_err();
    assert!(err.contains("ambiguous"), "{err}");
    // Three host matches, even if a fork pair is among them.
    let fx = fixture();
    fx.process(270, HOST, 2700, Some(&fx.host_file.clone()), &[272]);
    fx.process(272, 270, 2720, Some(&fx.host_file.clone()), &[]);
    fx.set_children(HOST, &[SUPERVISOR, 270]);
    let err = fx.resolve().unwrap_err();
    assert!(err.contains("3 examined descendants"), "{err}");
}

#[test]
fn a_peer_outside_the_supervised_position_is_an_error() {
    let message = "outside a supervised position";
    // A direct child of the host.
    let fx = fixture();
    fx.process(280, HOST, 2800, Some(&fx.peer_file.clone()), &[]);
    fx.set_children(HOST, &[SUPERVISOR, 280]);
    let err = fx.resolve().unwrap_err();
    assert!(err.contains(message), "{err}");
    // A supervisor with a second child.
    let fx = fixture();
    fx.process(301, SUPERVISOR, 3010, None, &[]);
    fx.set_children(SUPERVISOR, &[PEER, 301]);
    let err = fx.resolve().unwrap_err();
    assert!(err.contains(message), "{err}");
    // A supervisor that is not the host's direct child.
    let fx = fixture();
    fx.process(255, HOST, 2550, None, &[SUPERVISOR]);
    fx.set_children(HOST, &[255]);
    fx.set_stat(SUPERVISOR, 255, 2500);
    let err = fx.resolve().unwrap_err();
    assert!(err.contains(message), "{err}");
}

#[test]
fn two_supervised_peers_are_an_error() {
    let fx = fixture();
    fx.process(251, HOST, 2510, Some(&fx.supervisor_file.clone()), &[302]);
    fx.process(302, 251, 3020, Some(&fx.peer_file.clone()), &[]);
    fx.set_children(HOST, &[SUPERVISOR, 251]);
    let err = fx.resolve().unwrap_err();
    assert!(err.contains("2 supervised peers"), "{err}");
}

#[test]
fn the_three_prepared_files_must_differ() {
    let fx = fixture();
    let err = fx
        .binder
        .resolve(
            WRAPPER,
            Executables {
                host: &fx.host_file,
                supervisor: &fx.host_file,
                peer: &fx.peer_file,
            },
        )
        .unwrap_err();
    assert!(err.contains("different files"), "{err}");
}

#[test]
fn equal_namespaces_other_than_the_initial_one_are_refused() {
    let fx = fixture();
    for pid in [HOST, PEER, CAPTURER] {
        fx.set_ns(pid, NS - 1);
    }
    let err = fx.resolve().unwrap_err();
    assert!(err.contains("inodes differ"), "{err}");
}

#[test]
fn recheck_holds_the_exact_supervised_position_and_executable() {
    // A second supervisor child after binding.
    let fx = fixture();
    let binding = fx.bound();
    fx.process(301, SUPERVISOR, 3010, None, &[]);
    fx.set_children(SUPERVISOR, &[PEER, 301]);
    let err = fx.binder.recheck(&binding).unwrap_err();
    assert!(err.contains("only child"), "{err}");
    // The supervisor runs something else.
    let fx = fixture();
    let binding = fx.bound();
    link(
        &other_or(&fx),
        &fx.proc_root.join(format!("{SUPERVISOR}/exe")),
    );
    let err = fx.binder.recheck(&binding).unwrap_err();
    assert!(err.contains("no longer runs"), "{err}");
    // The prepared supervisor file was replaced.
    let fx = fixture();
    let binding = fx.bound();
    fs::remove_file(&fx.supervisor_file).unwrap();
    write(&fx.supervisor_file, "replacement\n");
    let err = fx.binder.recheck(&binding).unwrap_err();
    assert!(err.contains("changed since binding"), "{err}");
    // A bound process that turns opaque.
    let fx = fixture();
    let binding = fx.bound();
    fx.opaque(PEER);
    let err = fx.binder.recheck(&binding).unwrap_err();
    assert!(err.contains("no longer readable"), "{err}");
}
