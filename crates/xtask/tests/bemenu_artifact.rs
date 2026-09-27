// Provenance: moved from Sophia crates/xtask/tests/bemenu_artifact.rs at
// 9fcaec782ce4fe9978568c0466ee17a78b3d4571 (Sophia rule 13).
//! The artifact preparer refuses ambiguous input and bounds process cleanup.
use xtask::bemenu_artifact;

use std::os::unix::{fs::DirBuilderExt, process::CommandExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

struct ChildPid(PathBuf);
impl ChildPid {
    fn new(name: &str) -> Self {
        // Reap only this fixture's orphan, never another parallel test's
        // leader. A zombie does not count as successful process cleanup.
        rustix::process::set_child_subreaper(Some(rustix::process::getpid())).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "bemenu-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        Self(dir.join("child.pid"))
    }

    fn assert_gone(&self) {
        let pid = std::fs::read_to_string(&self.0).expect("fixture must record its child pid");
        let pid: u32 = pid.trim().parse().unwrap();
        let child = rustix::process::Pid::from_raw(pid as i32).unwrap();
        let proc = PathBuf::from(format!("/proc/{pid}/stat"));
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let _ = rustix::process::waitpid(Some(child), rustix::process::WaitOptions::NOHANG);
            match std::fs::read_to_string(&proc) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                Err(error) => panic!("cannot inspect fixture child: {error}"),
                Ok(stat) => assert!(
                    Instant::now() < deadline,
                    "fixture descendant {pid} survived group cleanup: {stat}"
                ),
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for ChildPid {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}

#[test]
fn ambiguous_revision_and_existing_destination_are_refused() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(
        bemenu_artifact::run(root, &[])
            .unwrap_err()
            .contains("usage")
    );
    let args = [root.display().to_string(), "HEAD".into(), "unused".into()];
    assert!(
        bemenu_artifact::run(root, &args)
            .unwrap_err()
            .contains("40 lowercase hex")
    );
    let args = [
        root.display().to_string(),
        "0".repeat(40),
        root.display().to_string(),
    ];
    assert!(
        bemenu_artifact::run(root, &args)
            .unwrap_err()
            .contains("already exists")
    );
}

#[test]
fn excess_output_is_refused_instead_of_truncated_success() {
    let error = bemenu_artifact::bounded(
        Command::new("head").args(["-c", "1048577", "/dev/zero"]),
        Duration::from_secs(5),
        "oversized fixture",
    )
    .unwrap_err();
    assert!(error.contains("output exceeds"), "{error}");
}

#[test]
fn timeout_stops_the_private_process_group() {
    let pid = ChildPid::new("timeout");
    let start = Instant::now();
    let error = bemenu_artifact::bounded(
        Command::new("sh")
            .args([
                "-c",
                "sleep 30 & printf '%s' \"$!\" >\"$1\"; wait",
                "fixture",
            ])
            .arg(&pid.0),
        Duration::from_secs(1),
        "timeout fixture",
    )
    .unwrap_err();
    assert!(error.contains("exceeded"), "{error}");
    assert!(start.elapsed() < Duration::from_secs(6));
    pid.assert_gone();
}

#[test]
fn inherited_pipe_cannot_keep_collection_waiting_after_leader_exit() {
    let pid = ChildPid::new("pipe");
    let start = Instant::now();
    let error = bemenu_artifact::bounded(
        Command::new("sh")
            .args([
                "-c",
                "sleep 30 & printf '%s' \"$!\" >\"$1\"; exit 0",
                "fixture",
            ])
            .arg(&pid.0),
        Duration::from_secs(5),
        "inherited pipe fixture",
    )
    .unwrap_err();
    assert!(error.contains("descendant kept its output open"), "{error}");
    assert!(start.elapsed() < Duration::from_secs(7));
    pid.assert_gone();
}

fn background_command(pid: &ChildPid) -> Command {
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 30 </dev/null >/dev/null 2>&1 & printf '%s' \"$!\" >\"$1\"; printf 'done'; exit 0", "fixture"])
        .arg(&pid.0);
    command
}

#[test]
fn successful_capture_cleans_up_a_child_that_holds_no_pipe() {
    let pid = ChildPid::new("success");
    let output = bemenu_artifact::bounded(
        &mut background_command(&pid),
        Duration::from_secs(5),
        "successful fixture",
    )
    .unwrap();
    assert_eq!(output, b"done");
    pid.assert_gone();
}

#[test]
fn successful_logged_build_cleans_up_a_child_that_holds_no_pipe() {
    let pid = ChildPid::new("logged-success");
    let log = pid.0.with_file_name("build.log");
    let file = std::fs::File::create(&log).unwrap();
    let child = background_command(&pid)
        .process_group(0)
        .stdout(file.try_clone().unwrap())
        .stderr(file)
        .spawn()
        .unwrap();
    bemenu_artifact::wait_logged(child, &log, Duration::from_secs(5), "logged fixture").unwrap();
    assert_eq!(std::fs::read(&log).unwrap(), b"done");
    pid.assert_gone();
}

#[test]
fn successful_build_that_already_exited_cannot_hide_a_log_overflow() {
    let fixture = ChildPid::new("logged-overflow");
    let log = fixture.0.with_file_name("build.log");
    let file = std::fs::File::create(&log).unwrap();
    let child = Command::new("head")
        .args(["-c", "4194305", "/dev/zero"])
        .process_group(0)
        .stdout(file.try_clone().unwrap())
        .stderr(file)
        .spawn()
        .unwrap();
    // Observe an exited leader without reaping it. This makes the old
    // success-before-size-check bug deterministic, independent of polling.
    let pid = rustix::process::Pid::from_raw(child.id() as i32).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = rustix::process::waitid(
            rustix::process::WaitId::Pid(pid),
            rustix::process::WaitIdOptions::EXITED
                | rustix::process::WaitIdOptions::NOWAIT
                | rustix::process::WaitIdOptions::NOHANG,
        )
        .unwrap()
        {
            assert_eq!(status.exit_status(), Some(0));
            break;
        }
        assert!(Instant::now() < deadline, "fixture did not exit");
        std::thread::sleep(Duration::from_millis(10));
    }
    let error =
        bemenu_artifact::wait_logged(child, &log, Duration::from_secs(5), "overflow fixture")
            .unwrap_err();
    assert!(
        error.contains("log") && error.contains("4194305"),
        "{error}"
    );
}
