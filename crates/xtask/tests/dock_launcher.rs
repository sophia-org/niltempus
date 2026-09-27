// Provenance: moved from Sophia crates/xtask/tests/dock_launcher.rs at
// 9fcaec782ce4fe9978568c0466ee17a78b3d4571 (Sophia rule 13). Adapted to the
// explicit inputs: a fixture Sophia checkout (SOPHIA_SOURCE, shared files
// pinned by digest) and prepared product artifacts bound to fixture commits.
//! Execute the actual shell launcher with supplied build/VT/session effects.
//! The real xtask profile/verifier binary and the real artifact intake run.
//! No device or live endpoint exists.
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn script(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, format!("#!/bin/bash\nset -euo pipefail\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn real_git() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("git"))
        .find(|path| path.is_file())
        .expect("git on PATH")
}

/// A prepared artifact directory; returns its commit.
fn artifact(directory: &Path, kind: &str, sdk_manifest: &Path) -> String {
    fs::create_dir_all(directory).unwrap();
    let raw = format!(
        "tree {}\nauthor A U Thor <a@example.com> 0 +0000\ncommitter A U Thor <a@example.com> 0 +0000\n\
         gpgsig -----BEGIN PGP SIGNATURE-----\n fixture\n -----END PGP SIGNATURE-----\n\n{kind}\n",
        "1".repeat(40)
    );
    let mut child = Command::new(real_git())
        .args(["hash-object", "--stdin", "-t", "commit"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(raw.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let commit = String::from_utf8(out.stdout).unwrap().trim().to_owned();
    let binary = if kind == "bemenu" { "bemenu-sophia" } else { kind };
    let body = format!("fixture {kind} binary\n");
    fs::write(directory.join(binary), &body).unwrap();
    fs::write(directory.join("source.commit"), &raw).unwrap();
    let common = format!(
        "binary={binary}\nbinary_sha256={}\nsource_commit={commit}\nsource_tree={}\n\
         signature_status=G\nsigner_fingerprint=ABCDEF0123\n",
        sha256(body.as_bytes()),
        "1".repeat(40)
    );
    let (name, manifest) = if kind == "bemenu" {
        (
            "bemenu-artifact.manifest",
            format!(
                "schema=1\n{common}sdk_revision={}\nsdk_manifest_sha256={}\n",
                "2".repeat(40),
                sha256(&fs::read(sdk_manifest).unwrap())
            ),
        )
    } else {
        let config = if kind == "hagia" {
            "config=none\nconfig_sha256=none\n".to_owned()
        } else {
            let text = format!("fixture {kind} configuration\n");
            fs::write(directory.join("config.kdl"), &text).unwrap();
            format!("config=config.kdl\nconfig_sha256={}\n", sha256(text.as_bytes()))
        };
        (
            "product-artifact.manifest",
            format!("schema=1\nproduct={kind}\n{common}{config}"),
        )
    };
    fs::write(directory.join(name), manifest).unwrap();
    commit
}

struct Fixture {
    directory: PathBuf,
    root: PathBuf,
    sophia: PathBuf,
    commits: Vec<(&'static str, String)>,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("dock-launcher-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let root = directory.join("integration");
        let sophia = directory.join("sophia");
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for name in [
            "tools/fixtures",
            "tools/lib",
            "tools/probes/lom_workload",
            "target/release",
            "pins/c-desktop-sdk",
        ] {
            fs::create_dir_all(root.join(name)).unwrap();
        }
        for name in [
            "tools/run_current_lom_panel_gate_tty4.sh",
            "tools/lib/artifacts.sh",
            "tools/fixtures/lom_panel_desktop.kdl",
            "tools/fixtures/lom_workload_budgets.json",
            "tools/probes/lom_workload/verify.py",
            "tools/probes/lom_workload/records.py",
            "tools/probes/lom_workload/memory.py",
            "pins/c-desktop-sdk/manifest.json",
        ] {
            fs::copy(repo.join(name), root.join(name)).unwrap();
        }
        fs::copy(
            env!("CARGO_BIN_EXE_xtask"),
            root.join("target/release/xtask"),
        )
        .unwrap();
        let sdk = root.join("pins/c-desktop-sdk/manifest.json");
        let commits = ["lom", "hagia", "bemenu", "provlita"]
            .into_iter()
            .map(|kind| {
                (
                    kind,
                    artifact(&directory.join("artifacts").join(kind), kind, &sdk),
                )
            })
            .collect();
        fs::create_dir(directory.join("bin")).unwrap();
        fs::write(
            directory.join("wm.kdl"),
            "schema 1\nshortcut { bind \"Super+Space\" \"session:application-launcher\"; }\n",
        )
        .unwrap();
        script(&directory.join("bin/tty"), "echo /dev/tty4");
        script(
            &directory.join("bin/git"),
            &format!(
                r#"
case "$*" in
    *'status --short'*) ;;
    *'rev-parse HEAD'*) printf '%040d\n' 1 ;;
    *'verify-commit '*) ;;
    *'hash-object --stdin -t commit'*) exec {} hash-object --stdin -t commit ;;
    *) exit 99 ;;
esac"#,
                real_git().display()
            ),
        );
        script(&directory.join("bin/cargo"), "echo build >> \"$TRACE\"");
        script(
            &sophia.join("target/release/sophia"),
            r#"case "$2" in print-effective) cat "$SOPHIA_DESKTOP_PROFILE" ;; check) test -f "${3#--desktop-profile=}" ;; *) exit 99 ;; esac"#,
        );
        script(
            &sophia.join("target/release/examples/desktop_profile_probe"),
            r#"
[[ "$#" == 3 && "$3" == --require-launcher-binding ]]
cat "$1"
tail -n +2 "$2""#,
        );
        script(
            &root.join("tools/lom_gpu_content_hardware_proof.sh"),
            r#"echo proof >> "$TRACE"; exit "${PROOF_STATUS:-0}""#,
        );
        script(
            &sophia.join("tools/run_sophia_session.sh"),
            r#"
echo session >> "$TRACE"
[[ "$#" == 2 && "$1" == --max-runtime-ms=90000 && "$2" == --wm-process=* ]]
[[ "$SOPHIA_SESSION_STARTUP" == none && "$SOPHIA_REQUIRE_LOCAL_VT" == true ]]
[[ "$SOPHIA_MANAGE_KEYD" == true && "$SOPHIA_SESSION_WATCHDOG_SECONDS" == 110 ]]
mkdir -p "$SOPHIA_DIAGNOSTIC_DIR"
printf '%s\n' 'sophia_tty_recovery schema=3 termios_restored=true done=true' 'sophia_tty_recovery_verification schema=1 keyd_restored=true' > "$SOPHIA_DIAGNOSTIC_DIR/recovery.log"
# Empty transcript must fail the real verifier after a simulated clean exit.
touch "$SOPHIA_DIAGNOSTIC_DIR/events.0.log"
exit "${SESSION_STATUS:-0}""#,
        );
        fs::create_dir_all(sophia.join("tools/fixtures")).unwrap();
        fs::write(
            sophia.join("tools/fixtures/native_launcher_core.kdl"),
            "fixture shared catalog\n",
        )
        .unwrap();
        let pins = [
            "tools/run_sophia_session.sh",
            "tools/fixtures/native_launcher_core.kdl",
        ]
        .map(|path| format!("{} {path}\n", sha256(&fs::read(sophia.join(path)).unwrap())))
        .concat();
        fs::write(root.join("pins/sophia-shared.sha256"), pins).unwrap();
        Self {
            directory,
            root,
            sophia,
            commits,
        }
    }
    fn run(&self, evidence: &str, proof: &str, session: &str) -> std::process::Output {
        let mut command = Command::new("timeout");
        command
            .args(["30", "bash"])
            .arg(self.root.join("tools/run_current_lom_panel_gate_tty4.sh"))
            .arg("dock")
            .env_clear()
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.directory.join("bin").display()),
            )
            .env("HOME", &self.directory)
            .env("SOPHIA_LOM_NATIVE_GATE_ARM", "1")
            .env("SOPHIA_SOURCE", &self.sophia)
            .env("SOPHIA_DESKTOP_PROFILE", self.directory.join("wm.kdl"))
            .env(
                "SOPHIA_LOM_NATIVE_EVIDENCE_DIR",
                self.directory.join(evidence),
            )
            .env("TRACE", self.directory.join("trace"))
            .env("PROOF_STATUS", proof)
            .env("SESSION_STATUS", session);
        for (kind, commit) in &self.commits {
            let upper = kind.to_ascii_uppercase();
            command
                .env(
                    format!("SOPHIA_{upper}_ARTIFACT"),
                    self.directory.join("artifacts").join(kind),
                )
                .env(format!("SOPHIA_{upper}_COMMIT"), commit);
        }
        command.output().unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn dock_launcher_uses_three_component_profile_and_refuses_failed_or_missing_evidence() {
    let f = Fixture::new();
    let failed_proof = f.run("proof-failure", "1", "0");
    assert!(!failed_proof.status.success());
    let trace = fs::read_to_string(f.directory.join("trace")).unwrap();
    assert!(
        trace.contains("proof"),
        "{}",
        String::from_utf8_lossy(&failed_proof.stderr)
    );
    assert!(!trace.contains("session"));
    fs::remove_file(f.directory.join("trace")).unwrap();
    let failed_session = f.run("watchdog", "0", "124");
    assert!(!failed_session.status.success());
    assert_eq!(
        fs::read_to_string(f.directory.join("watchdog/native-outcome.txt")).unwrap(),
        "native_exit_status=124\n"
    );
    let empty = f.run("empty", "0", "0");
    assert!(!empty.status.success());
    assert!(
        String::from_utf8_lossy(&empty.stderr).contains("missing/repeated lifecycle evidence"),
        "{}",
        String::from_utf8_lossy(&empty.stderr)
    );
    let config = fs::read_to_string(f.directory.join("empty/desktop.kdl")).unwrap();
    assert!(config.contains("shell-component \"dock\" \"dock\""));
    assert!(config.contains("reservation \"bottom\" 64"));
    assert!(config.contains("bind \"Super+Space\""));
    let manifest = fs::read_to_string(f.directory.join("empty/identity.manifest")).unwrap();
    assert!(manifest.contains("provlita_binary_sha256="));
    assert!(manifest.contains("latency_acceptance=NOT_RUN"));
    let provlita = &f.commits.iter().find(|(k, _)| *k == "provlita").unwrap().1;
    assert!(manifest.contains(&format!("provlita_commit={provlita}")));
    assert!(
        !f.run("empty", "0", "0").status.success(),
        "must not overwrite evidence"
    );
}
