# Provenance: moved from Sophia tools/probes/lom_workload/tests/test_launcher.py at 9fcaec782ce4fe9978568c0466ee17a78b3d4571 (Sophia rule 13).
"""Actual launcher control flow with fake build/proof/session executables.

No real source checkout, binary, device, endpoint, credential or VT is used.
The real transcript verifiers and the real artifact intake run; signatures,
builds and native results do not. Product artifacts are fixture directories
in the prepared-artifact layout, bound to fixture commit objects.
"""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixture import transcript, encode

REAL_GIT = shutil.which("git")
PRODUCT_KEYS = ("schema", "product", "binary", "binary_sha256", "source_commit", "source_tree",
                "signature_status", "signer_fingerprint", "config", "config_sha256")


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def commit_object(label):
    """A raw signed-looking commit object and its real Git identity."""
    raw = (f"tree {'1' * 40}\nauthor A U Thor <a@example.com> 0 +0000\n"
           f"committer A U Thor <a@example.com> 0 +0000\n"
           f"gpgsig -----BEGIN PGP SIGNATURE-----\n fixture\n -----END PGP SIGNATURE-----\n\n{label}\n").encode()
    commit = subprocess.run([REAL_GIT, "hash-object", "--stdin", "-t", "commit"], input=raw,
                            capture_output=True, check=True).stdout.decode().strip()
    return raw, commit


def write_artifact(directory, kind, sdk_manifest=None, label=None):
    """A prepared artifact directory for KIND; returns its commit."""
    directory.mkdir(parents=True)
    raw, commit = commit_object(label or kind)
    binary = "bemenu-sophia" if kind == "bemenu" else kind
    body = f"fixture {kind} binary; hash input only\n".encode()
    (directory / binary).write_bytes(body)
    (directory / "source.commit").write_bytes(raw)
    values = {"schema": "1", "binary": binary, "binary_sha256": sha256(body), "source_commit": commit,
              "source_tree": "1" * 40, "signature_status": "G", "signer_fingerprint": "ABCDEF0123"}
    if kind == "bemenu":
        values.update(sdk_revision="2" * 40, sdk_manifest_sha256=sha256(sdk_manifest.read_bytes()))
        keys = ("schema", "binary", "binary_sha256", "source_commit", "source_tree", "signature_status",
                "signer_fingerprint", "sdk_revision", "sdk_manifest_sha256")
        manifest = "bemenu-artifact.manifest"
    else:
        values["product"] = kind
        if kind == "hagia":
            values.update(config="none", config_sha256="none")
        else:
            config = f"fixture {kind} config\n".encode()
            (directory / "config.kdl").write_bytes(config)
            values.update(config="config.kdl", config_sha256=sha256(config))
        keys = PRODUCT_KEYS
        manifest = "product-artifact.manifest"
    (directory / manifest).write_text("".join(f"{k}={values[k]}\n" for k in keys))
    return commit


class LauncherTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        repo = Path(__file__).resolve().parents[4]
        self.root = self.base / "integration"
        self.tools = self.root / "tools"
        self.tools.mkdir(parents=True)
        shutil.copyfile(repo / "tools/run_current_lom_panel_gate_tty4.sh", self.tools / "run.sh")
        shutil.copyfile(repo / "tools/verify_lom_panel_native_gate.sh", self.tools / "verify_lom_panel_native_gate.sh")
        (self.tools / "verify_lom_panel_native_gate.sh").chmod(0o700)
        shutil.copytree(repo / "tools/lib", self.tools / "lib")
        shutil.copytree(repo / "tools/probes/lom_workload", self.tools / "probes/lom_workload",
                        ignore=shutil.ignore_patterns("__pycache__"))
        (self.tools / "fixtures").mkdir()
        for name in ("lom_panel_core.kdl", "lom_panel_desktop.kdl", "lom_workload_budgets.json"):
            shutil.copyfile(repo / "tools/fixtures" / name, self.tools / "fixtures" / name)
        (self.root / "pins/c-desktop-sdk").mkdir(parents=True)
        shutil.copyfile(repo / "pins/c-desktop-sdk/manifest.json", self.root / "pins/c-desktop-sdk/manifest.json")
        self.fakebin = self.base / "bin"
        self.fakebin.mkdir()
        # The explicit Sophia checkout under test: fake executables and the two
        # shared files whose digests the fixture pins (pin_shared).
        self.sophia = self.base / "sophia"
        (self.sophia / "tools/fixtures").mkdir(parents=True)
        (self.sophia / "tools/fixtures/native_launcher_core.kdl").write_text("fixture shared core catalog\n")
        for path in (self.sophia / "target/release/sophia",):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("not an executable; fixture hash input only\n")
        self.artifacts = self.base / "artifacts"
        self.lom_commit = write_artifact(self.artifacts / "lom", "lom")
        self.hagia_commit = write_artifact(self.artifacts / "hagia", "hagia")
        self.script(self.fakebin / "tty", 'printf "%s\\n" "${TEST_TTY:-/dev/tty4}"')
        self.script(self.fakebin / "git", f'''
case "$*" in
    *"status --short"*) printf "%s" "${{TEST_DIRTY:-}}" ;;
    *"rev-parse HEAD"*) printf '%040d\\n' 1 ;;
    *"verify-commit HEAD"*) exit 0 ;;
    *"hash-object --stdin -t commit"*) exec {REAL_GIT} hash-object --stdin -t commit ;;
    *) exit 99 ;;
esac''')
        self.script(self.fakebin / "cargo", 'echo build >> "$TEST_TRACE"')
        self.wm_profile = self.base / "wm.kdl"
        self.wm_profile.write_text('schema 1\nshortcut { profile "operator"; bind "Super+4" "policy:focus-workspace" "7"; }\n')
        # Configuration executables are supplied effects in this launcher test.
        # Rust desktop_probe controls cover the real parser/composition policy.
        self.script(self.sophia / "target/release/sophia", '''
[[ "$1" == config ]]
case "$2" in
  print-effective) [[ "$3" == --desktop-profile="$SOPHIA_DESKTOP_PROFILE" ]]; cat "$SOPHIA_DESKTOP_PROFILE" ;;
  check) [[ -f "${3#--desktop-profile=}" ]] ;;
  *) exit 99 ;;
esac''')
        examples = self.sophia / "target/release/examples"
        examples.mkdir()
        self.script(examples / "desktop_profile_probe", '''
[[ "$#" == 2 ]]
cat "$1"
tail -n +2 "$2"''')
        self.script(self.tools / "lom_gpu_content_hardware_proof.sh", '''
echo proof >> "$TEST_TRACE"
[[ "$SOPHIA_LOM_GPU_PROOF_ARM" == 1 ]]
[[ "$SOPHIA_SOURCE" == /* && "$SOPHIA_LOM_ARTIFACT" == /* && -n "$SOPHIA_LOM_COMMIT" ]]
exit "${TEST_PROOF_STATUS:-0}"''')
        self.script(self.sophia / "tools/run_sophia_session.sh", '''
echo session >> "$TEST_TRACE"
[[ "$#" == 3 && "$1" == --max-runtime-ms=90000 ]]
[[ "$2" == --shell-process="$SOPHIA_LOM_NATIVE_EVIDENCE_DIR/lom" ]]
[[ "$3" == --wm-process="$SOPHIA_HAGIA_BIN" ]]
[[ "$SOPHIA_HAGIA_BIN" == "$SOPHIA_LOM_NATIVE_EVIDENCE_DIR/hagia" ]]
[[ "$SOPHIA_SESSION_WATCHDOG_SECONDS" == 110 && "$SOPHIA_SESSION_STARTUP" == none ]]
[[ "$SOPHIA_REQUIRE_LOCAL_VT" == true && "$SOPHIA_MANAGE_KEYD" == true ]]
mkdir -p "$SOPHIA_DIAGNOSTIC_DIR"
cp "$TEST_HOST" "$SOPHIA_DIAGNOSTIC_DIR/events.0.log"
cp "$TEST_CLIENT" "$SOPHIA_UNTRUSTED_SESSION_OUTPUT_LOG"
if [[ "${TEST_RECOVERY:-yes}" == yes ]]; then
    printf '%s\\n' 'sophia_tty_recovery schema=3 termios_restored=true done=true' \\
      'sophia_tty_recovery_verification schema=1 keyd_restored=true' > "$SOPHIA_DIAGNOSTIC_DIR/recovery.log"
fi
if [[ "${TEST_CHANGE_INPUT:-no}" == yes ]]; then echo changed >> "$SOPHIA_SHELL_CONFIG"; fi
exit "${TEST_SESSION_STATUS:-0}"''')
        self.pin_shared()
        host, client = transcript()
        native = "\n".join([
            "sophia_live_shell_gpu schema=1 status=granted device=fixture",
            "sophia_live_wm_configuration schema=2 status=committed generation=1",
            "sophia_live_shell_content schema=1 status=outputs outputs=2",
            *[f"sophia_live_shell_content schema=1 status=presented output={o} candidate_generation={g} done=true"
              for o in (1, 2) for g in (1, 2)],
        ]) + "\n"
        (self.base / "host.log").write_text(native + encode(host))
        (self.base / "client.log").write_text(encode(client))
        self.evidence = self.base / "evidence"
        self.env = {**os.environ, "PATH": str(self.fakebin) + ":/usr/bin:/bin",
                    "SOPHIA_SOURCE": str(self.sophia),
                    "SOPHIA_LOM_ARTIFACT": str(self.artifacts / "lom"), "SOPHIA_LOM_COMMIT": self.lom_commit,
                    "SOPHIA_HAGIA_ARTIFACT": str(self.artifacts / "hagia"), "SOPHIA_HAGIA_COMMIT": self.hagia_commit,
                    "SOPHIA_LOM_NATIVE_GATE_ARM": "1",
                    "SOPHIA_DESKTOP_PROFILE": str(self.wm_profile),
                    "SOPHIA_LOM_NATIVE_EVIDENCE_DIR": str(self.evidence),
                    "TEST_TRACE": str(self.base / "trace"), "TEST_HOST": str(self.base / "host.log"),
                    "TEST_CLIENT": str(self.base / "client.log")}
        for name in ("SOPHIA_LOM_CORE_CONFIG", "DISPLAY", "WAYLAND_DISPLAY"):
            self.env.pop(name, None)

    def pin_shared(self):
        """Pin the fixture Sophia checkout's shared files, as pins/ does for real."""
        lines = [f"{sha256((self.sophia / path).read_bytes())} {path}\n"
                 for path in ("tools/run_sophia_session.sh", "tools/fixtures/native_launcher_core.kdl")]
        (self.root / "pins/sophia-shared.sha256").write_text("".join(lines))

    def script(self, path, body):
        path.write_text("#!/usr/bin/env bash\nset -euo pipefail\n" + body + "\n")
        path.chmod(0o700)

    def run_launcher(self, **env):
        return subprocess.run(["bash", str(self.tools / "run.sh")], env={**self.env, **env},
                              capture_output=True, text=True, timeout=10, umask=0o002)

    def test_generated_profiles_are_private_under_group_writable_umask(self):
        result = self.run_launcher()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for name in ("wm-profile.kdl", "desktop.kdl"):
            self.assertEqual((self.evidence / name).stat().st_mode & 0o777, 0o600)

    def test_normal_exit_runs_both_real_verifiers(self):
        result = self.run_launcher()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        report = json.loads((self.evidence / "workload-verification.json").read_text())
        self.assertEqual(report["status"], "pass")
        self.assertEqual(report["memory"]["slot_bound"], 4)
        self.assertEqual((self.evidence / "native-outcome.txt").read_text(), "native_exit_status=0\n")
        self.assertEqual((self.base / "trace").read_text().splitlines(), ["build", "build", "proof", "session"])
        self.assertEqual((self.evidence / "wm-profile.kdl").read_bytes(), self.wm_profile.read_bytes())
        self.assertIn('bind "Super+4" "policy:focus-workspace" "7"', (self.evidence / "desktop.kdl").read_text())
        self.assertIn("wm_profile_sha256=", (self.evidence / "identity.manifest").read_text())
        self.assertIn("probe_overrides_sha256=", (self.evidence / "identity.manifest").read_text())
        self.assertIn(f"lom_commit={self.lom_commit}", (self.evidence / "identity.manifest").read_text())
        self.assertIn(f"hagia_commit={self.hagia_commit}", (self.evidence / "identity.manifest").read_text())
        self.assertIn("integration_commit=", (self.evidence / "identity.manifest").read_text())
        self.assertIn(str(self.sophia / "tools/run_sophia_session.sh"), (self.evidence / "inputs.sha256").read_text())

    def test_missing_selected_wm_profile_never_launches_proof_or_session(self):
        result = self.run_launcher(SOPHIA_DESKTOP_PROFILE=str(self.base / "missing.kdl"))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("existing absolute WM profile", result.stderr)
        self.assertNotIn("proof", (self.base / "trace").read_text())
        self.assertNotIn("session", (self.base / "trace").read_text())

    def test_preconditions_stop_before_build_and_proof(self):
        for env in ({"TEST_TTY": "/dev/pts/1"}, {"SOPHIA_LOM_NATIVE_GATE_ARM": "0"}, {"TEST_DIRTY": " M fixture"}):
            with self.subTest(env=env):
                self.assertNotEqual(self.run_launcher(**env).returncode, 0)
                self.assertFalse((self.base / "trace").exists())

    def test_missing_explicit_inputs_stop_before_build(self):
        for name in ("SOPHIA_SOURCE", "SOPHIA_LOM_ARTIFACT", "SOPHIA_LOM_COMMIT",
                     "SOPHIA_HAGIA_ARTIFACT", "SOPHIA_HAGIA_COMMIT"):
            with self.subTest(name=name):
                env = {k: v for k, v in self.env.items() if k != name}
                result = subprocess.run(["bash", str(self.tools / "run.sh")], env=env,
                                        capture_output=True, text=True, timeout=10)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(f"{name} is required", result.stderr)
                self.assertFalse((self.base / "trace").exists())
                self.assertFalse(self.evidence.exists())

    def test_unpinned_shared_sophia_file_stops_before_build(self):
        with (self.sophia / "tools/run_sophia_session.sh").open("a") as script:
            script.write("# drift\n")
        result = self.run_launcher()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("differs from pins/sophia-shared.sha256", result.stderr)
        self.assertFalse((self.base / "trace").exists())

    def test_artifact_binding_failures_stop_before_build(self):
        other = write_artifact(self.base / "other", "lom", label="another signed revision")
        tamper = self.base / "tampered"
        shutil.copytree(self.artifacts / "lom", tamper)
        (tamper / "lom").write_text("substituted binary\n")
        cases = {
            "commit mismatch": {"SOPHIA_LOM_COMMIT": other},
            "SHA-256 mismatch": {"SOPHIA_LOM_ARTIFACT": str(tamper)},
            "product": {"SOPHIA_HAGIA_ARTIFACT": str(self.artifacts / "lom"),
                        "SOPHIA_HAGIA_COMMIT": self.lom_commit},
            "absolute directory": {"SOPHIA_LOM_ARTIFACT": "artifacts/lom"},
        }
        for message, env in cases.items():
            with self.subTest(message=message):
                shutil.rmtree(self.evidence, ignore_errors=True)
                (self.base / "trace").unlink(missing_ok=True)
                result = self.run_launcher(**env)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(message, result.stderr)
                self.assertFalse((self.base / "trace").exists())

    def test_failed_proof_never_launches_session(self):
        self.assertNotEqual(self.run_launcher(TEST_PROOF_STATUS="1").returncode, 0)
        self.assertIn("proof", (self.base / "trace").read_text())
        self.assertNotIn("session", (self.base / "trace").read_text())

    def test_watchdog_is_failure_and_preserves_existing_evidence(self):
        self.assertNotEqual(self.run_launcher(TEST_SESSION_STATUS="124").returncode, 0)
        self.assertEqual((self.evidence / "native-outcome.txt").read_text(), "native_exit_status=124\n")
        old = (self.base / "trace").read_bytes()
        self.assertNotEqual(self.run_launcher().returncode, 0)
        self.assertEqual((self.base / "trace").read_bytes(), old)

    def test_missing_recovery_fails(self):
        result = self.run_launcher(TEST_RECOVERY="no")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.evidence / "workload-verification.json").exists())

    def test_changed_inputs_fail(self):
        result = self.run_launcher(TEST_CHANGE_INPUT="yes")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.evidence / "workload-verification.json").exists())

    def test_missing_workload_action_fails_real_verifier(self):
        path = self.base / "host.log"
        lines = path.read_text().splitlines()
        lines = [line for line in lines if not ("sophia_shell_action_cause" in line and "event_id=1 " in line)]
        path.write_text("\n".join(lines) + "\n")
        self.assertNotEqual(self.run_launcher().returncode, 0)
        result = json.loads((self.evidence / "workload-verification.json").read_text())
        self.assertEqual(result["status"], "fail")
