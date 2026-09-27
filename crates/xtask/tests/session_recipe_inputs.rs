// Provenance: moved from Sophia crates/sophia-cli/tests/session_prepare_inputs.rs
// at a6edbbcad02ad9e9bb4c790e7cfd714cf333d30b (original copy; source unchanged at the pin de776c68) (Sophia rule 13):
// discovery, Firefox staging, failed staging, the Kitty parser deadline
// (process-group reap) and direct-scanout staging. Controls, the legacy
// launcher refusal and check-launch acceptance stay in Sophia with their
// commands. Direct-scanout configurations are read from --sophia-root.
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt, symlink},
    path::PathBuf,
    process::{Command, Output},
    time::{Duration, Instant},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("sophia inputs {} {nonce}", std::process::id()));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        fs::create_dir(path.join("bin")).unwrap();
        fs::DirBuilder::new()
            .mode(0o700)
            .create(path.join("state"))
            .unwrap();
        Self(path)
    }
    fn script(&self, name: &str, body: &str) -> PathBuf {
        let path = self.0.join("bin").join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
    fn command(&self, subcommand: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
        command
            .env_clear()
            .env("PATH", self.0.join("bin"))
            .args(["session-recipe", subcommand]);
        command
    }
    fn stage(&self, profile: &str) -> Command {
        let mut command = self.command("stage-proofs");
        command
            .arg(format!("--profile={profile}"))
            .arg(format!("--root={}", self.0.display()))
            .arg(format!("--sophia-root={}", self.0.display()))
            .arg(format!("--state-dir={}/state", self.0.display()));
        command
    }
    fn compare(&self, profile: &str, settings: &[(&str, &str)], proof: bool) {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut old = Command::new("/bin/bash");
        old.args(["-eu", "-c", r#"
source "$SOURCE/tools/session/lib/session_terminal.sh"
SESSION_PROFILE="$PROFILE"
TRUECOLOR_PROOF="${SOPHIA_TRUECOLOR_PROOF:-false}"
normal_application_defaults=false
if sophia_session_uses_application_defaults "$SESSION_PROFILE" "$FIREFOX_M10_ANY_PROOF" "$TRUECOLOR_PROOF"; then normal_application_defaults=true; fi
terminal_kind=""
source "$SOURCE/crates/xtask/tests/fixtures/session_inputs_before_t027.sh"
printf '%s\0' "$terminal_bin" "$terminal_kind" "$hagia_browser_bin" "$standalone_bin"
"#]);
        old.env_clear()
            .env("PATH", self.0.join("bin"))
            .env("SOURCE", source)
            .env("PROFILE", profile)
            .env(
                "FIREFOX_M10_ANY_PROOF",
                if proof { "true" } else { "false" },
            )
            .envs(settings.iter().copied());
        let mut new = self.command("prepare-inputs");
        new.arg(format!("--profile={profile}"))
            .envs(settings.iter().copied())
            .arg("--");
        if proof {
            new.arg("--firefox-m10-proof");
        }
        let old = old.output().unwrap();
        let new = new.output().unwrap();
        assert_eq!(
            old.status.success(),
            new.status.success(),
            "{profile} {settings:?}: old {old:?}, new {new:?}"
        );
        if new.status.success() {
            let fields = fields(&new);
            assert_eq!(fields.len(), 8);
            assert_eq!(fields[0], "sophia_session_inputs schema=1 status=prepared");
            let expected: Vec<_> = old.stdout.split(|b| *b == 0).collect();
            assert_eq!(
                &fields[1..5],
                expected[..4]
                    .iter()
                    .map(|b| std::str::from_utf8(b).unwrap())
                    .collect::<Vec<_>>()
            );
        } else {
            assert!(new.stdout.is_empty());
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn fields(output: &Output) -> Vec<&str> {
    assert!(output.status.success(), "{output:?}");
    std::str::from_utf8(&output.stdout)
        .unwrap()
        .split('\0')
        .collect()
}

#[test]
fn discovery_matches_retained_launchers_with_missing_defaults_and_explicit_adapters() {
    let f = Fixture::new();
    for profile in ["hagia", "native", "kitty", "standalone"] {
        f.compare(profile, &[], false);
    }
    for name in ["kitty", "xterm", "firefox", "helium", "vkcube", "glxgears"] {
        f.script(name, "exit 0");
    }
    for name in ["readlink", "basename"] {
        symlink(format!("/usr/bin/{name}"), f.0.join("bin").join(name)).unwrap();
    }
    for profile in ["hagia", "native", "kitty", "standalone"] {
        f.compare(profile, &[], false);
    }
    for proof in [false, true] {
        f.compare("hagia", &[], proof);
        f.compare("hagia", &[("SOPHIA_TRUECOLOR_PROOF", "true")], proof);
        f.compare(
            "hagia",
            &[
                ("SOPHIA_TERMINAL_BIN", "/bin/true"),
                ("SOPHIA_TERMINAL_KIND", "xterm"),
            ],
            proof,
        );
    }
    for workload in ["kitty", "vkcube", "glxgears", "xterm", "bad"] {
        f.compare(
            "standalone",
            &[("SOPHIA_STANDALONE_WORKLOAD", workload)],
            false,
        );
    }
    for interval in ["0", "01", "1", "1000", "1001", "-1", "word"] {
        f.compare(
            "standalone",
            &[
                ("SOPHIA_STANDALONE_WORKLOAD", "xterm"),
                ("SOPHIA_XTERM_INTERVAL_MSEC", interval),
            ],
            false,
        );
    }
    f.compare("hagia", &[("SOPHIA_TERMINAL_BIN", "/missing")], false);
    f.compare("hagia", &[("SOPHIA_HAGIA_BROWSER_BIN", "/missing")], false);
    let literal = f.script("literal $(touch nope); ' app", "exit 0");
    f.compare(
        "hagia",
        &[("SOPHIA_TERMINAL_BIN", literal.to_str().unwrap())],
        false,
    );
}

#[test]
fn firefox_staging_is_private_reclaims_stale_profiles_and_does_not_follow_links() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("state/firefox-m10.stale")).unwrap();
    fs::create_dir(f.0.join("outside")).unwrap();
    fs::write(f.0.join("outside/keep"), "keep").unwrap();
    symlink(f.0.join("outside"), f.0.join("state/firefox-m10.link")).unwrap();
    let result = f
        .stage("hagia")
        .args(["--", "--firefox-m10-proof"])
        .output()
        .unwrap();
    let fields = fields(&result);
    assert_eq!(fields[0], "sophia_session_proofs schema=1 status=prepared");
    for path in [PathBuf::from(fields[1]), PathBuf::from(fields[2])] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    let prefs = PathBuf::from(fields[2]).join("user.js");
    assert_eq!(
        fs::metadata(&prefs).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let prefs = fs::read_to_string(prefs).unwrap();
    assert!(prefs.contains("user_pref(\"fission.autostart\", false)"));
    assert!(prefs.contains("user_pref(\"middlemouse.paste\", true)"));
    assert!(!f.0.join("state/firefox-m10.stale").exists());
    assert!(f.0.join("outside/keep").exists());
    assert!(f.0.join("state/firefox-m10.link").is_symlink());
}

#[test]
fn failed_proof_preparation_removes_new_profile_and_never_publishes_acceptance() {
    let f = Fixture::new();
    let kitty = f.script("kitty", "exit 9");
    let result = f
        .stage("standalone")
        .env("SOPHIA_STANDALONE_WORKLOAD", "kitty")
        .arg(format!("--standalone={}", kitty.display()))
        .args(["--", "--firefox-m10-proof"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(fs::read_dir(f.0.join("state")).unwrap().all(|e| {
        !e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("firefox-m10.")
    }));
    fs::set_permissions(f.0.join("state"), fs::Permissions::from_mode(0o755)).unwrap();
    let result = f.stage("hagia").output().unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
}

#[test]
fn kitty_parser_deadline_reaps_its_process_group() {
    let f = Fixture::new();
    let kitty = f.script(
        "kitty",
        &format!(
            "/bin/sleep 120 &\nprintf '%s' $! > '{}'\nwait",
            f.0.join("child").display()
        ),
    );
    let start = Instant::now();
    let result = f
        .stage("standalone")
        .env("SOPHIA_STANDALONE_WORKLOAD", "kitty")
        .arg(format!("--standalone={}", kitty.display()))
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(start.elapsed() < Duration::from_secs(15));
    let pid = fs::read_to_string(f.0.join("child")).unwrap();
    for _ in 0..100 {
        match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Err(_) => return,
            Ok(stat) if stat.split(") ").nth(1).unwrap().starts_with('Z') => return,
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    panic!("preparation left a running descendant {pid}");
}

#[test]
fn direct_scanout_inputs_replace_destination_links_with_private_copies() {
    let f = Fixture::new();
    fs::create_dir_all(f.0.join("tools/fixtures")).unwrap();
    for name in ["core", "desktop"] {
        fs::write(
            f.0.join(format!("tools/fixtures/direct_scanout_{name}.kdl")),
            name,
        )
        .unwrap();
        fs::write(f.0.join(format!("outside-{name}")), "keep").unwrap();
        symlink(
            f.0.join(format!("outside-{name}")),
            f.0.join(format!("state/standalone-{name}.kdl")),
        )
        .unwrap();
    }
    let output = f
        .stage("standalone")
        .env("SOPHIA_ENABLE_DIRECT_SCANOUT", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    for name in ["core", "desktop"] {
        let path = f.0.join(format!("state/standalone-{name}.kdl"));
        assert!(!path.is_symlink());
        assert_eq!(fs::read_to_string(&path).unwrap(), name);
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::read_to_string(f.0.join(format!("outside-{name}"))).unwrap(),
            "keep"
        );
    }
}
