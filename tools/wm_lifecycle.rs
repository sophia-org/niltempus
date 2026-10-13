//! Offline current-SDK Hagia lifecycle qualification. Compile with rustc;
//! no dependency on niltempus's older public-crate qualification pin.
//!
//! Run the whole command under one `cargo-slot sophia` lease. Build products
//! (archived sources, Nim cache, Hagia binary) live under BUILD_ROOT, never in
//! the NEW_WORK evidence records, which hold records, sources and hashes only.
use std::os::unix::{
    fs::{MetadataExt, PermissionsExt},
    io::AsRawFd,
    process::CommandExt,
};
use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const SOPHIA: &str = "1b476dbdcc30215fe7ee5841e5a872f136ebb3a0";
const MOUNT: &str = "crates/sophia-session/tests/support/desktop_launch_reload.rs";
/// Written first into a build root this runner created; reuse and cleanup
/// require it.
const MARKER: &str = ".wm-lifecycle-build-root";
const MARKER_HEADER: &str = "owner=niltempus tools/wm_lifecycle.rs";
const EVIDENCE: &str = ".local/state/sophia/development-evidence";
const SLOTS: &str = ".cache/cargo-slots";
const CASES: [&str; 22] = [
    "sdk_lifecycle_startup",
    "sdk_lifecycle_profile_rollback",
    "sdk_lifecycle_restart",
    "sdk_occupied_settlement",
    "sdk_occupied_restart",
    "sdk_occupied_reused_surface_generation_after_restart",
    "sdk_disconnect_aborts_pending_settlement",
    "sdk_occupied_manage_settles_admission",
    "sdk_occupied_manage_admits_cpu_candidate",
    "sdk_occupied_manage_admits_software_present",
    "sdk_operation_settlement",
    "sdk_operation_refused_action",
    "sdk_operation_disconnect",
    "sdk_slow_peer_response_deadline",
    "sdk_presented_chord_switcher",
    "sdk_presented_revocation_closes_switcher",
    "sdk_presented_disconnect_preserves_release_debt",
    "sdk_presented_cancel_preserves_focus",
    "sdk_presented_receipts_require_all_heads",
    "sdk_presented_source_repaint_preserves_identity",
    "sdk_presented_service_withdraws_when_unavailable",
    "sdk_presented_service_revokes_before_requested_restart",
];
/// Production spans the external fixtures mirror or call, found by
/// semantic anchors in the archived pin. A changed hash refuses the run: the
/// copied dispatch and routing glue must be compared again, by review.
const GLUE: [(&str, &str, &str, &str); 5] = [
    (
        "crates/sophia-session/src/live_session/owner_loop/policy_input_phase.rs",
        "macro_rules! dispatch_physical_policy_inputs",
        "PhysicalPolicyInput::Action(action) => action,",
        "e0a546c455e64d7c4678b1277e44eb2c80e4ca5373d89546d4b0db1faa9eb11b",
    ),
    (
        "crates/sophia-session/src/live_session/owner_loop/lifecycle.rs",
        "Chords owe the WM whatever physical input is doing",
        "dispatch_physical_policy_inputs!(&chord_report);",
        "fc05de66b519c123e2bd3cbf9deadfde69cd351a0a11635608f7e7a6a26e7acf",
    ),
    (
        "crates/sophia-session/src/live_session/input/routing.rs",
        "fn route_input_events_with_launcher(",
        ") -> Result<PhysicalInputRouteReport",
        "83aae5ec3331b8e4d69e1b5d1998392bd789d6dd93c5cda588698a6f72be26cb",
    ),
    (
        "crates/sophia-session/src/live_session/wm/presentation.rs",
        "fn service_presented_policy(",
        "fn preflight_staged_presentation(",
        "756783ab90e37f4f48b977c5ea1a87df652c2a954dd38534ca405a6e68b448eb",
    ),
    (
        "crates/sophia-session/src/live_session/owner_loop/session_control.rs",
        "if completion.key.kind == XAuthorityControlKind::SetPresentationState",
        "service_layout_progress!(\"control\");",
        "244a9c1ab6d74142f86a3288be001cecf1452d8f5980b872140e619563e1ffe5",
    ),
];

/// The lines from the one line containing `start` through the first line
/// after it containing `end`.
fn span(text: &str, start: &str, end: &str) -> Result<String> {
    let lines: Vec<_> = text.split('\n').collect();
    let starts: Vec<_> = (0..lines.len())
        .filter(|i| lines[*i].contains(start))
        .collect();
    let [first] = starts[..] else {
        return Err(format!("anchor {start:?} matched {} lines", starts.len()).into());
    };
    let last = (first..lines.len())
        .find(|i| lines[*i].contains(end))
        .ok_or(format!("anchor {end:?} missing"))?;
    Ok(lines[first..=last].join("\n") + "\n")
}

fn sha256(bytes: &[u8]) -> Result<String> {
    use std::io::Write;
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    child.stdin.take().ok_or("stdin")?.write_all(bytes)?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err("sha256sum failed".into());
    }
    Ok(String::from_utf8(out.stdout)?
        .split_whitespace()
        .next()
        .ok_or("hash missing")?
        .into())
}

/// Refuse a pin whose mirrored production spans changed.
fn check_glue(overlay: &Path) -> Result<String> {
    let mut record = String::new();
    for (path, start, end, expected) in GLUE {
        let digest =
            sha256(span(&fs::read_to_string(overlay.join(path))?, start, end)?.as_bytes())?;
        if digest != expected {
            return Err(format!("glue span changed: {path} {digest}").into());
        }
        record.push_str(&format!("{digest}  {path} [{start}] .. [{end}]\n"));
    }
    Ok(record)
}

fn home() -> Result<PathBuf> {
    Ok(PathBuf::from(env::var_os("HOME").ok_or("HOME is unset")?))
}

/// An existing directory, canonical, or the canonical parent joined with a
/// plain final component when it does not exist yet.
fn resolved(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(format!("{} is not absolute", path.display()).into());
    }
    if fs::symlink_metadata(path).is_ok() {
        return Ok(fs::canonicalize(path)?);
    }
    let name = path.file_name().ok_or("path has no final component")?;
    if name == OsStr::new("..") || name == OsStr::new(".") {
        return Err("path ends in a relative component".into());
    }
    Ok(fs::canonicalize(path.parent().ok_or("path has no parent")?)?.join(name))
}

/// The target directory cargo-slot assigned: a direct numbered child of the
/// default sophia pool. This is a path check, not proof of an active lease;
/// the lease is the wrapper process holding its lock for this run.
fn slot_target(pool: &Path, overrides: bool, target: Option<&OsStr>) -> Result<PathBuf> {
    if overrides {
        return Err("CARGO_SLOT_ROOT/CARGO_SLOTS must not be set".into());
    }
    let target =
        fs::canonicalize(target.ok_or("CARGO_TARGET_DIR is unset: run under cargo-slot sophia")?)?;
    let pool = fs::canonicalize(pool)?;
    let numbered = target
        .file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    if target.parent() != Some(pool.as_path()) || !numbered {
        return Err(format!("{} is not a sophia cargo slot", target.display()).into());
    }
    Ok(target)
}

