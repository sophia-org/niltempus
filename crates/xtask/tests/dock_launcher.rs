// Provenance: moved from Sophia crates/xtask/tests/dock_launcher.rs at
// 9fcaec782ce4fe9978568c0466ee17a78b3d4571 (Sophia rule 13). Adapted to the
// explicit inputs: a fixture Sophia Git checkout pinned by revision and staged
// by the gate, a private build directory, and prepared product artifacts bound
// to fixture commits and expected digests.
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

fn git(directory: &Path, args: &[&str]) -> String {
    let out = Command::new(real_git())
        .arg("-C")
        .arg(directory)
        .args(["-c", "user.name=Fixture", "-c", "user.email=f@example.com"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// Expected identity of a prepared artifact: commit, binary and config digests.
struct Identity {
    commit: String,
    sha256: String,
    config_sha256: Option<String>,
}

/// A prepared artifact directory and its expected identity.
fn artifact(directory: &Path, kind: &str, sdk_manifest: &Path) -> Identity {
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
    child
        .stdin
        .take()
        .unwrap()
        .write_all(raw.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let commit = String::from_utf8(out.stdout).unwrap().trim().to_owned();
    let binary = if kind == "bemenu" {
        "bemenu-sophia"
    } else {
        kind
    };
    let body = format!("fixture {kind} binary\n");
    fs::write(directory.join(binary), &body).unwrap();
    fs::write(directory.join("source.commit"), &raw).unwrap();
    let common = format!(
        "binary={binary}\nbinary_sha256={}\nsource_commit={commit}\nsource_tree={}\n\
         signature_status=G\nsigner_fingerprint=ABCDEF0123\n",
        sha256(body.as_bytes()),
        "1".repeat(40)
    );
    let mut config_sha256 = None;
    let (name, manifest) = if kind == "bemenu" {
        let sdk = fs::read_to_string(sdk_manifest).unwrap();
        let revision = sdk
            .split_once("\"revision\": \"")
            .and_then(|(_, rest)| rest.get(..40))
            .unwrap()
            .to_owned();
        (
            "bemenu-artifact.manifest",
            format!(
                "schema=1\n{common}sdk_revision={revision}\nsdk_manifest_sha256={}\n",
                sha256(sdk.as_bytes())
            ),
        )
    } else {
        let config = if kind == "hagia" {
            "config=none\nconfig_sha256=none\n".to_owned()
        } else {
            let text = format!("fixture {kind} configuration\n");
            fs::write(directory.join("config.kdl"), &text).unwrap();
            let digest = sha256(text.as_bytes());
            config_sha256 = Some(digest.clone());
            format!("config=config.kdl\nconfig_sha256={digest}\n")
        };
        (
            "product-artifact.manifest",
            format!("schema=1\nproduct={kind}\n{common}{config}"),
        )
    };
    fs::write(directory.join(name), manifest).unwrap();
    Identity {
        commit,
        sha256: sha256(body.as_bytes()),
        config_sha256,
    }
}

struct Fixture {
    directory: PathBuf,
    root: PathBuf,
    sophia: PathBuf,
    build: PathBuf,
    identities: Vec<(&'static str, Identity)>,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("dock-launcher-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let root = directory.join("integration");
        let sophia = directory.join("sophia");
        let build = directory.join("build");
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for name in [
            "tools/fixtures",
            "tools/lib",
            "tools/probes/lom_workload",
            "tools/session",
            "pins/c-desktop-sdk",
            ".provision",
        ] {
            fs::create_dir_all(root.join(name)).unwrap();
        }
        // The runner consumes prebuilt integration tools; only the bounded
        // input-preparation operation is stubbed below.
        fs::write(
            root.join(".provision/accepted"),
            "url=fixture\nrev=fixture\ncargo_lock_sha256=fixture\ncargo_home=/nonexistent/fixture-cargo-home\n",
        )
        .unwrap();
        for name in [
            "tools/run_current_lom_panel_gate_tty4.sh",
            "tools/session/run_desktop_session.sh",
            "tools/lib/artifacts.sh",
            "tools/lib/physical_inputs.sh",
            "tools/fixtures/lom_panel_desktop.kdl",
            "tools/fixtures/lom_workload_budgets.json",
            "tools/probes/lom_workload/verify.py",
            "tools/probes/lom_workload/records.py",
            "tools/probes/lom_workload/memory.py",
            "pins/c-desktop-sdk/manifest.json",
        ] {
            fs::copy(repo.join(name), root.join(name)).unwrap();
        }
        fs::create_dir_all(build.join("integration-target/release")).unwrap();
        fs::copy(
            env!("CARGO_BIN_EXE_xtask"),
            build.join("integration-target/release/xtask-real"),
        )
        .unwrap();
        fs::copy(
            env!("CARGO_BIN_EXE_active-session-preflight"),
            build.join("integration-target/release/active-session-preflight"),
        )
        .unwrap();
        let sdk = root.join("pins/c-desktop-sdk/manifest.json");
        let identities = ["lom", "hagia", "bemenu", "provlita"]
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
if [[ "${{1:-}}" == -C && "${{2:-}}" == "{root}" ]]; then
    case "$*" in
        *'status --short'*) ;;
        *'rev-parse HEAD'*) printf '%040d\n' 1 ;;
        *'verify-commit '*) ;;
        *) exit 99 ;;
    esac
    exit
fi
case "$*" in *'verify-commit '*) exit 0 ;; esac
exec {git} "$@""#,
                root = root.display(),
                git = real_git().display()
            ),
        );
        script(
            &directory.join("bin/cargo"),
            "echo forbidden-build >> \"$TRACE\"; exit 99",
        );
        script(
            &build.join("integration-target/release/xtask"),
            r#"
if [[ "$1" != prepare-physical-inputs ]]; then
    exec "$(dirname "$0")/xtask-real" "$@"
fi
shift
sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
if [[ "${1:-}" == verify ]]; then
    [[ "$*" == *"--manifest-sha256=$sha"* ]] || exit 99
    exit 0
fi
[[ "$CARGO_BUILD_JOBS" == 2 && "$(ps -o ni= -p $$ | tr -d ' ')" == 19 ]] || exit 98
[[ "$*" == *--sophia-packages=sophia-cli,sophia-conformance* ]] || exit 97
out=
for arg in "$@"; do
    case "$arg" in --out=*) out=${arg#--out=} ;; esac
done
[[ "$out" == "$SOPHIA_GATE_BUILD_DIR/"* && ! -e "$out" ]] || exit 96
mkdir -m 700 "$out"
cat >"$out/inputs.env" <<ENV
SOPHIA_PHYSICAL_INPUTS=$out
SOPHIA_ROOT=$SOPHIA_GATE_BUILD_DIR/sophia-tree
SOPHIA_COMMIT=$(git -C "$SOPHIA_SOURCE" rev-parse HEAD)
SOPHIA_INTEGRATION_COMMIT=0000000000000000000000000000000000000001
SOPHIA_BIN=$SOPHIA_GATE_BUILD_DIR/sophia-target/release/sophia
SOPHIA_PROFILE_PROBE_BIN=$SOPHIA_GATE_BUILD_DIR/sophia-target/release/examples/desktop_profile_probe
ENV
echo build >> "$TRACE"
echo "physical_inputs status=prepared manifest_sha256=$sha dir=$out"
"#,
        );
        script(
            &build.join("sophia-target/release/sophia"),
            r#"case "$1 $2" in
    "config print-effective") cat "$SOPHIA_DESKTOP_PROFILE" ;;
    "config check") test -f "${3#--desktop-profile=}" ;;
    "session check-host") [[ "$#" == 3 && "$3" == --tty=/dev/tty4 ]] && echo host >> "$TRACE" ;;
    *) exit 99 ;;
