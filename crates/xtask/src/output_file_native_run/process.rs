//! The staged generic wrapper owns input guard, Session shutdown and TTY
//! restoration. Keep its controlling terminal; do not put it in a background
//! process group where termios writes would receive SIGTTOU.
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{IsTerminal, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, WaitIdStatus};

const LOG_CAP: u64 = 64 << 20;

pub(super) fn attended(tty: &str) -> Result<String, String> {
    if std::env::var("SOPHIA_FRAME_FED_OUTPUT_ARM").as_deref() != Ok("1") {
        return Err("physical execution requires SOPHIA_FRAME_FED_OUTPUT_ARM=1 and an authorized attended window".into());
    }
    if !std::io::stdin().is_terminal()
        || fs::read_link("/proc/self/fd/0").map_err(|e| e.to_string())? != Path::new(tty)
    {
        return Err(format!("run from the prepared console {tty}"));
    }
    foreground(tty)
}

fn foreground(tty: &str) -> Result<String, String> {
    // An inherited tty4 fd remains tty4 after the operator switches to tty2.
    // Check the kernel's foreground VT as well, before any wrapper takeover.
    let active = fs::read_to_string("/sys/class/tty/tty0/active")
        .map_err(|e| format!("cannot verify foreground console: {e}"))?;
    super::check_foreground_tty(tty, &active)?;
    Ok(active.trim_end_matches('\n').to_owned())
}

struct Wrapper {
    child: Child,
    recovery: String,
}

impl Wrapper {
    fn status(&self) -> Result<Option<WaitIdStatus>, String> {
        rustix::process::waitid(
            WaitId::Pid(Pid::from_raw(self.child.id() as i32).unwrap()),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        )
        .map_err(|e| e.to_string())
    }
}

impl Drop for Wrapper {
    fn drop(&mut self) {
        // The wrapper owns Session in a separate session/process group. Give
        // its TERM trap time to stop that group and restore the console before
        // escalating. A hard cleanup failure never qualifies as restoration.
        if !matches!(self.status(), Ok(Some(_))) {
            let _ = rustix::process::kill_process(
                Pid::from_raw(self.child.id() as i32).unwrap(),
                Signal::TERM,
            );
            let end = Instant::now() + Duration::from_secs(10);
            while Instant::now() < end && matches!(self.status(), Ok(None)) {
                std::thread::sleep(Duration::from_millis(20));
            }
            if !matches!(self.status(), Ok(Some(_))) {
                let _ = self.child.kill();
                eprintln!(
                    "Output proof wrapper required KILL; Session and console restoration are unproven. {}",
                    self.recovery
                );
            }
        }
        let _ = self.child.wait();
    }
}

fn new_log(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn log_bounds(stage: &Path, logs: &Path) -> Result<(), String> {
    for name in [
        "wrapper.log",
        "untrusted-session-output.log",
        "session.log",
        "recovery.log",
        "input-guard.log",
        "lifecycle.log",
    ] {
        let path = if name == "wrapper.log" {
            stage.join(name)
        } else {
            logs.join(name)
        };
        match fs::metadata(path) {
            Ok(metadata) if metadata.len() > LOG_CAP => {
                return Err(format!("{name} exceeds {LOG_CAP} bytes"));
            }
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("{name}: {e}"));
            }
            _ => (),
        }
    }
    Ok(())
}

