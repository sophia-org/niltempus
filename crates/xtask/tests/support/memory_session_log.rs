use super::*;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "session-log-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    fn log(&self) -> PathBuf {
        self.0
            .join("state/sophia/session/untrusted-session-output.log")
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn capture_is_memory_backed_before_the_wrapper_runs() {
    let root = Root::new();
    let capture = SessionLog::new(&root.log(), 1024).unwrap();
    assert_eq!(
        rustix::fs::fstatfs(&capture.file).unwrap().f_type as i64,
        TMPFS_MAGIC
    );
    assert_eq!(
        fs::metadata(capture.target.parent().unwrap())
            .unwrap()
            .mode()
            & 0o777,
        0o700
    );
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(root.log().with_extension("storage.json")).unwrap())
            .unwrap();
    assert_eq!(receipt["source"], capture.target.to_string_lossy().as_ref());
    assert_eq!(fs::metadata(root.log()).unwrap().mode() & 0o777, 0o600);
    capture.persist().unwrap();
}

fn shell(path: &Path, exit: u8) -> std::process::Output {
    Command::new("/bin/bash")
        .args(["-c", ": >\"$1\"; chmod 600 \"$1\"; (printf 'host\\n'; /bin/sh -c 'printf \"peer\\n\"; printf \"err\\n\" >&2'; exit \"$2\") >\"$1\" 2>&1", "capture"])
        .arg(path).arg(exit.to_string()).output().unwrap()
}

#[test]
fn real_wrapper_redirection_and_live_readers_keep_exact_bytes() {
    let root = Root::new();
    let capture = SessionLog::new(&root.log(), 1024).unwrap();
    assert!(shell(&root.log(), 0).status.success());
    assert_eq!(fs::read(root.log()).unwrap(), b"host\npeer\nerr\n");
    assert_eq!(fs::metadata(root.log()).unwrap().len(), 14);
    capture.persist().unwrap();
    assert!(!fs::symlink_metadata(root.log()).unwrap().is_symlink());
    assert_eq!(fs::read(root.log()).unwrap(), b"host\npeer\nerr\n");
    assert_eq!(fs::metadata(root.log()).unwrap().mode() & 0o777, 0o600);
}

#[test]
fn wrapper_failure_keeps_the_named_source_after_runner_state_drops() {
    let root = Root::new();
    let capture = SessionLog::new(&root.log(), 1024).unwrap();
    assert_eq!(shell(&root.log(), 23).status.code(), Some(23));
    let target = capture.target.clone();
    drop(capture);
    assert_eq!(fs::read(root.log()).unwrap(), b"host\npeer\nerr\n");
    assert!(fs::symlink_metadata(root.log()).unwrap().is_symlink());
    fs::remove_file(&target).unwrap();
    fs::remove_dir(target.parent().unwrap()).unwrap();
}

#[test]
fn collisions_and_replaced_links_are_never_overwritten() {
    let root = Root::new();
    let capture = SessionLog::new(&root.log(), 1024).unwrap();
    assert!(SessionLog::new(&root.log(), 1024).is_err());

    assert!(shell(&root.log(), 0).status.success());
    let partial = root.log().with_extension("log.partial");
    fs::write(&partial, b"existing").unwrap();
    let target = capture.target.clone();
    let error = capture.persist().unwrap_err();
    assert!(error.contains("persistence failed"));
    assert_eq!(fs::read(root.log()).unwrap(), b"host\npeer\nerr\n");
    fs::remove_file(&target).unwrap();
    fs::remove_dir(target.parent().unwrap()).unwrap();
    assert_eq!(fs::read(&partial).unwrap(), b"existing");

    let root = Root::new();
    let capture = SessionLog::new(&root.log(), 1024).unwrap();
    fs::remove_file(root.log()).unwrap();
    fs::write(root.log(), b"replacement").unwrap();
    let target = capture.target.clone();
    assert!(capture.persist().unwrap_err().contains("replaced"));
    fs::remove_file(&target).unwrap();
    fs::remove_dir(target.parent().unwrap()).unwrap();
    assert_eq!(fs::read(root.log()).unwrap(), b"replacement");
}

#[test]
fn over_cap_preserves_full_ram_source_and_bounded_partial_file() {
    let root = Root::new();
    let capture = SessionLog::new(&root.log(), 8).unwrap();
    assert!(shell(&root.log(), 0).status.success());
    let target = capture.target.clone();
    let error = capture.persist().unwrap_err();
    assert!(error.contains("exceeds cap 8"));
    assert_eq!(
        fs::read(root.log().with_extension("log.partial")).unwrap(),
        b"host\npee"
    );
    assert!(fs::symlink_metadata(root.log()).unwrap().is_symlink());
    assert_eq!(fs::read(root.log()).unwrap(), b"host\npeer\nerr\n");
    fs::remove_file(&target).unwrap();
    fs::remove_dir(target.parent().unwrap()).unwrap();
}

#[test]
fn canonical_evidence_move_refuses_existing_destination() {
    let root = Root::new();
    let from = root.0.join("from");
    let to = root.0.join("to");
    fs::write(&from, b"new").unwrap();
    fs::write(&to, b"old").unwrap();
    assert!(move_new(&from, &to).is_err());
    assert_eq!(fs::read(&from).unwrap(), b"new");
    assert_eq!(fs::read(&to).unwrap(), b"old");
    fs::remove_file(&to).unwrap();
    move_new(&from, &to).unwrap();
    assert_eq!(fs::read(to).unwrap(), b"new");
}

#[test]
fn external_kill_preserves_named_source_and_live_path() {
    const CHILD: &str = "SOPHIA_MEMORY_LOG_TEST_CHILD";
    if let Some(path) = std::env::var_os(CHILD) {
        let path = PathBuf::from(path);
        let _capture = SessionLog::new(&path, 1024).unwrap();
        assert!(shell(&path, 0).status.success());
        fs::write(path.with_extension("ready"), b"ready").unwrap();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    let root = Root::new();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "output_file_native_run::process::session_log::tests::external_kill_preserves_named_source_and_live_path"])
        .env(CHILD, root.log()).stdout(std::process::Stdio::null()).spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !root.log().with_extension("ready").exists() {
        if child.try_wait().unwrap().is_some() || std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("child never staged its log");
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(fs::read(root.log()).unwrap(), b"host\npeer\nerr\n");
    let target = fs::read_link(root.log()).unwrap();
    fs::remove_file(&target).unwrap();
    fs::remove_dir(target.parent().unwrap()).unwrap();
}

#[test]
fn replaced_source_is_neither_copied_nor_removed() {
    let root = Root::new();
    let capture = SessionLog::new(&root.log(), 1024).unwrap();
    assert!(shell(&root.log(), 0).status.success());
    let target = capture.target.clone();
    fs::remove_file(&target).unwrap();
    fs::write(&target, b"replacement").unwrap();
    assert!(
        capture
            .persist()
            .unwrap_err()
            .contains("source was replaced")
    );
    assert_eq!(fs::read(root.log()).unwrap(), b"replacement");
    assert!(!root.log().with_extension("log.partial").exists());
    fs::remove_file(&target).unwrap();
    fs::remove_dir(target.parent().unwrap()).unwrap();
}
