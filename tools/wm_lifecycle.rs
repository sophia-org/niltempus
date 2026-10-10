//! Offline current-SDK Hagia lifecycle qualification. Compile with rustc;
//! no dependency on niltempus's older public-crate qualification pin.
use std::os::unix::fs::PermissionsExt;
use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const SOPHIA: &str = "f77244abe3063255c57c283a32eafe1666d6e684";
const MOUNT: &str = "crates/sophia-session/tests/support/desktop_launch_reload.rs";
const CASES: [&str; 10] = [
    "sdk_lifecycle_startup",
    "sdk_lifecycle_profile_rollback",
    "sdk_lifecycle_restart",
    "sdk_occupied_settlement",
    "sdk_occupied_restart",
    "sdk_operation_settlement",
    "sdk_operation_refused_action",
    "sdk_operation_disconnect",
    "sdk_slow_peer_response_deadline",
    "sdk_presented_chord_switcher",
];
/// Production spans the presentation fixture mirrors or calls, found by
/// semantic anchors in the archived pin. A changed hash refuses the run: the
/// copied dispatch and routing glue must be compared again, by review.
const GLUE: [(&str, &str, &str, &str); 4] = [
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
        "0d2b862c482ce624981887337bf321895e304d8741fd8651302d230797ac6292",
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

fn archive(root: &Path, revision: &str, destination: &Path) -> Result<()> {
    let tar = destination.with_extension("tar");
    checked(
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["archive", "--format=tar"])
            .arg(format!("--output={}", tar.display()))
            .arg(revision),
    )?;
    fs::create_dir(destination)?;
    checked(
        Command::new("tar")
            .arg("-xf")
            .arg(&tar)
            .arg("-C")
            .arg(destination),
    )?;
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

struct Run {
    work: PathBuf,
    target: PathBuf,
    cargo: PathBuf,
}
impl Run {
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
        for writable in [&self.work, &self.target, &self.cargo] {
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
    if work.join("cases").exists() {
        for case in fs::read_dir(work.join("cases"))? {
            for entry in fs::read_dir(case?.path())? {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    files.push(entry.path());
                }
            }
        }
    }
    files.sort();
    let mut manifest = String::new();
    for file in files {
        manifest.push_str(&format!(
            "{}  {}\n",
            hash(&file)?,
            file.strip_prefix(work)?.display()
        ));
    }
    fs::write(work.join("SHA256SUMS"), manifest)?;
    checked(Command::new("sha256sum").current_dir(work).args([
        "--check",
        "--quiet",
        "SHA256SUMS",
    ]))?;
    Ok(())
}

fn run() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 5 {
        return Err(
            "usage: wm-lifecycle SOPHIA_CHECKOUT HAGIA_CHECKOUT NEW_WORK TARGET CARGO_HOME".into(),
        );
    }
    let sophia = fs::canonicalize(&args[0])?;
    let hagia = fs::canonicalize(&args[1])?;
    let work = PathBuf::from(&args[2]);
    if !work.is_absolute() || work.exists() {
        return Err("work must be a new absolute directory".into());
    }
    let target = fs::canonicalize(&args[3])?;
    let cargo = fs::canonicalize(&args[4])?;
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
    fs::create_dir(&work)?;
    fs::set_permissions(&work, fs::Permissions::from_mode(0o700))?;
    let result = (|| -> Result<()> {
        let fixture = hagia.join("tests/external/lifecycle.rs");
        let occupied = hagia.join("tests/external/occupied.rs");
        let operations = hagia.join("tests/external/operations.rs");
        let slow_peer = hagia.join("tests/external/slow_peer.rs");
        let presentation = hagia.join("tests/external/presentation.rs");
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
                "sophia={SOPHIA}\nhagia={}\nfixture_sha256={}\nmount_sha256={}\nscene=empty_and_supplied_occupied\nlayout_completion=supplied\nadmission=protected_supervisor\nautomatic_restart_trigger=terminate_fixture_child\nslow_peer_trigger=sigstop_fixture_peer_production_response_deadline\npresented_case=sdk_presented_chord_switcher native_ceiling=selected_without_device presentation_completion=supplied surface_content=supplied_cpu_intake heads=supplied router_time=supplied admission_visual_readiness=bypassed\nnative=false\n",
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
        fs::copy(&operations, work.join("operations.rs"))?;
        fs::copy(&slow_peer, work.join("slow_peer.rs"))?;
        fs::copy(&presentation, work.join("presentation.rs"))?;
        let overlay = work.join("sophia");
        archive(&sophia, SOPHIA, &overlay)?;
        let hagia_source = work.join("hagia-source");
        archive(&hagia, &git(&hagia, &["rev-parse", "HEAD"])?, &hagia_source)?;
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
        let operations_module = module.with_file_name("operations.rs");
        fs::copy(&operations, &operations_module)?;
        let slow_peer_module = module.with_file_name("slow_peer.rs");
        fs::copy(&slow_peer, &slow_peer_module)?;
        let presentation_module = module.with_file_name("presentation.rs");
        if presentation_module.exists() {
            return Err("presentation module would replace a pinned file".into());
        }
        fs::copy(&presentation, &presentation_module)?;
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
                "{}  {MOUNT}\n{}  hagia_sdk_lifecycle.rs\n{}  occupied.rs\n{}  operations.rs\n{}  slow_peer.rs\n{}  presentation.rs\n",
                hash(&mount)?,
                hash(&module)?,
                hash(&occupied_module)?,
                hash(&operations_module)?,
                hash(&slow_peer_module)?,
                hash(&presentation_module)?
            ),
        )?;
        let run = Run {
            work: fs::canonicalize(&work)?,
            target,
            cargo,
        };
        let binary = run.work.join("hagia");
        let nim_args = vec![
            "c".into(),
            "--hints:off".into(),
            "--parallelBuild:1".into(),
            "-d:release".into(),
            format!("--nimcache:{}", run.work.join("nimcache").display()),
            format!("--out:{}", binary.display()),
            "src/hagia.nim".into(),
        ];
        log_step(
            &work,
            "build-hagia",
            &mut run.isolated(&hagia_source, 300, "nim", &nim_args),
        )?;
        let digest = hash(&binary)?;
        fs::write(work.join("hagia.sha256"), format!("{digest}  hagia\n"))?;
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
            || hash(&operations)? != hash(&operations_module)?
            || hash(&slow_peer)? != hash(&slow_peer_module)?
            || hash(&presentation)? != hash(&presentation_module)?
            || hash(&binary)? != digest
        {
            return Err("input changed during run".into());
        }
        Ok(())
    })();
    fs::write(
        work.join("RESULT"),
        match &result {
            Ok(()) => {
                "PASS lifecycle=3 occupied=2 operations=3 slow_peer=1 presented=1 native=false\n"
                    .into()
            }
            Err(e) => format!("STOP {e}\n"),
        },
    )?;
    close_records(&work)?;
    result
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
