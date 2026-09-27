//! Every external session entry point requires BOTH Sophia's prebuilt binary
//! (SOPHIA_BIN) and the host checker (SOPHIA_SESSION_PREFLIGHT) as absolute
//! executables, and refuses before any state, TTY or display-manager effect
//! when either is missing, empty, relative or not executable.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn executable(path: &Path) {
    fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn launchers_refuse_each_missing_host_path_before_any_effect() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let scratch = Scratch(std::env::temp_dir().join(format!("host-paths-{}", std::process::id())));
    let base = &scratch.0;
    fs::create_dir_all(base.join("sophia/tools")).unwrap();
    executable(&base.join("sophia/tools/run_sophia_session.sh"));
    executable(&base.join("sophia-bin"));
    executable(&base.join("preflight"));
    fs::write(base.join("not-executable"), "").unwrap();
    let good = [
        ("SOPHIA_ROOT", base.join("sophia")),
        ("SOPHIA_BIN", base.join("sophia-bin")),
        ("SOPHIA_SESSION_PREFLIGHT", base.join("preflight")),
        (
            "SOPHIA_INTEGRATION_XTASK",
            PathBuf::from(env!("CARGO_BIN_EXE_xtask")),
        ),
    ];
    for script in [
        "tools/session/start_sophia_tty3.sh",
        "tools/session/run_desktop_session.sh",
    ] {
        for missing in ["SOPHIA_BIN", "SOPHIA_SESSION_PREFLIGHT"] {
            for bad in [
                None,
                Some(""),
                Some("relative/path"),
                Some("NOT_EXECUTABLE"),
            ] {
                let home = base.join(format!("home-{missing}"));
                let _ = fs::remove_dir_all(&home);
                fs::create_dir(&home).unwrap();
                let mut command = Command::new("/bin/bash");
                command
                    .arg(repo.join(script))
                    .env_clear()
                    .env("PATH", "/usr/bin:/bin")
                    .env("HOME", &home)
                    .env("XDG_STATE_HOME", home.join("state"))
                    .env("XDG_RUNTIME_DIR", &home)
                    .env("TMPDIR", &home)
                    .env("SOPHIA_TTY_PROFILE", "hagia");
                for (name, value) in &good {
                    if *name != missing {
                        command.env(name, value);
                    }
                }
                match bad {
                    None => {}
                    Some("NOT_EXECUTABLE") => {
                        command.env(missing, base.join("not-executable"));
                    }
                    Some(value) => {
                        command.env(missing, value);
                    }
                }
                let output = command.output().unwrap();
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert_eq!(
                    output.status.code(),
                    Some(2),
                    "{script} {missing}={bad:?}: {stderr}"
                );
                assert!(
                    stderr.contains(missing),
                    "{script} {missing}={bad:?}: {stderr}"
                );
                // Nothing was created: no handoff log, no recipe state.
                let created = fs::read_dir(&home).unwrap().count();
                assert_eq!(created, 0, "{script} {missing}={bad:?} created state");
            }
        }
    }
}
