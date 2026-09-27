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
        for name in ["sophia", "preflight"] {
            Self::script(&root.join(name), "exit 0");
        }
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
            .env("TMPDIR", self.0.join("runtime"))
            .env("RECORD", self.0.join("record"))
            .env("SOPHIA_ROOT", self.0.join("sophia"))
            .env("SOPHIA_BIN", self.0.join("sophia"))
            .env("SOPHIA_SESSION_PREFLIGHT", self.0.join("preflight"))
            .env("SOPHIA_INTEGRATION_XTASK", xtask)
            .env("SOPHIA_TTY_PROFILE", "native")
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