pub(super) fn session(
    values: &BTreeMap<String, String>,
    argv: &[String],
    stage: &Path,
) -> Result<(), String> {
    let active = attended(&values["tty"])?;
    let record = serde_json::json!({"schema":1,"planned_tty":values["tty"],"active_tty":format!("/dev/{active}"),"status":"matched"});
    new_log(&stage.join("foreground-console.json"))?
        .write_all(&serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let wrapper = Path::new(&values["inputs"]).join("sophia-tree/tools/run_sophia_session.sh");
    let runtime = super::number(&values["runtime-ms"])?;
    let state = stage.join("state");
    let logs = state.join("sophia/output-file-native-session");
    let log = new_log(&stage.join("wrapper.log"))?;
    let mut command = Command::new("/bin/bash");
    command.arg(wrapper).arg("--").args(&argv[1..]).env_clear();
    // Session runtime and home are explicit operator context. No inherited
    // display, bus address, LD_*, SOPHIA_* override or proof control survives.
    for name in ["HOME", "USER", "LOGNAME", "XDG_RUNTIME_DIR", "LANG"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .env("PATH", "/usr/bin:/bin")
        .env("RUST_LOG", "info")
        .env("NO_COLOR", "1")
        // The wrapper owns this private log root. Do not set DIAGNOSTIC_DIR:
        // that enables Sophia's reduced daily recorder, which consumes Session
        // lines and strips display identity from the physical proof transcript.
        // The explicit proof/runtime arguments prevent automatic daily capture.
        .env("XDG_STATE_HOME", &state)
        .env("SOPHIA_BIN", &argv[0])
        .env("SOPHIA_BUILD_SESSION", "false")
        .env("SOPHIA_MANAGE_KEYD", "false")
        .env("SOPHIA_REQUIRE_LOCAL_VT", "true")
        .env("SOPHIA_REQUIRE_RUNTIME_DIR", "true")
        .env("SOPHIA_SESSION_PREFLIGHT", &values["preflight"])
        .env("SOPHIA_TTY_PROFILE", "output-file-native")
        .env("SOPHIA_SESSION_HANDOFF", "cycle_runner")
        .env("SOPHIA_INPUT_GUARD_ARMING", "automatic")
        .env(
            "SOPHIA_SESSION_WATCHDOG_SECONDS",
            (runtime / 1000 + 20).to_string(),
        )
        .env("SOPHIA_FRAME_FED_OUTPUT_ARM", "1")
        .env("SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE", "1")
        .env(
            "SOPHIA_UNTRUSTED_SESSION_OUTPUT_LOG",
            logs.join("untrusted-session-output.log"),
        )
        .stdin(Stdio::inherit())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log);
    let child = Wrapper {
        child: command
            .spawn()
            .map_err(|e| format!("start recovery wrapper: {e}"))?,
        recovery: format!(
            "From the recovery VT, inspect $XDG_RUNTIME_DIR/sophia-output-file-native-session-$UID/wrapper.pid and use {}/sophia-tree/tools/stop_sophia_session.sh output-file-native; retain {}.",
            values["inputs"],
            stage.display()
        ),
    };
    let deadline = Instant::now() + Duration::from_millis(runtime) + Duration::from_secs(90);
    let status = loop {
        log_bounds(stage, &logs)?;
        if let Some(status) = child.status()? {
            break status;
        }
        if Instant::now() >= deadline {
            return Err(
                "wrapper deadline; native restoration is unproven, inspect console recovery".into(),
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    drop(child);
    log_bounds(stage, &logs)?;
    if status.exit_status() != Some(0) {
        return Err(format!(
            "wrapper failed: {status:?}; evidence retained at {}",
            stage.display()
        ));
    }
    let recovery = fs::read_to_string(logs.join("recovery.log")).map_err(|e| e.to_string())?;
    check_recovery(&recovery)?;
    // Retain the wrapper's separate lifecycle log and put only the actual
    // Session/peer output at the verifier's canonical path.
    fs::rename(logs.join("session.log"), stage.join("wrapper-session.log"))
        .map_err(|e| e.to_string())?;
    fs::rename(
        logs.join("untrusted-session-output.log"),
        stage.join("session.log"),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub(super) fn check_recovery(text: &str) -> Result<(), String> {
    let mut records = BTreeMap::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let Some(kind) = words.next() else {
            continue;
        };
        if !matches!(
            kind,
            "sophia_tty_recovery" | "sophia_tty_recovery_verification"
        ) {
            continue;
        }
        let mut fields = BTreeMap::new();
        for word in words {
            let (key, value) = word
                .split_once('=')
                .ok_or("malformed TTY recovery record")?;
            if fields.insert(key, value).is_some() {
                return Err("duplicate TTY recovery field".into());
            }
        }
        if records.insert(kind, fields).is_some() {
            return Err("duplicate TTY recovery record".into());
        }
    }
    let tty = records
        .get("sophia_tty_recovery")
        .ok_or("missing TTY recovery record")?;
    let keyboard = records
        .get("sophia_tty_recovery_verification")
        .ok_or("missing keyboard recovery record")?;
    let equal = |row: &BTreeMap<&str, &str>, a, b| {
        row.get(a)
            .is_some_and(|v| *v != "unavailable" && row.get(b) == Some(v))
    };
    if tty.get("schema") != Some(&"3")
        || tty.get("emergency") != Some(&"false")
        || tty.get("termios_restored") != Some(&"true")
        || !equal(tty, "kd_mode_before", "kd_mode_after")
        || keyboard.get("schema") != Some(&"1")
        || keyboard.get("keyd_restored") != Some(&"true")
        || !equal(keyboard, "keyboard_mode_before", "keyboard_mode_after")
    {
        return Err("TTY, keyboard or normal shutdown recovery was not proved".into());
    }
    Ok(())
}
