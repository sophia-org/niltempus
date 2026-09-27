//! The host active-session checker's contract with `sophia session
//! check-host`, against a synthetic process table (and one real run).
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use xtask::session_preflight::{ACTIVE_SESSION_NAMES, active_sessions, run};

struct Table(PathBuf);
impl Table {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("preflight-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).unwrap();
        // Non-process entries are ignored.
        fs::create_dir(path.join("self")).unwrap();
        fs::write(path.join("uptime"), "1 1\n").unwrap();
        Self(path)
    }
    fn process(&self, pid: u32, name: &str, state: &str) {
        let dir = self.0.join(pid.to_string());
        fs::create_dir(&dir).unwrap();
        fs::write(
            dir.join("stat"),
            format!("{pid} ({name}) {state} 1 1 1 0 -1\n"),
        )
        .unwrap();
    }
}
impl Drop for Table {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tty() -> Vec<String> {
    vec!["--tty=/dev/tty3".to_owned()]
}

#[test]
fn each_graphical_session_name_refuses_with_its_pid() {
    for (index, name) in ACTIVE_SESSION_NAMES.iter().enumerate() {
        let table = Table::new(&format!("one-{index}"));
        table.process(100, "bash", "S");
        table.process(4200 + index as u32, name, "S");
        let (status, stdout, stderr) = run(&tty(), &table.0);
        assert_eq!(status, 1, "{name}");
        assert!(stdout.is_empty());
        assert!(
            stderr.contains(&format!("{name}:{}", 4200 + index)),
            "{name}: {stderr}"
        );
        assert!(stderr.contains("Refusing to take over a TTY"));
    }
}

#[test]
fn zombies_and_near_names_do_not_refuse() {
    let table = Table::new("zombie");
    table.process(10, "Xorg", "Z");
    table.process(11, "sway-helper", "S");
    table.process(12, "niri ", "S");
    table.process(13, "(Hyprland)", "S");
    table.process(14, "xorg", "S");
    assert!(active_sessions(&table.0).unwrap().is_empty());
    let (status, stdout, stderr) = run(&tty(), &table.0);
    assert_eq!(status, 0, "{stderr}");
    assert_eq!(
        stdout,
        "sophia_session_preflight schema=1 status=clear tty=/dev/tty3\n"
    );
}

#[test]
fn names_with_spaces_and_parentheses_are_parsed_from_the_last_parenthesis() {
    let table = Table::new("comm");
    // A crafted comm cannot hide a real name, and cannot fake one either.
    table.process(20, "a) Xorg (b", "S");
    assert!(active_sessions(&table.0).unwrap().is_empty());
    table.process(21, "kwin_wayland", "R");
    assert_eq!(active_sessions(&table.0).unwrap(), ["kwin_wayland:21"]);
}

#[test]
fn several_sessions_are_all_named_in_a_stable_order() {
    let table = Table::new("several");
    table.process(30, "Xorg", "S");
    table.process(31, "river", "S");
    table.process(29, "river", "S");
    assert_eq!(
        active_sessions(&table.0).unwrap(),
        ["river:29", "river:31", "Xorg:30"]
    );
}

#[test]
fn usage_errors_exit_2_without_a_record() {
    let table = Table::new("usage");
    for args in [
        vec![],
        vec!["--tty=relative".to_owned()],
        vec!["/dev/tty3".to_owned()],
        vec!["--tty=/dev/tty3".to_owned(), "--extra".to_owned()],
        vec!["--tty=/dev/tty3\nstatus=clear".to_owned()],
    ] {
        let (status, stdout, _) = run(&args, &table.0);
        assert_eq!(status, 2, "{args:?}");
        assert!(stdout.is_empty());
    }
}

/// Root's check-host rule for the checker's result (the contract Sophia
/// implements): 0 with the exact record clears; 1 clears only with an
/// explicit --allow-active; every other status, or a malformed 0, refuses.
fn check_host_accepts(status: i32, stdout: &str, tty: &str, allow_active: bool) -> bool {
    match status {
        0 => stdout == format!("sophia_session_preflight schema=1 status=clear tty={tty}\n"),
        1 => allow_active,
        _ => false,
    }
}

#[test]
fn an_unreadable_process_table_exits_3_and_cannot_be_overridden() {
    let (status, stdout, stderr) = run(&tty(), Path::new("/nonexistent/proc"));
    assert_eq!(status, 3, "{stderr}");
    assert!(stdout.is_empty());
    for allow_active in [false, true] {
        assert!(!check_host_accepts(
            status,
            &stdout,
            "/dev/tty3",
            allow_active
        ));
    }
    // An active session is the only overridable refusal.
    let table = Table::new("override");
    table.process(40, "sway", "S");
    let (status, stdout, _) = run(&tty(), &table.0);
    assert_eq!(status, 1);
    assert!(!check_host_accepts(status, &stdout, "/dev/tty3", false));
    assert!(check_host_accepts(status, &stdout, "/dev/tty3", true));
}

#[test]
fn per_process_inspection_failures_exit_3() {
    use std::os::unix::fs::PermissionsExt;
    let cases: [(&str, &dyn Fn(&Table)); 5] = [
        ("unreadable stat", &|t: &Table| {
            t.process(50, "bash", "S");
            fs::set_permissions(t.0.join("50/stat"), fs::Permissions::from_mode(0o000)).unwrap();
        }),
        ("truncated stat", &|t: &Table| t.raw(51, "51 (bas")),
        ("garbage stat", &|t: &Table| t.raw(52, "garbage\n")),
        ("pid mismatch", &|t: &Table| t.raw(53, "54 (bash) S 1 1\n")),
        ("missing state", &|t: &Table| t.raw(55, "55 (bash)\n")),
    ];
    for (index, (what, setup)) in cases.iter().enumerate() {
        let table = Table::new(&format!("inspect-{index}"));
        table.process(10, "bash", "S");
        setup(&table);
        let (status, stdout, stderr) = run(&tty(), &table.0);
        if *what == "unreadable stat" && fs::read_to_string(table.0.join("50/stat")).is_ok() {
            // Running as root: the permission cannot be denied here.
            continue;
        }
        assert_eq!(status, 3, "{what}: {stderr}");
        assert!(stdout.is_empty(), "{what}");
        assert!(
            !check_host_accepts(status, &stdout, "/dev/tty3", true),
            "{what}"
        );
    }
    // A process that exits between the listing and the read is tolerated.
    let table = Table::new("vanished");
    fs::create_dir(table.0.join("60")).unwrap();
    assert_eq!(run(&tty(), &table.0).0, 0);
}

#[test]
fn the_binary_follows_the_same_contract() {
    let checker = env!("CARGO_BIN_EXE_active-session-preflight");
    let output = Command::new(checker).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let output = Command::new(checker)
        .arg("--tty=/dev/tty63")
        .output()
        .unwrap();
    // On a build host a graphical session may be running (1), or a hardened
    // /proc may deny inspection (3); every verdict must have a contract shape.
    match output.status.code() {
        Some(0) => assert_eq!(
            output.stdout,
            b"sophia_session_preflight schema=1 status=clear tty=/dev/tty63\n"
        ),
        Some(1 | 3) => assert!(output.stdout.is_empty()),
        other => panic!("unexpected status {other:?}"),
    }
}
