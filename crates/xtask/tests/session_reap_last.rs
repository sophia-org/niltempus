//! A bounded recipe child is a process group that is signalled before it is
//! reaped (root's pattern, Sophia a6edbbcad): a successful leader must not
//! leave a background child running, and the leader must not be reaped before
//! the final group signal.
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

#[test]
fn a_successful_leader_takes_its_background_child_with_it() {
    let pid_file: PathBuf = std::env::temp_dir().join(format!("reap-last-{}", std::process::id()));
    let _ = std::fs::remove_file(&pid_file);
    // The child holds no pipe of ours: its stdio is /dev/null.
    let mut command = Command::new("/bin/sh");
    command.args([
        "-c",
        &format!(
            "/bin/sleep 60 </dev/null >/dev/null 2>&1 & printf '%s' $! > '{}'; exit 0",
            pid_file.display()
        ),
    ]);
    xtask::session::bounded_check(&mut command, "reap-last fixture").unwrap();
    let pid = std::fs::read_to_string(&pid_file).unwrap();
    let _ = std::fs::remove_file(&pid_file);
    for _ in 0..200 {
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Err(_) => return,
            Ok(stat) if stat.rsplit_once(") ").unwrap().1.starts_with('Z') => return,
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    panic!("background child {pid} survived its successful leader");
}
