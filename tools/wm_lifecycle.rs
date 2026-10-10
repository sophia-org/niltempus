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
const SOPHIA: &str = "0f2ad2386baa063ab92d9567a145148f5232bccb";
const MOUNT: &str = "crates/sophia-session/tests/support/desktop_launch_reload.rs";
const CASES: [&str; 3] = [
    "sdk_lifecycle_startup",
    "sdk_lifecycle_profile_rollback",
    "sdk_lifecycle_restart",
];

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
                "sophia={SOPHIA}\nhagia={}\nfixture_sha256={}\nmount_sha256={}\nscene=empty\nadmission=protected_supervisor\nautomatic_restart_trigger=terminate_fixture_child\nnative=false\n",
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
        let mount = overlay.join(MOUNT);
        let original = fs::read_to_string(&mount)?;
        fs::write(
            &mount,
            format!("{original}\n#[path = \"hagia_sdk_lifecycle.rs\"]\nmod hagia_sdk_lifecycle;\n"),
        )?;
        fs::write(
            work.join("overlay.sha256"),
            format!(
                "{}  {MOUNT}\n{}  hagia_sdk_lifecycle.rs\n",
                hash(&mount)?,
                hash(&module)?
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
        if hash(&fixture)? != hash(&module)? || hash(&binary)? != digest {
            return Err("input changed during run".into());
        }
        Ok(())
    })();
    fs::write(
        work.join("RESULT"),
        match &result {
            Ok(()) => "PASS lifecycle=3 native=false\n".into(),
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
        assert_eq!(listed_tests(&text).unwrap().len(), 3);
        assert!(listed_tests(&(text.clone() + &text)).is_err());
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
