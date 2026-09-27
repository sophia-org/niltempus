//! Host active-session preflight for Sophia's `session check-host`; see
//! xtask::session_preflight for the contract.
use std::io::Write;

fn main() -> std::process::ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let (status, stdout, stderr) =
        xtask::session_preflight::run(&args, std::path::Path::new("/proc"));
    let _ = std::io::stdout().write_all(stdout.as_bytes());
    let _ = std::io::stderr().write_all(stderr.as_bytes());
    std::process::ExitCode::from(status as u8)
}