/// The parent process's command line, which must be cargo-slot for sophia.
fn wrapper_record() -> Result<String> {
    let parent = std::os::unix::process::parent_id();
    let mut bytes = Vec::new();
    fs::File::open(format!("/proc/{parent}/cmdline"))?.read_to_end(&mut bytes)?;
    let argv: Vec<String> = bytes
        .split(|b| *b == 0)
        .filter(|a| !a.is_empty())
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    let index = argv
        .iter()
        .position(|a| Path::new(a).file_name() == Some(OsStr::new("cargo-slot")))
        .ok_or("parent is not cargo-slot")?;
    if argv.get(index + 1).map(String::as_str) != Some("sophia") {
        return Err("cargo-slot project is not sophia".into());
    }
    let wrapper = fs::canonicalize(&argv[index])?;
    Ok(format!(
        "wrapper={} sha256={} invocation={argv:?}\n",
        wrapper.display(),
        hash(&wrapper)?
    ))
}

fn inside_checkout(path: &Path) -> bool {
    let mut probe = path.to_path_buf();
    while fs::symlink_metadata(&probe).is_err() {
        if !probe.pop() {
            return false;
        }
    }
    Command::new("git")
        .arg("-C")
        .arg(&probe)
        .args(["rev-parse", "--is-inside-work-tree"])
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|o| o.status.success() && o.stdout.starts_with(b"true"))
}

fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// Refuse a build root that overlaps, in either direction, the records, the
/// evidence directory, the cargo slots, any other protected path (Cargo home,
/// supplied checkouts, this executable, HOME itself), or that lies inside a
/// Git checkout. Records must not overlap the slots or protected paths
/// either. Paths are resolved through existing parents, so a symlinked
/// parent cannot hide an overlap.
fn check_roots(
    work: &Path,
    build: &Path,
    evidence: &Path,
    slots: &Path,
    protected: &[PathBuf],
) -> Result<()> {
    if overlaps(work, build) {
        return Err("NEW_WORK and BUILD_ROOT overlap".into());
    }
    if overlaps(build, evidence) {
        return Err("BUILD_ROOT overlaps the evidence directory".into());
    }
    if overlaps(build, slots) || overlaps(work, slots) {
        return Err("a root overlaps the cargo slots".into());
    }
    for path in protected {
        if overlaps(build, path) || overlaps(work, path) {
            return Err(format!("a root overlaps protected {}", path.display()).into());
        }
    }
    if inside_checkout(build) {
        return Err("BUILD_ROOT is inside a Git checkout".into());
    }
    Ok(())
}

/// One exclusive, nonblocking lease per build root, on a stable sibling lock
/// file that is never unlinked, so every holder locks the same inode. It is
/// flock(2), so external controls and checks can hold it with flock(1). Std
/// opens it close-on-exec; only steps given it by `inherit_lease` keep it.
/// Run, cleanup and controls take it before any marker read, reuse or
/// deletion.
fn lease_build_root(build: &Path) -> Result<(fs::File, String)> {
    let name = build.file_name().ok_or("BUILD_ROOT has no name")?;
    let lock = build
        .parent()
        .ok_or("BUILD_ROOT has no parent")?
        .join(format!(".{}.wm-lifecycle.lock", name.to_string_lossy()));
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&lock)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(fs::TryLockError::WouldBlock) => {
            return Err(format!("BUILD_ROOT is busy: {} is held", lock.display()).into());
        }
        Err(fs::TryLockError::Error(e)) => return Err(e.into()),
    }
    let meta = file.metadata()?;
    let identity = format!(
        "build_root_lock={} dev={} ino={}\n",
        lock.display(),
        meta.dev(),
        meta.ino()
    );
    Ok((file, identity))
}

unsafe extern "C" {
    fn fcntl(fd: i32, cmd: i32, ...) -> i32;
}
const F_SETFD: i32 = 2;

