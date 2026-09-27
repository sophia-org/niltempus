//! A bounded recipe child is a process group that is signalled before it is
//! reaped (root's pattern, Sophia a6edbbcad): a successful leader must not
//! leave a background child running. "Gone" means the child's /proc entry no
//! longer exists (ENOENT); a zombie is not gone.
//!
//! This binary holds exactly this one test, so it can make itself a child
//! subreaper: the orphaned background child re-parents to the test process,
//! which reaps it, so the entry actually disappears within the bound instead
//! of waiting on init.
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn a_successful_leader_takes_its_background_child_with_it() {
    rustix::process::set_child_subreaper(Some(rustix::process::getpid())).unwrap();
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
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        // Reap whatever re-parented to us (the killed background child).
        while let Ok(Some(_)) = rustix::process::wait(rustix::process::WaitOptions::NOHANG) {}
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Err(error) if error.kind() == ErrorKind::NotFound => return,
            Err(error) => panic!("unexpected error reading /proc/{pid}/stat: {error}"),
            Ok(stat) => {
                assert!(
                    Instant::now() < deadline,
                    "background child {pid} is still present (state {:?}) after its successful leader",
                    stat.rsplit_once(") ").map(|(_, rest)| &rest[..1])
                );
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
