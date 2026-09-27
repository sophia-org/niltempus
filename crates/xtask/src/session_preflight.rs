//! The host-specific active-session check that Sophia's retained launch
//! wrappers call through `sophia session check-host` (SOPHIA_SESSION_PREFLIGHT).
//!
//! It carries the refusal that Sophia's run_sophia_session.sh:123-144 made at
//! 9fcaec782ce4fe9978568c0466ee17a78b3d4571: taking over a TTY while one of
//! river, niri, sway, Hyprland, kwin_wayland or Xorg runs is refused; zombie
//! entries do not count (pgrep -x plus the `ps -o stat=` Z* exclusion). The
//! names are this host's policy, which is why they live outside Sophia.
//!
//! Contract with `sophia session check-host` (approved):
//!   argv: --tty=<absolute tty path>, exactly one argument
//!   exit 0: stdout is exactly one line
//!           `sophia_session_preflight schema=1 status=clear tty=<tty>`
//!   exit 1: an active session was found; stderr names process:pid
//!   exit 2: usage
use std::path::Path;

/// The graphical sessions whose presence refuses a takeover.
pub const ACTIVE_SESSION_NAMES: [&str; 6] =
    ["river", "niri", "sway", "Hyprland", "kwin_wayland", "Xorg"];

/// `process:pid` for every live (non-zombie) process whose name is exactly
/// one of the names, in name order then pid order, read from `proc_root`
/// (normally /proc).
pub fn active_sessions(proc_root: &Path) -> std::io::Result<Vec<String>> {
    let mut processes = Vec::new();
    for entry in std::fs::read_dir(proc_root)? {
        let entry = entry?;
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        // A process may exit between the listing and the read.
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // pid (comm) state ...; comm may contain spaces and parentheses.
        let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else {
            continue;
        };
        let name = &stat[open + 1..close];
        let state = stat[close + 1..].split_whitespace().next().unwrap_or("");
        if state.starts_with('Z') {
            continue;
        }
        if let Some(index) = ACTIVE_SESSION_NAMES.iter().position(|n| *n == name) {
            processes.push((index, pid, name.to_owned()));
        }
    }
    processes.sort();
    Ok(processes
        .into_iter()
        .map(|(_, pid, name)| format!("{name}:{pid}"))
        .collect())
}

/// The checker's decision for `args` (argv without the program name):
/// (exit status, stdout, stderr).
pub fn run(args: &[String], proc_root: &Path) -> (i32, String, String) {
    let [argument] = args else {
        return (2, String::new(), usage());
    };
    let Some(tty) = argument.strip_prefix("--tty=") else {
        return (2, String::new(), usage());
    };
    if !tty.starts_with('/') || tty.len() < 2 || tty.chars().any(char::is_control) {
        return (2, String::new(), usage());
    }
    match active_sessions(proc_root) {
        Ok(active) if active.is_empty() => (
            0,
            format!("sophia_session_preflight schema=1 status=clear tty={tty}\n"),
            String::new(),
        ),
        Ok(active) => (
            1,
            String::new(),
            format!(
                "Refusing to take over a TTY while a graphical session is active.\n\
                 Still active (process:pid): {}\n",
                active.join(" ")
            ),
        ),
        // Unreadable process table: refuse rather than clear.
        Err(error) => (
            1,
            String::new(),
            format!("cannot read the process table: {error}\n"),
        ),
    }
}

fn usage() -> String {
    "usage: active-session-preflight --tty=/absolute/tty\n".to_owned()
}