/// Hand the lease to a child that writes the build root, so it outlives a
/// killed runner.
///
/// timeout puts itself in its own process group, so a runner-only SIGKILL or
/// a terminal interrupt leaves the step running in BUILD_ROOT. A flock
/// belongs to the open file description and ends only when its last
/// descriptor closes, so the inherited copy keeps the root leased until the
/// step's whole process chain exits. timeout and nice keep descriptors across
/// exec; bwrap may close extras in the sandboxed command, but timeout holds
/// its copy until bwrap exits, and with --unshare-pid every sandboxed
/// process ends with bwrap. No step here starts a daemon outside that
/// sandbox. Source archive writers also inherit it; read-only helpers do not.
fn inherit_lease(command: &mut Command, lease: &fs::File) {
    let fd = lease.as_raw_fd();
    // SAFETY: fcntl is async-signal-safe and clears close-on-exec only in
    // the forked child's descriptor table; the runner's copy is unchanged.
    unsafe {
        command.pre_exec(move || {
            if fcntl(fd, F_SETFD, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

fn marker_text(protected: &[PathBuf]) -> String {
    let mut text = format!("{MARKER_HEADER}\n");
    for path in protected {
        text.push_str(&format!("protect={}\n", path.display()));
    }
    text
}

/// The protected paths a marked root's creator recorded; None if unmarked.
fn marked(build: &Path) -> Option<Vec<PathBuf>> {
    let text = fs::read_to_string(build.join(MARKER)).ok()?;
    let mut lines = text.lines();
    (lines.next()? == MARKER_HEADER).then(|| {
        lines
            .filter_map(|l| l.strip_prefix("protect=").map(PathBuf::from))
            .collect()
    })
}

/// Create the build root, or reuse one this runner marked. The caller holds
/// the build-root lease.
fn own_build_root(build: &Path, protected: &[PathBuf]) -> Result<()> {
    if fs::symlink_metadata(build).is_ok() {
        if marked(build).is_none() {
            return Err("existing BUILD_ROOT is not marked as this runner's".into());
        }
        return Ok(());
    }
    fs::create_dir(build)?;
    fs::set_permissions(build, fs::Permissions::from_mode(0o700))?;
    fs::write(build.join(MARKER), marker_text(protected))?;
    Ok(())
}

/// What a record directory may hold: regular files at the top and one level
/// of case directories of regular files. Symlinks, other directories and
/// executable or object files are violations; so is anything that cannot be
/// inspected. Size is not a criterion.
fn record_violations(work: &Path) -> Vec<String> {
    fn artifact(path: &Path) -> std::io::Result<bool> {
        let mut head = [0u8; 8];
        let read = fs::File::open(path)?.read(&mut head)?;
        let name = path.extension().and_then(OsStr::to_str).unwrap_or("");
        Ok(head[..read].starts_with(b"\x7fELF")
            || head[..read].starts_with(b"!<arch>")
            || ["rlib", "rmeta", "o", "so", "a", "d"].contains(&name)
            || fs::metadata(path)?.mode() & 0o111 != 0)
    }
    fn file(out: &mut Vec<String>, path: &Path, rel: &str) {
        match artifact(path) {
            Ok(true) => out.push(format!("artifact {rel}")),
            Ok(false) => {}
            Err(e) => out.push(format!("inspection error {rel}: {e}")),
        }
    }
    fn entries(
        out: &mut Vec<String>,
        dir: &Path,
        rel: &str,
    ) -> Vec<(String, PathBuf, fs::FileType)> {
        let mut found = Vec::new();
        match fs::read_dir(dir) {
            Ok(iter) => {
                for entry in iter {
                    match entry.and_then(|e| Ok((e.file_name(), e.path(), e.file_type()?))) {
                        Ok((name, path, kind)) => {
                            found.push((format!("{rel}{}", name.to_string_lossy()), path, kind));
                        }
                        Err(e) => out.push(format!("inspection error {rel}: {e}")),
                    }
                }
            }
            Err(e) => out.push(format!("inspection error {rel}: {e}")),
        }
        found
    }
    let mut out = Vec::new();
    for (rel, path, kind) in entries(&mut out, work, "") {
        if kind.is_symlink() {
            out.push(format!("symlink {rel}"));
        } else if kind.is_file() {
            file(&mut out, &path, &rel);
        } else if rel == "cases" && kind.is_dir() {
            for (case, path, kind) in entries(&mut out, &path, "cases/") {
                if !kind.is_dir() {
                    out.push(format!("cases entry {case}"));
                    continue;
                }
                for (rel, path, kind) in entries(&mut out, &path, &format!("{case}/")) {
                    if !kind.is_file() {
                        out.push(format!("not a file {rel}"));
                    } else {
                        file(&mut out, &path, &rel);
                    }
                }
            }
        } else {
            out.push(format!("directory {rel}"));
        }
    }
    out.sort();
    out
}

/// Record the outcome and close the records. The original failure and any
/// record-policy violation are both kept; a closure that cannot hash every
/// record turns the result into STOP and still writes what it could.
fn finish(work: &Path, result: Result<()>, pass: &str) -> Result<()> {
    let violations = record_violations(work);
    let mut policy = format!("violations={}\n", violations.len());
    for line in violations.iter().take(200) {
        policy.push_str(line);
        policy.push('\n');
    }
    let mut result = match result {
        Ok(()) if !violations.is_empty() => Err(format!(
            "record policy: {} violation(s); see record-policy",
            violations.len()
        )
        .into()),
        Err(e) if !violations.is_empty() => Err(format!(
            "{e}; record policy: {} violation(s); see record-policy",
            violations.len()
        )
        .into()),
        other => other,
    };
    let written = fs::write(work.join("record-policy"), policy).and_then(|()| {
        fs::write(
            work.join("RESULT"),
            match &result {
                Ok(()) => pass.to_owned(),
                Err(e) => format!("STOP {e}\n"),
            },
        )
    });
    if let Err(e) = written {
        return Err(format!("records not written: {e}").into());
    }
    if let Err(e) = close_records(work) {
        let prior = match &result {
            Ok(()) => "none".to_owned(),
            Err(prior) => prior.to_string(),
        };
        result = Err(format!("record closure failed: {e}; prior: {prior}").into());
        fs::write(
            work.join("RESULT"),
            format!("STOP {}\n", result.as_ref().unwrap_err()),
        )?;
        let _ = close_records(work);
    }
    result
}

fn checked(command: &mut Command) -> Result<String> {
    let out = command.output()?;
    if !out.status.success() {
        return Err(format!("{command:?}: {}", String::from_utf8_lossy(&out.stderr)).into());
    }
    Ok(String::from_utf8(out.stdout)?)
}
fn hash(path: &Path) -> Result<String> {
    Ok(checked(Command::new("sha256sum").arg(path))?
        .split_whitespace()
        .next()
        .ok_or("hash missing")?
        .into())
}
fn git(root: &Path, args: &[&str]) -> Result<String> {
    checked(Command::new("git").arg("-C").arg(root).args(args)).map(|s| s.trim().into())
}

fn archive(root: &Path, revision: &str, destination: &Path, lease: &fs::File) -> Result<()> {
    let tar = destination.with_extension("tar");
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(["archive", "--format=tar"])
        .arg(format!("--output={}", tar.display()))
        .arg(revision);
    inherit_lease(&mut command, lease);
    checked(&mut command)?;
    fs::create_dir(destination)?;
    let mut command = Command::new("tar");
    // Git's historical mtimes can make a restored source look older than a
    // compiled control mutant. Refresh mtimes on every extraction so Cargo
    // must reconsider these workspace inputs before reusing slot artifacts.
    command
        .arg("--touch")
        .arg("-xf")
        .arg(&tar)
        .arg("-C")
        .arg(destination);
    inherit_lease(&mut command, lease);
    checked(&mut command)?;
    fs::remove_file(tar)?;
    Ok(())
}
fn log_step(work: &Path, name: &str, command: &mut Command) -> Result<()> {
    fs::write(work.join(format!("{name}.argv")), format!("{command:?}\n"))?;
    let log = fs::File::create(work.join(format!("{name}.log")))?;
    let status = command
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .status()?;
    fs::write(
        work.join(format!("{name}.exit")),
        format!("{}\n", status.code().unwrap_or(128)),
    )?;
    if !status.success() {
        return Err(format!("{name} failed: {status}; see {}", work.display()).into());
    }
    Ok(())
}

struct Run<'a> {
    work: PathBuf,
    build: PathBuf,
    target: PathBuf,
    cargo: PathBuf,
    lease: &'a fs::File,
}
impl Run<'_> {
    fn isolated(&self, cwd: &Path, seconds: u32, program: &str, args: &[String]) -> Command {
        let mut cmd = Command::new("timeout");
        cmd.args(["--signal=TERM", "--kill-after=5"])
            .arg(seconds.to_string())
            .arg("nice")
            .args(["-n", "10", "bwrap"])
            .args([
                "--unshare-net",
                "--unshare-pid",
                "--die-with-parent",
                "--ro-bind",
                "/",
                "/",
                "--dev",
                "/dev",
                "--proc",
                "/proc",
                "--tmpfs",
                "/tmp",
                "--tmpfs",
                "/run/user",
            ]);
        for writable in [&self.work, &self.build, &self.target, &self.cargo] {
            cmd.arg("--bind").arg(writable).arg(writable);
        }
        cmd.arg("--chdir")
            .arg(cwd)
            .arg("--")
            .arg(program)
            .args(args);
        for (key, _) in env::vars_os() {
            let key_s = key.to_string_lossy();
            if key_s.starts_with("SOPHIA_")
                || key_s.starts_with("HAGIA_")
                || [
                    "DISPLAY",
                    "WAYLAND_DISPLAY",
                    "XAUTHORITY",
                    "DBUS_SESSION_BUS_ADDRESS",
                    "SSH_AUTH_SOCK",
                ]
                .contains(&key_s.as_ref())
            {
                cmd.env_remove(key);
            }
        }
        cmd.env("CARGO_HOME", &self.cargo)
            .env("CARGO_TARGET_DIR", &self.target)
            .env("CARGO_BUILD_JOBS", "4")
            .env("RUST_TEST_THREADS", "1")
            .env("CARGO_INCREMENTAL", "0")
            .env("CARGO_PROFILE_DEV_DEBUG", "0")
            .env("CARGO_PROFILE_TEST_DEBUG", "0")
            .env("RUSTUP_TOOLCHAIN", "1.96.1");
        inherit_lease(&mut cmd, self.lease);
        cmd
    }
}

fn listed_tests(text: &str) -> Result<Vec<String>> {
    CASES
        .iter()
        .map(|case| {
            let names: Vec<_> = text
                .lines()
                .filter_map(|l| l.strip_suffix(": test"))
                .filter(|l| l.ends_with(&format!("::{case}")))
                .collect();
            if names.len() != 1 {
                return Err(format!("expected one listed {case}, got {}", names.len()).into());
            }
            Ok(names[0].to_owned())
        })
        .collect()
}

fn close_records(work: &Path) -> Result<()> {
    let mut files = Vec::new();
    for entry in fs::read_dir(work)? {
        let entry = entry?;
        if entry.file_type()?.is_file() && entry.file_name() != "SHA256SUMS" {
            files.push(entry.path());
        }
    }
    if work.join("cases").is_dir() && !work.join("cases").is_symlink() {
        for case in fs::read_dir(work.join("cases"))? {
            let case = case?;
            if !case.file_type()?.is_dir() {
                continue;
            }
            for entry in fs::read_dir(case.path())? {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    files.push(entry.path());
                }
            }
        }
    }
    files.sort();
    let mut manifest = String::new();
    let mut unhashed = Vec::new();
    for file in files {
        match hash(&file) {
            Ok(digest) => manifest.push_str(&format!(
                "{digest}  {}\n",
                file.strip_prefix(work)?.display()
            )),
            Err(_) => unhashed.push(file.strip_prefix(work)?.display().to_string()),
        }
    }
    fs::write(work.join("SHA256SUMS"), manifest)?;
    if !unhashed.is_empty() {
        return Err(format!("unhashable records: {}", unhashed.join(", ")).into());
    }
    checked(Command::new("sha256sum").current_dir(work).args([
        "--check",
        "--quiet",
        "SHA256SUMS",
    ]))?;
    Ok(())
}

const USAGE: &str = "usage: cargo-slot sophia wm-lifecycle SOPHIA_CHECKOUT HAGIA_CHECKOUT NEW_WORK BUILD_ROOT CARGO_HOME\n       wm-lifecycle cleanup BUILD_ROOT NEW_CLEANUP_RECORD";

fn run() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.first().map(|a| a.as_os_str()) == Some(OsStr::new("cleanup")) {
        if args.len() != 3 {
            return Err(USAGE.into());
        }
        let home = home()?;
        return cleanup(
            &resolved(Path::new(&args[1]))?,
            &resolved(Path::new(&args[2]))?,
            &resolved(&home.join(EVIDENCE))?,
            &resolved(&home.join(SLOTS))?,
            &fs::canonicalize(&home)?,
            &fs::canonicalize(env::current_exe()?)?,
        );
    }
    if args.len() != 5 {
        return Err(USAGE.into());
    }
    let sophia = fs::canonicalize(&args[0])?;
    let hagia = fs::canonicalize(&args[1])?;
    let work = resolved(Path::new(&args[2]))?;
    if fs::symlink_metadata(&work).is_ok() {
        return Err("NEW_WORK must not exist".into());
    }
    let build = resolved(Path::new(&args[3]))?;
    let cargo = fs::canonicalize(&args[4])?;
    let home = home()?;
    let slots = resolved(&home.join(SLOTS))?;
    let protected = vec![
        cargo.clone(),
        sophia.clone(),
        hagia.clone(),
        fs::canonicalize(env::current_exe()?)?,
    ];
    if fs::canonicalize(&home)?.starts_with(&build) {
        return Err("BUILD_ROOT contains HOME".into());
    }
    check_roots(
        &work,
        &build,
        &resolved(&home.join(EVIDENCE))?,
        &slots,
        &protected,
    )?;
    let target = slot_target(
        &slots.join("sophia"),
        env::var_os("CARGO_SLOT_ROOT").is_some() || env::var_os("CARGO_SLOTS").is_some(),
        env::var_os("CARGO_TARGET_DIR").as_deref(),
    )?;
    let wrapper = wrapper_record()?;
    if git(&sophia, &["rev-parse", "HEAD"])? != SOPHIA {
        return Err("unsupported Sophia source pin".into());
    }
    if !git(&sophia, &["status", "--porcelain"])?.is_empty() {
        return Err("dirty Sophia source".into());
    }
    if !git(
        &hagia,
        &[
            "status",
            "--porcelain",
            "--",
            "src",
            "vendor",
            "hagia.nimble",
        ],
    )?
    .is_empty()
    {
        return Err("dirty Hagia production inputs".into());
    }
    // The overlay is deliberately external. Record the Hagia working-tree
    // diff too: a development run cannot silently become signed qualification.
    // The build-root lease comes before any mutation; busy roots are refused.
    let (lease, lease_identity) = lease_build_root(&build)?;
    fs::create_dir(&work)?;
    fs::set_permissions(&work, fs::Permissions::from_mode(0o700))?;
    let result = (|| -> Result<()> {
        own_build_root(&build, &protected)?;
        if fs::canonicalize(&work)? != work || fs::canonicalize(&build)? != build {
            return Err("a root resolved differently after creation".into());
        }
        let run_name = work.file_name().ok_or("NEW_WORK has no name")?;
        let products = build.join("runs").join(run_name);
        fs::create_dir_all(build.join("runs"))?;
        fs::create_dir(&products)?;
        fs::write(
            work.join("resources"),
            format!(
                "{wrapper}cargo_target_dir={} (cargo-slot path check only; the lease is the wrapper's lock, not verified here)\n{lease_identity}build_root={}\nrun_products={}\ncargo_home={}\n",
                target.display(),
                build.display(),
                products.display(),
                cargo.display()
            ),
        )?;
        let fixture = hagia.join("tests/external/lifecycle.rs");
        let occupied = hagia.join("tests/external/occupied.rs");
        let reused_identity = hagia.join("tests/external/reused_identity.rs");
        let operations = hagia.join("tests/external/operations.rs");
        let slow_peer = hagia.join("tests/external/slow_peer.rs");
        let presentation = hagia.join("tests/external/presentation.rs");
        let revocation = hagia.join("tests/external/presentation_revocation.rs");
        let disconnect = hagia.join("tests/external/presentation_disconnect.rs");
        let cancel = hagia.join("tests/external/presentation_cancel.rs");
        let abort = hagia.join("tests/external/abort_settlement.rs");
        let manage = hagia.join("tests/external/manage.rs");
        let manage_pixels = hagia.join("tests/external/manage_pixels.rs");
        let consensus = hagia.join("tests/external/presentation_consensus.rs");
        let repaint = hagia.join("tests/external/presentation_repaint.rs");
        let service = hagia.join("tests/external/presentation_service.rs");
        fs::write(
            work.join("runner.sha256"),
            hash(&env::current_exe()?)? + "\n",
        )?;
        for tool in ["nim", "cc", "bwrap"] {
            fs::write(
                work.join(format!("{tool}.version")),
                checked(Command::new(tool).arg("--version"))?,
            )?;
        }
        fs::write(
            work.join("rustc.version"),
            checked(Command::new("rustc").args(["+1.96.1", "-Vv"]))?,
        )?;
        fs::write(
            work.join("identity"),
            format!(
                "sophia={SOPHIA}\nhagia={}\nfixture_sha256={}\nmount_sha256={}\nscene=empty_and_supplied_occupied\nlayout_completion=supplied\nadmission=protected_supervisor_launch_factotum\nfactotum_session_channel=supplied_trusted\nautomatic_restart_trigger=terminate_fixture_child\nslow_peer_trigger=sigstop_fixture_peer_production_response_deadline\npresented_case=sdk_presented_chord_switcher native_ceiling=selected_without_device presentation_completion=supplied surface_content=supplied_cpu_intake heads=supplied router_time=supplied admission_visual_readiness=bypassed\nnative=false\n",
                git(&hagia, &["rev-parse", "HEAD"])?,
                hash(&fixture)?,
                hash(&sophia.join(MOUNT))?
            ),
        )?;
        fs::write(
            work.join("hagia.diff"),
            git(&hagia, &["diff", "HEAD", "--binary"])?,
        )?;
        fs::write(
            work.join("hagia.status"),
            git(&hagia, &["status", "--porcelain"])?,
        )?;
        fs::copy(&fixture, work.join("lifecycle.rs"))?;
        fs::copy(&occupied, work.join("occupied.rs"))?;
        fs::copy(&reused_identity, work.join("reused_identity.rs"))?;
        fs::copy(&operations, work.join("operations.rs"))?;
        fs::copy(&slow_peer, work.join("slow_peer.rs"))?;
        fs::copy(&presentation, work.join("presentation.rs"))?;
        fs::copy(&revocation, work.join("presentation_revocation.rs"))?;
        fs::copy(&disconnect, work.join("presentation_disconnect.rs"))?;
        fs::copy(&cancel, work.join("presentation_cancel.rs"))?;
        fs::copy(&abort, work.join("abort_settlement.rs"))?;
        fs::copy(&manage, work.join("manage.rs"))?;
        fs::copy(&manage_pixels, work.join("manage_pixels.rs"))?;
        fs::copy(&consensus, work.join("presentation_consensus.rs"))?;
        fs::copy(&repaint, work.join("presentation_repaint.rs"))?;
        fs::copy(&service, work.join("presentation_service.rs"))?;
        // One build workspace per slice: re-extracted at the same path so
        // cargo reuses the slot's unchanged artifacts.
        let overlay = build.join("sophia");
        if fs::symlink_metadata(&overlay).is_ok() {
            fs::remove_dir_all(&overlay)?;
        }
        archive(&sophia, SOPHIA, &overlay, &lease)?;
        let hagia_source = products.join("hagia-source");
        archive(
            &hagia,
            &git(&hagia, &["rev-parse", "HEAD"])?,
            &hagia_source,
            &lease,
        )?;
        fs::copy(
            hagia_source.join("vendor/sophia-desktop-sdk/manifest.json"),
            work.join("sdk-manifest.json"),
        )?;
        fs::write(
            work.join("sdk-manifest.sha256"),
            hash(&work.join("sdk-manifest.json"))? + "\n",
        )?;
        let module = overlay.join("crates/sophia-session/tests/support/hagia_sdk_lifecycle.rs");
        fs::copy(&fixture, &module)?;
        let occupied_module = module.with_file_name("occupied.rs");
        fs::copy(&occupied, &occupied_module)?;
        let reused_identity_module = module.with_file_name("reused_identity.rs");
        if reused_identity_module.exists() {
            return Err("reused identity module would replace a pinned file".into());
        }
        fs::copy(&reused_identity, &reused_identity_module)?;
        let operations_module = module.with_file_name("operations.rs");
        fs::copy(&operations, &operations_module)?;
        let slow_peer_module = module.with_file_name("slow_peer.rs");
        fs::copy(&slow_peer, &slow_peer_module)?;
        let presentation_module = module.with_file_name("presentation.rs");
        if presentation_module.exists() {
            return Err("presentation module would replace a pinned file".into());
        }
        fs::copy(&presentation, &presentation_module)?;
        let revocation_module = module.with_file_name("presentation_revocation.rs");
        if revocation_module.exists() {
            return Err("revocation module would replace a pinned file".into());
        }
        fs::copy(&revocation, &revocation_module)?;
        let disconnect_module = module.with_file_name("presentation_disconnect.rs");
        if disconnect_module.exists() {
            return Err("disconnect module would replace a pinned file".into());
        }
        fs::copy(&disconnect, &disconnect_module)?;
        let cancel_module = module.with_file_name("presentation_cancel.rs");
        if cancel_module.exists() {
            return Err("cancel module would replace a pinned file".into());
        }
        fs::copy(&cancel, &cancel_module)?;
        let abort_module = module.with_file_name("abort_settlement.rs");
        if abort_module.exists() {
            return Err("abort module would replace a pinned file".into());
        }
        fs::copy(&abort, &abort_module)?;
        let manage_module = module.with_file_name("manage.rs");
        if manage_module.exists() {
            return Err("manage module would replace a pinned file".into());
        }
        fs::copy(&manage, &manage_module)?;
        let manage_pixels_module = module.with_file_name("manage_pixels.rs");
        if manage_pixels_module.exists() {
            return Err("manage pixels module would replace a pinned file".into());
        }
        fs::copy(&manage_pixels, &manage_pixels_module)?;
        let consensus_module = module.with_file_name("presentation_consensus.rs");
        if consensus_module.exists() {
            return Err("presentation consensus module would replace a pinned file".into());
        }
        fs::copy(&consensus, &consensus_module)?;
        let repaint_module = module.with_file_name("presentation_repaint.rs");
        if repaint_module.exists() {
            return Err("presentation repaint module would replace a pinned file".into());
        }
        fs::copy(&repaint, &repaint_module)?;
        let service_module = module.with_file_name("presentation_service.rs");
        if service_module.exists() {
            return Err("presentation service module would replace a pinned file".into());
        }
        fs::copy(&service, &service_module)?;
        fs::write(work.join("glue.sha256"), check_glue(&overlay)?)?;
        let mount = overlay.join(MOUNT);
        let original = fs::read_to_string(&mount)?;
        fs::write(
            &mount,
            format!("{original}\n#[path = \"hagia_sdk_lifecycle.rs\"]\nmod hagia_sdk_lifecycle;\n"),
        )?;
        fs::write(
            work.join("overlay.sha256"),
            format!(
                "{}  {MOUNT}\n{}  hagia_sdk_lifecycle.rs\n{}  occupied.rs\n{}  reused_identity.rs\n{}  operations.rs\n{}  slow_peer.rs\n{}  presentation.rs\n{}  presentation_revocation.rs\n{}  presentation_disconnect.rs\n{}  presentation_cancel.rs\n{}  abort_settlement.rs\n{}  manage.rs\n{}  manage_pixels.rs\n{}  presentation_consensus.rs\n{}  presentation_repaint.rs\n{}  presentation_service.rs\n",
                hash(&mount)?,
                hash(&module)?,
                hash(&occupied_module)?,
                hash(&reused_identity_module)?,
                hash(&operations_module)?,
                hash(&slow_peer_module)?,
                hash(&presentation_module)?,
                hash(&revocation_module)?,
                hash(&disconnect_module)?,
                hash(&cancel_module)?,
                hash(&abort_module)?,
                hash(&manage_module)?,
                hash(&manage_pixels_module)?,
                hash(&consensus_module)?,
                hash(&repaint_module)?,
                hash(&service_module)?
            ),
        )?;
        let run = Run {
            work: work.clone(),
            build: build.clone(),
            target,
            cargo,
            lease: &lease,
        };
        let binary = products.join("hagia");
        let nim_args = vec![
            "c".into(),
            "--hints:off".into(),
            "--parallelBuild:1".into(),
            "-d:release".into(),
            format!("--nimcache:{}", products.join("nimcache").display()),
            format!("--out:{}", binary.display()),
            "src/hagia.nim".into(),
        ];
        log_step(
            &work,
            "build-hagia",
            &mut run.isolated(&hagia_source, 300, "nim", &nim_args),
        )?;
        let digest = hash(&binary)?;
        fs::write(
            work.join("hagia.sha256"),
            format!(
                "{digest}  {}\nsize={}\n",
                binary.display(),
                fs::metadata(&binary)?.len()
            ),
        )?;
        let base = [
            "test",
            "--offline",
            "--locked",
            "--features",
            "native-session",
            "-p",
            "sophia-session",
            "--lib",
            "hagia_sdk_lifecycle",
            "--",
        ];
        let mut list: Vec<String> = base.iter().map(|s| (*s).into()).collect();
        list.extend(["--ignored".into(), "--list".into()]);
        log_step(
            &work,
            "list",
            &mut run.isolated(&overlay, 1200, "cargo", &list),
        )?;
        let tests = listed_tests(&fs::read_to_string(work.join("list.log"))?)?;
        fs::write(work.join("tests.txt"), tests.join("\n") + "\n")?;
        let evidence = work.join("cases");
        fs::create_dir(&evidence)?;
        for (case, test) in CASES.iter().zip(tests) {
            let args: Vec<String> = [
                "test",
                "--offline",
                "--locked",
                "--features",
                "native-session",
                "-p",
                "sophia-session",
                "--lib",
                &test,
                "--",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ]
            .iter()
            .map(|s| (*s).into())
            .collect();
            let mut command = run.isolated(&overlay, 90, "cargo", &args);
            command
                .env("HAGIA_LIFECYCLE_BIN", &binary)
                .env("HAGIA_LIFECYCLE_SHA256", &digest)
                .env("HAGIA_LIFECYCLE_EVIDENCE", &evidence);
            log_step(&work, case, &mut command)?;
            let log = fs::read_to_string(work.join(format!("{case}.log")))?;
            if !log.contains("test result: ok. 1 passed; 0 failed; 0 ignored;") {
                return Err(format!("{case}: missing one-test result").into());
            }
        }
        if hash(&fixture)? != hash(&module)?
            || hash(&occupied)? != hash(&occupied_module)?
            || hash(&reused_identity)? != hash(&reused_identity_module)?
            || hash(&operations)? != hash(&operations_module)?
            || hash(&slow_peer)? != hash(&slow_peer_module)?
            || hash(&presentation)? != hash(&presentation_module)?
            || hash(&revocation)? != hash(&revocation_module)?
            || hash(&disconnect)? != hash(&disconnect_module)?
            || hash(&cancel)? != hash(&cancel_module)?
            || hash(&abort)? != hash(&abort_module)?
            || hash(&manage)? != hash(&manage_module)?
            || hash(&manage_pixels)? != hash(&manage_pixels_module)?
            || hash(&consensus)? != hash(&consensus_module)?
            || hash(&repaint)? != hash(&repaint_module)?
            || hash(&service)? != hash(&service_module)?
            || hash(&binary)? != digest
        {
            return Err("input changed during run".into());
        }
        Ok(())
    })();
    let result = finish(
        &work,
        result,
        "PASS lifecycle=3 occupied=7 operations=3 slow_peer=1 presented=8 native=false\n",
    );
    // Supervised steps have been waited for; the lease ends after closure.
    drop(lease);
    result
}

/// Remove exactly one marked build root once its slice's runs, controls and
/// checks have ended, recording what was removed in a new record directory.
fn cleanup(
    build: &Path,
    record: &Path,
    evidence: &Path,
    slots: &Path,
    home: &Path,
    exe: &Path,
) -> Result<()> {
    if fs::symlink_metadata(record).is_ok() {
        return Err("cleanup record must not exist".into());
    }
    if !build.is_dir() || fs::canonicalize(build)? != build {
        return Err("BUILD_ROOT is not a canonical directory".into());
    }
    if home.starts_with(build) {
        return Err("BUILD_ROOT contains HOME".into());
    }
    check_roots(record, build, evidence, slots, &[exe.to_path_buf()])?;
    // The lease before the marker is read, held through removal and closure.
    let (lease, lease_identity) = lease_build_root(build)?;
    let mut protected = marked(build).ok_or("BUILD_ROOT is not marked as this runner's")?;
    protected.push(exe.to_path_buf());
    check_roots(record, build, evidence, slots, &protected)?;
    // Read-only check that no process of ours still runs from or in the root.
    let uid = fs::metadata("/proc/self")?.uid();
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        if !entry
            .file_name()
            .to_string_lossy()
            .bytes()
            .all(|b| b.is_ascii_digit())
            || entry.metadata().map(|m| m.uid()).ok() != Some(uid)
        {
            continue;
        }
        for link in ["cwd", "exe"] {
            if fs::read_link(entry.path().join(link)).is_ok_and(|p| p.starts_with(build)) {
                return Err(format!(
                    "process {} still uses BUILD_ROOT",
                    entry.file_name().to_string_lossy()
                )
                .into());
            }
        }
    }
    let listing = checked(
        Command::new("du")
            .args(["--bytes", "--max-depth=2"])
            .arg(build),
    )?;
    fs::create_dir(record)?;
    fs::set_permissions(record, fs::Permissions::from_mode(0o700))?;
    fs::write(
        record.join("removed"),
        format!("build_root={}\n{lease_identity}{listing}", build.display()),
    )?;
    let removal = fs::remove_dir_all(build);
    let gone = fs::symlink_metadata(build).is_err();
    fs::write(
        record.join("RESULT"),
        match (&removal, gone) {
            (Ok(()), true) => "PASS build root removed\n".to_owned(),
            (removal, _) => format!("STOP removal {removal:?} gone={gone}\n"),
        },
    )?;
    close_records(record)?;
    drop(lease);
    removal?;
    if !gone {
        return Err("build root still present".into());
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_refreshes_source_times_after_a_control_build() {
        use std::time::{Duration, SystemTime};
        let root = scratch("archive-times");
        let repo = root.join("repo");
        checked(Command::new("git").args(["init", "--quiet"]).arg(&repo)).unwrap();
        fs::write(repo.join("source.rs"), "restored source\n").unwrap();
        git(&repo, &["add", "source.rs"]).unwrap();
        let tree = git(&repo, &["write-tree"]).unwrap();
        let revision = checked(
            Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args([
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@invalid",
                    "commit-tree",
                    &tree,
                    "-m",
                    "historical archive fixture",
                ])
                .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
                .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z"),
        )
        .unwrap();
        let prior_build = SystemTime::now() - Duration::from_secs(2);
        let destination = root.join("source");
        let (lease, _) = lease_build_root(&root.join("build")).unwrap();
        archive(&repo, revision.trim(), &destination, &lease).unwrap();
        let source = destination.join("source.rs");
        assert_eq!(fs::read_to_string(&source).unwrap(), "restored source\n");
        assert!(
            fs::metadata(&source).unwrap().modified().unwrap() > prior_build,
            "archived sources must be newer than artifacts from a prior control"
        );
        drop(lease);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_zero_or_duplicate_test_listing_refuses() {
        for value in [
            "",
            "0 tests, 0 benchmarks",
            "x::sdk_lifecycle_startup: test\n",
        ] {
            assert!(listed_tests(value).is_err());
        }
        let text = CASES
            .iter()
            .map(|c| format!("owner::{c}: test\n"))
            .collect::<String>();
        assert_eq!(listed_tests(&text).unwrap().len(), CASES.len());
        assert!(listed_tests(&(text.clone() + &text)).is_err());
    }

    #[test]
    fn glue_spans_need_one_start_and_an_end() {
        let text = "a\nstart x\nb\nend y\nc\n";
        assert_eq!(span(text, "start", "end").unwrap(), "start x\nb\nend y\n");
        assert!(span("start\nstart\nend\n", "start", "end").is_err());
        assert!(span("start\nb\n", "start", "end").is_err());
        assert!(span("b\nend\n", "start", "end").is_err());
    }

    fn scratch(name: &str) -> PathBuf {
        let root = env::temp_dir().join(format!("wm-lifecycle-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        fs::canonicalize(root).unwrap()
    }

    #[test]
    fn only_a_numbered_default_sophia_slot_is_a_target() {
        let root = scratch("slots");
        let pool = root.join("sophia");
        for dir in ["1", "x"] {
            fs::create_dir_all(pool.join(dir)).unwrap();
        }
        let elsewhere = root.join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        let one = pool.join("1");
        assert_eq!(
            slot_target(&pool, false, Some(one.as_os_str())).unwrap(),
            one
        );
        assert!(slot_target(&pool, true, Some(one.as_os_str())).is_err());
        assert!(slot_target(&pool, false, None).is_err());
        assert!(slot_target(&pool, false, Some(pool.join("x").as_os_str())).is_err());
        assert!(slot_target(&pool, false, Some(elsewhere.as_os_str())).is_err());
        assert!(slot_target(&pool, false, Some(pool.as_os_str())).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn overlapping_or_misplaced_roots_refuse() {
        let root = scratch("roots");
        let evidence = root.join("state/evidence");
        let slots = root.join("cache/slots");
        let cargo = root.join("cache/cargo-home");
        let sophia = root.join("dev/sophia");
        let exe = root.join("bin/runner");
        for dir in [&evidence, &slots, &cargo, &sophia, &root.join("bin")] {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(&exe, "sentinel").unwrap();
        let protected = [cargo.clone(), sophia.clone(), exe.clone()];
        let roots =
            |work: &Path, build: &Path| check_roots(work, build, &evidence, &slots, &protected);
        let work = evidence.join("run");
        let build = root.join("scratch/build");
        fs::create_dir_all(root.join("scratch")).unwrap();
        assert!(roots(&work, &build).is_ok());
        // Descendants, ancestors and equality, in both directions.
        for bad in [
            work.join("b"),
            work.clone(),
            evidence.join("b"),
            evidence.clone(),
            root.join("state"),
            slots.join("sophia/1/b"),
            slots.clone(),
            root.join("cache"),
            cargo.join("b"),
            sophia.join("b"),
            sophia.clone(),
            root.join("dev"),
            root.join("bin"),
            root.clone(),
        ] {
            assert!(roots(&work, &bad).is_err(), "{}", bad.display());
        }
        assert!(roots(&build.join("w"), &build).is_err());
        assert!(roots(&slots.join("w"), &build).is_err());
        // A symlinked parent resolves to the directory it names.
        fs::create_dir(&work).unwrap();
        std::os::unix::fs::symlink(&work, root.join("link")).unwrap();
        let hidden = resolved(&root.join("link").join("b")).unwrap();
        assert_eq!(hidden, work.join("b"));
        assert!(roots(&work, &hidden).is_err());
        // A tracked checkout cannot hold build products.
        let checkout = root.join("checkout");
        fs::create_dir(&checkout).unwrap();
        assert!(
            Command::new("git")
                .arg("init")
                .arg("-q")
                .arg(&checkout)
                .status()
                .unwrap()
                .success()
        );
        assert!(roots(&work, &checkout.join("b")).is_err());
        // Nothing was created or removed by refusing.
        assert!(!build.exists() && evidence.exists() && slots.exists() && exe.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn record_policy_flags_artifacts_not_size() {
        let root = scratch("records");
        fs::write(root.join("big.log"), "x".repeat(3 << 20)).unwrap();
        fs::create_dir_all(root.join("cases/one")).unwrap();
        fs::write(root.join("cases/one/observations.txt"), "ok\n").unwrap();
        assert!(
            record_violations(&root).is_empty(),
            "large text is a record"
        );
        assert!(finish(&root, Ok(()), "PASS\n").is_ok());
        assert_eq!(fs::read_to_string(root.join("RESULT")).unwrap(), "PASS\n");
        fs::write(root.join("hagia"), b"\x7fELF\x02\x01\x01").unwrap();
        fs::write(root.join("cases/one/lib.rlib"), "!<arch>\n").unwrap();
        fs::write(root.join("run.sh"), "#!/bin/sh\n").unwrap();
        fs::set_permissions(root.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::create_dir(root.join("nimcache")).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", root.join("escape")).unwrap();
        assert_eq!(
            record_violations(&root),
            [
                "artifact cases/one/lib.rlib",
                "artifact hagia",
                "artifact run.sh",
                "directory nimcache",
                "symlink escape",
            ]
        );
        // The original failure and the policy refusal are both kept, and
        // the violating directory still closes so the STOP stays diagnosable.
        assert!(finish(&root, Err("case failed".into()), "PASS\n").is_err());
        let result = fs::read_to_string(root.join("RESULT")).unwrap();
        assert!(
            result.starts_with("STOP case failed; record policy: 5"),
            "{result}"
        );
        assert!(
            fs::read_to_string(root.join("record-policy"))
                .unwrap()
                .contains("artifact hagia")
        );
        let manifest = fs::read_to_string(root.join("SHA256SUMS")).unwrap();
        assert!(manifest.contains("big.log") && !manifest.contains("escape"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_unreadable_record_cannot_pass() {
        let root = scratch("unreadable");
        fs::write(root.join("log"), "text\n").unwrap();
        fs::set_permissions(root.join("log"), fs::Permissions::from_mode(0o000)).unwrap();
        assert!(finish(&root, Ok(()), "PASS\n").is_err());
        let result = fs::read_to_string(root.join("RESULT")).unwrap();
        assert!(result.starts_with("STOP "), "{result}");
        let policy = fs::read_to_string(root.join("record-policy")).unwrap();
        assert!(policy.contains("inspection error log"), "{policy}");
        let manifest = fs::read_to_string(root.join("SHA256SUMS")).unwrap();
        assert!(manifest.contains("RESULT") && !manifest.contains("  log\n"));
        fs::set_permissions(root.join("log"), fs::Permissions::from_mode(0o600)).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn one_holder_leases_a_build_root() {
        let root = scratch("lease");
        let evidence = root.join("evidence");
        let slots = root.join("slots");
        for dir in [&evidence, &slots] {
            fs::create_dir(dir).unwrap();
        }
        let build = root.join("build");
        let (held, identity) = lease_build_root(&build).unwrap();
        assert!(identity.contains(".build.wm-lifecycle.lock"));
        // A second, independently opened handle is refused while held.
        let busy = lease_build_root(&build).unwrap_err().to_string();
        assert!(busy.contains("busy"), "{busy}");
        own_build_root(&build, &[]).unwrap();
        fs::write(build.join("in-use"), "active").unwrap();
        // Cleanup cannot remove an active root.
        let exe = root.join("exe");
        fs::write(&exe, "sentinel").unwrap();
        let home = root.join("home");
        let refused = cleanup(&build, &evidence.join("c1"), &evidence, &slots, &home, &exe)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("BUILD_ROOT is busy"), "{refused}");
        assert!(build.join("in-use").exists() && !evidence.join("c1").exists());
        drop(held);
        // The same lock inode serves the next holder; it is never unlinked.
        let (_again, again) = lease_build_root(&build).unwrap();
        assert_eq!(identity, again);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_a_marked_build_root_is_reused_or_removed() {
        let root = scratch("cleanup");
        let evidence = root.join("evidence");
        let slots = root.join("slots");
        let checkout = root.join("checkout");
        for dir in [&evidence, &slots, &checkout] {
            fs::create_dir(dir).unwrap();
        }
        let exe = root.join("exe");
        fs::write(&exe, "sentinel").unwrap();
        let home = root.join("home");
        let foreign = root.join("foreign");
        fs::create_dir(&foreign).unwrap();
        assert!(own_build_root(&foreign, &[]).is_err());
        assert!(
            cleanup(
                &foreign,
                &evidence.join("c0"),
                &evidence,
                &slots,
                &home,
                &exe
            )
            .is_err()
        );
        assert!(foreign.exists());
        // A root that contains the cleanup executable or HOME is never removed.
        assert!(cleanup(&root, &evidence.join("c1"), &evidence, &slots, &home, &exe).is_err());
        let homed = root.join("homed");
        own_build_root(&homed, &[]).unwrap();
        assert!(
            cleanup(
                &homed,
                &evidence.join("c1"),
                &evidence,
                &slots,
                &homed.join("h"),
                &exe
            )
            .is_err()
        );
        assert!(exe.exists() && foreign.exists() && homed.exists());
        let build = root.join("build");
        own_build_root(&build, &[checkout.clone()]).unwrap();
        own_build_root(&build, &[checkout.clone()]).unwrap();
        assert_eq!(marked(&build), Some(vec![checkout.clone()]));
        fs::create_dir_all(build.join("runs/one")).unwrap();
        fs::write(build.join("runs/one/hagia"), b"\x7fELF").unwrap();
        let record = evidence.join("cleanup-01");
        cleanup(&build, &record, &evidence, &slots, &home, &exe).unwrap();
        assert!(!build.exists() && checkout.exists() && exe.exists());
        assert_eq!(
            fs::read_to_string(record.join("RESULT")).unwrap(),
            "PASS build root removed\n"
        );
        assert!(record_violations(&record).is_empty());
        assert!(cleanup(&build, &evidence.join("c2"), &evidence, &slots, &home, &exe).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_supervised_steps_inherit_the_lease() {
        let root = scratch("inherit");
        let (lease, _) = lease_build_root(&root.join("build")).unwrap();
        let probe = format!("test -e /proc/$$/fd/{}", lease.as_raw_fd());
        let mut step = Command::new("sh");
        step.args(["-c", &probe]);
        inherit_lease(&mut step, &lease);
        assert!(step.status().unwrap().success());
        assert!(
            !Command::new("sh")
                .args(["-c", &probe])
                .status()
                .unwrap()
                .success()
        );
        drop(lease);
        fs::remove_dir_all(root).unwrap();
    }

    const HOLDER: &str = "WM_LIFECYCLE_LEASE_HOLDER";

    /// The lease-owning runner for the control below, run in its own process
    /// so the control can SIGKILL it. Without the variable it does nothing.
    #[test]
    #[ignore]
    fn lease_holder_process() {
        let Some(root) = env::var_os(HOLDER).map(PathBuf::from) else {
            return;
        };
        let build = root.join("build");
        let (lease, _) = lease_build_root(&build).unwrap();
        own_build_root(&build, &[]).unwrap();
        if env::var("WM_LIFECYCLE_LEASE_STEP").unwrap() != "step" {
            let revision = fs::read_to_string(root.join("revision")).unwrap();
            fs::write(root.join("work/held.argv"), "archive child control\n").unwrap();
            archive(
                &root.join("repo"),
                revision.trim(),
                &build.join("source"),
                &lease,
            )
            .unwrap();
            return;
        }
        let run = Run {
            work: root.join("work"),
            build,
            target: root.join("target"),
            cargo: root.join("cargo"),
            lease: &lease,
        };
        // The step waits for the control's release, bounded by its timeout.
        let script =
            "touch started; until [ -e release ]; do sleep 0.02; done; touch finished".to_owned();
        let _ = log_step(
            &run.work,
            "held",
            &mut run.isolated(&run.work, 20, "sh", &["-c".into(), script]),
        );
    }

    #[test]
    fn a_killed_runner_leaves_the_root_leased_until_its_step_exits() {
        killed_writer("step");
    }

    #[test]
    fn a_killed_runner_leaves_the_root_leased_until_git_archive_exits() {
        killed_writer("git");
    }

    #[test]
    fn a_killed_runner_leaves_the_root_leased_until_tar_exits() {
        killed_writer("tar");
    }

    fn killed_writer(writer: &str) {
        use std::{
            thread::sleep,
            time::{Duration, Instant},
        };
        let root = scratch(&format!("orphan-{writer}"));
        for dir in ["work", "target", "cargo"] {
            fs::create_dir(root.join(dir)).unwrap();
        }
        let mut command = Command::new(env::current_exe().unwrap());
        command
            .args(["--exact", "tests::lease_holder_process", "--ignored"])
            .env(HOLDER, &root)
            .env("WM_LIFECYCLE_LEASE_STEP", writer)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if writer != "step" {
            checked(
                Command::new("git")
                    .args(["init", "--quiet"])
                    .arg(root.join("repo")),
            )
            .unwrap();
            let revision = checked(
                Command::new("git")
                    .arg("-C")
                    .arg(root.join("repo"))
                    .arg("mktree"),
            )
            .unwrap();
            fs::write(root.join("revision"), revision).unwrap();
            let executable =
                checked(Command::new("sh").args(["-c", &format!("command -v {writer}")])).unwrap();
            let bin = root.join("bin");
            fs::create_dir(&bin).unwrap();
            // Pause the actual archive writer at exec, with a bounded
            // handshake, so SIGKILL tests the production archive() call.
            let script = "#!/bin/sh\nexec timeout 15 sh -c 'touch \"$WM_LIFECYCLE_LEASE_HOLDER/work/started\"; until [ -e \"$WM_LIFECYCLE_LEASE_HOLDER/work/release\" ]; do sleep 0.02; done; \"$WM_LIFECYCLE_REAL_WRITER\" \"$@\"; result=$?; touch \"$WM_LIFECYCLE_LEASE_HOLDER/work/finished\"; exit \"$result\"' sh \"$@\"\n";
            fs::write(bin.join(writer), script).unwrap();
            fs::set_permissions(bin.join(writer), fs::Permissions::from_mode(0o700)).unwrap();
            let mut paths = vec![bin];
            paths.extend(env::split_paths(&env::var_os("PATH").unwrap()));
            command.env("PATH", env::join_paths(paths).unwrap());
            command.env("WM_LIFECYCLE_REAL_WRITER", executable.trim());
        }
        let mut holder = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !root.join("work/started").exists() {
            assert!(holder.try_wait().unwrap().is_none(), "holder exited early");
            assert!(Instant::now() < deadline, "sandboxed step never started");
            sleep(Duration::from_millis(20));
        }
        // SIGKILL the runner alone; timeout, in its own group, survives.
        holder.kill().unwrap();
        holder.wait().unwrap();
        let build = root.join("build");
        let busy = lease_build_root(&build).unwrap_err().to_string();
        assert!(busy.contains("BUILD_ROOT is busy"), "{busy}");
        assert!(!root.join("work/finished").exists());
        // Still busy after a pause: the orphaned step, not timing, holds it.
        sleep(Duration::from_millis(200));
        let busy = lease_build_root(&build).unwrap_err().to_string();
        assert!(busy.contains("BUILD_ROOT is busy"), "{busy}");
        fs::write(root.join("work/release"), "").unwrap();
        let (_lease, _) = loop {
            match lease_build_root(&build) {
                Ok(lease) => break lease,
                Err(e) => assert!(e.to_string().contains("BUILD_ROOT is busy"), "{e}"),
            }
            assert!(Instant::now() < deadline, "lease never released");
            sleep(Duration::from_millis(20));
        };
        // The lease ended only after the orphaned step had finished.
        assert!(root.join("work/finished").exists());
        assert!(root.join("work/held.argv").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_step_is_recorded_and_cannot_pass() {
        let root = env::temp_dir().join(format!("wm-lifecycle-controls-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        assert!(log_step(&root, "failed", &mut Command::new("false")).is_err());
        assert_eq!(fs::read_to_string(root.join("failed.exit")).unwrap(), "1\n");
        assert!(log_step(&root, "passed", &mut Command::new("true")).is_ok());
        close_records(&root).unwrap();
        let manifest = fs::read_to_string(root.join("SHA256SUMS")).unwrap();
        assert!(manifest.contains("failed.exit"));
        assert!(!manifest.contains("SHA256SUMS"));
        // A closed artifact changing invalidates the manifest.
        fs::write(root.join("failed.exit"), "0\n").unwrap();
        assert!(
            checked(Command::new("sha256sum").current_dir(&root).args([
                "--check",
                "--quiet",
                "SHA256SUMS"
            ]))
            .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
