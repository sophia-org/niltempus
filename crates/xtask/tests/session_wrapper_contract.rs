//! The external adapter against Sophia's generic launcher contract (root,
//! pin pending): `run_sophia_session.sh -- session run <args...>`, an opaque
//! SOPHIA_TTY_PROFILE label, exactly one input selector among the arguments,
//! the product environment supplied by the caller, and the stop primitive
//! receiving the same label. A recording stand-in plays Sophia's wrapper.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Fixture(PathBuf);
impl Fixture {
    fn new(tag: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("wrapper-contract-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sophia/tools")).unwrap();
        fs::create_dir_all(root.join("runtime")).unwrap();
        fs::set_permissions(root.join("runtime"), fs::Permissions::from_mode(0o700)).unwrap();
        Self::script(
            &root.join("sophia/tools/run_sophia_session.sh"),
            "printf 'label=%s\\n' \"$SOPHIA_TTY_PROFILE\" > \"$RECORD\"\n\
             printf 'probe_slice=%s\\n' \"${SOPHIA_FIREFOX_M10_PROOF_SLICE:-}\" >> \"$RECORD\"\n\
             for a in \"$@\"; do printf 'arg=%s\\n' \"$a\" >> \"$RECORD\"; done",
        );
        Self::script(
            &root.join("sophia/tools/stop_sophia_session.sh"),
            "printf 'stop=%s\\n' \"$*\" > \"$RECORD\"",
        );
        // Sophia: records every command; check-host refuses when told to.
        Self::script(
            &root.join("sophia-bin"),
            "printf 'sophia=%s\\n' \"$*\" >> \"$TRACE\"\n\
             [ \"$1 $2\" != 'session check-host' ] || [ -z \"${REFUSE_HOST:-}\" ] || exit 1",
        );
        Self::script(&root.join("preflight"), "exit 0");
        Self(root)
    }
    fn script(path: &Path, body: &str) {
        fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fn run(&self, xtask: &Path, extra: &[&str], settings: &[(&str, &str)]) -> Output {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        Command::new("/bin/bash")
            .arg(repo.join("tools/session/run_desktop_session.sh"))
            .args(extra)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("XDG_RUNTIME_DIR", self.0.join("runtime"))
            .env("XDG_STATE_HOME", self.0.join("state"))
            .env("TMPDIR", self.0.join("runtime"))
            .env("RECORD", self.0.join("record"))
            .env("TRACE", self.0.join("trace"))
            .env("SOPHIA_ROOT", self.0.join("sophia"))
            .env("SOPHIA_BIN", self.0.join("sophia-bin"))
            .env("SOPHIA_SESSION_PREFLIGHT", self.0.join("preflight"))
            .env("SOPHIA_INTEGRATION_XTASK", xtask)
            .env("SOPHIA_TTY_PROFILE", "native")
            .env("SOPHIA_SESSION_TTY", "/dev/tty63")
            .env("SOPHIA_TERMINAL_BIN", "/bin/true")
            .env("SOPHIA_TERMINAL_KIND", "xterm")
            .envs(settings.iter().copied())
            .output()
            .unwrap()
    }
    fn record(&self) -> Vec<String> {
        fs::read_to_string(self.0.join("record"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn xtask() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_xtask"))
}

#[test]
fn the_wrapper_receives_dash_dash_session_run_and_an_opaque_label() {
    let f = Fixture::new("label");
    let output = f.run(&xtask(), &[], &[]);
    assert!(output.status.success(), "{output:?}");
    let record = f.record();
    assert_eq!(record[0], "label=native");
    assert_eq!(&record[2..5], ["arg=--", "arg=session", "arg=run"]);
    // Product names never reach Sophia: every profile maps to a label.
    for (profile, label, extra) in [
        ("kitty", "terminal", vec![]),
        (
            "standalone",
            "standalone",
            vec![("SOPHIA_STANDALONE_APP_BIN", "/bin/true")],
        ),
    ] {
        let mut settings = vec![("SOPHIA_TTY_PROFILE", profile)];
        settings.extend(extra);
        let output = f.run(&xtask(), &[], &settings);
        assert!(output.status.success(), "{profile}: {output:?}");
        assert_eq!(f.record()[0], format!("label={label}"));
    }
}

#[test]
fn exactly_one_input_selector_is_required() {
    let f = Fixture::new("selector");
    // One selector (the recipe's own): accepted, and exactly one reaches Sophia.
    let output = f.run(
        &xtask(),
        &[],
        &[("SOPHIA_OPERATOR_INPUT_DEVICES", "/dev/input/event7")],
    );
    assert!(output.status.success(), "{output:?}");
    let selectors = f
        .record()
        .iter()
        .filter(|l| l.starts_with("arg=--input-seat=") || l.starts_with("arg=--input-devices="))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(selectors, ["arg=--input-devices=/dev/input/event7"]);
    // Two: an extra operator selector on top of the recipe's.
    let _ = fs::remove_file(f.0.join("record"));
    let output = f.run(&xtask(), &["--input-seat=seat1"], &[]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("exactly one"));
    assert!(f.record().is_empty(), "Sophia's wrapper was started");
    // Zero: a recipe tool whose arguments carry no selector.
    let stub = f.0.join("stub-xtask");
    Fixture::script(
        &stub,
        "case \"$2\" in\n\
         prepare-inputs) printf 'sophia_session_inputs schema=1 status=prepared\\0/bin/true\\0xterm\\0\\0\\0\\0\\0' ;;\n\
         stage-proofs) printf 'sophia_session_proofs schema=1 status=prepared\\0\\0\\0' ;;\n\
         prepare-arguments) printf 'sophia_session_arguments schema=1 status=prepared\\0session\\0run\\0--native-scanout\\0' ;;\n\
         prepare-environment) printf 'sophia_desktop_recipe_environment schema=1 status=prepared\\0' ;;\n\
         esac",
    );
    let output = f.run(&stub, &[], &[]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("found 0"));
    assert!(f.record().is_empty(), "Sophia's wrapper was started");
}

#[test]
fn the_caller_supplies_the_product_environment() {
    let f = Fixture::new("environment");
    fs::write(f.0.join("desktop.kdl"), "schema 1\n").unwrap();
    let desktop = f.0.join("desktop.kdl");
    let output = f.run(
        &xtask(),
        &["--firefox-m10-selection-proof"],
        &[
            ("SOPHIA_TTY_PROFILE", "hagia"),
            ("SOPHIA_TERMINAL_KIND", "kitty"),
            ("SOPHIA_FIREFOX_BIN", "/bin/true"),
            ("SOPHIA_DESKTOP_PROFILE", desktop.to_str().unwrap()),
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let record = f.record();
    assert_eq!(record[0], "label=managed");
    // Sophia's wrapper inherits the recipe environment from its caller.
    assert_eq!(record[1], "probe_slice=selection");
}

#[test]
fn benchmark_records_go_to_the_adapter_log_not_to_sophia() {
    let f = Fixture::new("benchmark");
    let output = f.run(
        &xtask(),
        &[],
        &[
            ("SOPHIA_TTY_PROFILE", "standalone"),
            ("SOPHIA_STANDALONE_WORKLOAD", "glxgears"),
            ("SOPHIA_STANDALONE_APP_BIN", "/bin/true"),
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let log = fs::read_to_string(f.0.join("state/sophia/desktop-session/standalone-adapter.log"))
        .unwrap();
    assert!(log.contains(
        "sophia_desktop_adapter schema=1 status=starting profile=standalone label=standalone"
    ));
    assert!(log.contains("sophia_glxgears_benchmark schema=1 duration_seconds=20 surface_width=500 surface_height=500 swap_interval=1"));
    // Nothing product-specific is handed to Sophia's wrapper for its logs.
    assert!(!f.record().iter().any(|l| l.contains("benchmark")));
}

#[test]
fn a_host_preflight_refusal_runs_no_recipe_command() {
    let f = Fixture::new("preflight-first");
    // A recording recipe tool: any call at all is a failure here.
    let recipe = f.0.join("recording-xtask");
    Fixture::script(
        &recipe,
        "printf 'recipe=%s\\n' \"$*\" >> \"$TRACE\"; exit 99",
    );
    let output = f.run(&recipe, &[], &[("REFUSE_HOST", "1")]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let trace = fs::read_to_string(f.0.join("trace")).unwrap();
    assert_eq!(trace, "sophia=session check-host --tty=/dev/tty63\n");
    assert!(f.record().is_empty(), "Sophia's wrapper was started");
    assert_eq!(
        fs::read_dir(f.0.join("runtime")).unwrap().count(),
        0,
        "recipe state was created"
    );
    // Accepted: check-host runs first, then the recipe commands.
    let output = f.run(&xtask(), &[], &[]);
    assert!(output.status.success(), "{output:?}");
    let trace = fs::read_to_string(f.0.join("trace")).unwrap();
    assert!(
        trace
            .lines()
            .nth(1)
            .is_some_and(|l| l == "sophia=session check-host --tty=/dev/tty63")
    );
}

#[test]
fn a_missing_target_tty_refuses_before_any_effect() {
    let f = Fixture::new("no-tty");
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("/bin/bash")
        .arg(repo.join("tools/session/run_desktop_session.sh"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("TRACE", f.0.join("trace"))
        .env("SOPHIA_ROOT", f.0.join("sophia"))
        .env("SOPHIA_BIN", f.0.join("sophia-bin"))
        .env("SOPHIA_SESSION_PREFLIGHT", f.0.join("preflight"))
        .env("SOPHIA_INTEGRATION_XTASK", xtask())
        .env("SOPHIA_TTY_PROFILE", "native")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(!f.0.join("trace").exists());
}

/// Every file under `dir`: relative path, sha256, mode and mtime.
fn snapshot(dir: &Path) -> Vec<(PathBuf, String, u32, i64)> {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::MetadataExt;
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(&path).unwrap() {
            let entry = entry.unwrap();
            let meta = fs::symlink_metadata(entry.path()).unwrap();
            if meta.is_dir() {
                stack.push(entry.path());
            }
            let digest = if meta.is_file() {
                format!("{:x}", Sha256::digest(fs::read(entry.path()).unwrap()))
            } else {
                String::new()
            };
            out.push((
                entry.path().strip_prefix(dir).unwrap().to_path_buf(),
                digest,
                meta.mode(),
                meta.mtime_nsec() + meta.mtime() * 1_000_000_000,
            ));
        }
    }
    out.sort();
    out
}

#[test]
fn a_second_launch_cannot_touch_an_active_sessions_proof_files() {
    let f = Fixture::new("second-launch");
    fs::write(f.0.join("desktop.kdl"), "schema 1\n").unwrap();
    // Sophia's wrapper stand-in: refuses while a live wrapper.pid exists,
    // otherwise records its pid and stays alive until released.
    Fixture::script(
        &f.0.join("sophia/tools/run_sophia_session.sh"),
        "pid_file=\"$RUNNING/wrapper.pid\"\n\
         if [ -s \"$pid_file\" ] && kill -0 \"$(cat \"$pid_file\")\" 2>/dev/null; then echo 'already running' >&2; exit 1; fi\n\
         echo $$ > \"$pid_file\"\n\
         while [ ! -e \"$RUNNING/release\" ]; do sleep 0.05; done",
    );
    fs::create_dir(f.0.join("running")).unwrap();
    let desktop = f.0.join("desktop.kdl");
    let settings = [
        ("SOPHIA_TTY_PROFILE", "hagia"),
        ("SOPHIA_TERMINAL_KIND", "kitty"),
        ("SOPHIA_FIREFOX_BIN", "/bin/true"),
        ("SOPHIA_DESKTOP_PROFILE", desktop.to_str().unwrap()),
    ];
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let launch = || {
        let mut command = Command::new("/bin/bash");
        command
            .arg(repo.join("tools/session/run_desktop_session.sh"))
            .arg("--firefox-m10-proof")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("XDG_RUNTIME_DIR", f.0.join("runtime"))
            .env("XDG_STATE_HOME", f.0.join("state"))
            .env("TMPDIR", f.0.join("runtime"))
            .env("TRACE", f.0.join("trace"))
            .env("RUNNING", f.0.join("running"))
            .env("SOPHIA_ROOT", f.0.join("sophia"))
            .env("SOPHIA_BIN", f.0.join("sophia-bin"))
            .env("SOPHIA_SESSION_PREFLIGHT", f.0.join("preflight"))
            .env("SOPHIA_INTEGRATION_XTASK", xtask())
            .env("SOPHIA_SESSION_TTY", "/dev/tty63")
            .envs(settings);
        command
    };
    let mut first = launch().spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !f.0.join("running/wrapper.pid").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "first launch never started its wrapper"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let active = fs::read_dir(f.0.join("runtime"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();
    assert_eq!(active.len(), 1, "{active:?}");
    let active = active[0].clone();
    assert_eq!(
        fs::metadata(&active).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert!(
        snapshot(&active)
            .iter()
            .any(|(p, ..)| p.ends_with("user.js"))
    );
    let before = snapshot(&active);
    let second = launch().output().unwrap();
    assert!(!second.status.success(), "{second:?}");
    assert!(String::from_utf8_lossy(&second.stderr).contains("already running"));
    assert_eq!(
        snapshot(&active),
        before,
        "the second attempt touched the active session's files"
    );
    let dirs = fs::read_dir(f.0.join("runtime"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();
    assert_eq!(
        dirs,
        [active.clone()],
        "the second attempt left recipe state behind"
    );
    // The first session's files live until its wrapper ends, then go.
    fs::write(f.0.join("running/release"), "").unwrap();
    assert!(first.wait().unwrap().success());
    assert!(!active.exists());
}

#[test]
fn stop_scripts_pass_the_same_opaque_label() {
    let f = Fixture::new("stop");
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (profile, label) in [
        ("hagia", "managed"),
        ("kitty", "terminal"),
        ("native", "native"),
        ("standalone", "standalone"),
    ] {
        let output = Command::new("/bin/bash")
            .arg(repo.join(format!("tools/session/stop_sophia_{profile}_session.sh")))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("RECORD", f.0.join("record"))
            .env("SOPHIA_ROOT", f.0.join("sophia"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{profile}: {output:?}");
        assert_eq!(f.record(), [format!("stop={label}")]);
    }
}