esac"#,
        );
        script(
            &build.join("sophia-target/release/examples/desktop_profile_probe"),
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
# Reached through the external launcher: `-- session run <arguments>`, the
# opaque label, and the host checker supplied as an absolute path.
[[ "$1 $2 $3" == "-- session run" && "$SOPHIA_TTY_PROFILE" == managed ]]
[[ "$SOPHIA_SESSION_PREFLIGHT" == /*/integration-target/release/active-session-preflight ]]
shift 3
selectors=0
for argument in "$@"; do
    case "$argument" in --input-seat=*|--input-devices=*) selectors=$((selectors + 1)) ;; esac
done
[[ "$selectors" == 1 ]]
[[ "${@: -2:1}" == --max-runtime-ms=90000 && "${@: -1}" == --wm-process=* ]]
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
        git(&directory, &["init", "-q", "sophia"]);
        git(&sophia, &["add", "-A"]);
        git(&sophia, &["commit", "-q", "-m", "fixture"]);
        let rev = git(&sophia, &["rev-parse", "HEAD"]);
        fs::write(
            root.join("pins/sophia.toml"),
            format!(
                "url = \"https://github.com/sophia-org/sophia.git\"\nrev = \"{}\"\n",
                rev.trim()
            ),
        )
        .unwrap();
        Self {
            directory,
            root,
            sophia,
            build,
            identities,
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
            .env("SOPHIA_GATE_BUILD_DIR", &self.build)
            .env("CARGO_HOME", self.directory.join("cargo-home"))
            .env(
                "SOPHIA_INTEGRATION_XTASK",
                self.build.join("integration-target/release/xtask"),
            )
            .env(
                "SOPHIA_SESSION_PREFLIGHT",
                self.build
                    .join("integration-target/release/active-session-preflight"),
            )
            .env("SOPHIA_DESKTOP_PROFILE", self.directory.join("wm.kdl"))
            .env(
                "SOPHIA_LOM_NATIVE_EVIDENCE_DIR",
                self.directory.join(evidence),
            )
            .env("TRACE", self.directory.join("trace"))
            .env("PROOF_STATUS", proof)
            .env("SESSION_STATUS", session);
        for (kind, identity) in &self.identities {
            let upper = kind.to_ascii_uppercase();
            command
                .env(
                    format!("SOPHIA_{upper}_ARTIFACT"),
                    self.directory.join("artifacts").join(kind),
                )
                .env(format!("SOPHIA_{upper}_COMMIT"), &identity.commit)
                .env(format!("SOPHIA_{upper}_SHA256"), &identity.sha256);
            if let Some(config) = &identity.config_sha256 {
                command.env(format!("SOPHIA_{upper}_CONFIG_SHA256"), config);
            }
        }
        command.output().unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // The staged Sophia tree is read-only by design.
        let _ = Command::new("chmod")
            .args(["-R", "u+w"])
            .arg(&self.directory)
            .status();
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
    assert!(!trace.contains("session") && !trace.contains("host"));
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
    let provlita = &f
        .identities
        .iter()
        .find(|(k, _)| *k == "provlita")
        .unwrap()
        .1;
    assert!(manifest.contains(&format!("provlita_commit={}", provlita.commit)));
    // Builds and staged Sophia files stay in the private build directory.
    assert!(
        f.build
            .join("sophia-tree/tools/run_sophia_session.sh")
            .is_file()
    );
    assert!(!f.sophia.join("target").exists() && !f.root.join("target").exists());
    assert!(
        !f.run("empty", "0", "0").status.success(),
        "must not overwrite evidence"
    );
}
