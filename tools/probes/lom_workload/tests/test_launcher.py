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
# Sophia's retained wrapper is reached only through the external launcher
# (tools/session/run_desktop_session.sh): `-- session run <prepared
# arguments>` under the opaque label, with the host checker supplied as an
# absolute path and exactly one input selector. The stub checks that once,
# then reduces its arguments to the runner's own trailing ones, which the
# recipe appends last, so the controls below keep asserting exactly those.
WRAPPER_PROLOGUE = '''
if [[ "${1:-}" == -- ]]; then
    [[ "$2 $3" == "session run" && "$SOPHIA_TTY_PROFILE" == managed ]]
    [[ "$SOPHIA_SESSION_PREFLIGHT" == */integration-target/release/active-session-preflight ]]
    selectors=0
    for argument in "$@"; do
        case "$argument" in --input-seat=*|--input-devices=*) selectors=$((selectors + 1)) ;; esac
    done
    [[ "$selectors" == 1 ]]
    [[ " $* " == *" --wm-process-default=$SOPHIA_HAGIA_BIN "* ]]
    while [[ "$#" -gt 0 && "$1" != --max-runtime-ms=* ]]; do shift; done
    export TEST_VIA_LAUNCHER=1
fi
[[ "${TEST_VIA_LAUNCHER:-}" == 1 ]]
'''
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


def write_artifact(directory, kind, sdk_manifest=None, label=None, sdk_revision=None):
    """A prepared artifact directory for KIND; returns commit and expected digests."""
    directory.mkdir(parents=True)
    raw, commit = commit_object(label or kind)
    binary = "bemenu-sophia" if kind == "bemenu" else kind
    body = f"fixture {kind} binary; hash input only\n".encode()
    (directory / binary).write_bytes(body)
    (directory / "source.commit").write_bytes(raw)
    values = {"schema": "1", "binary": binary, "binary_sha256": sha256(body), "source_commit": commit,
              "source_tree": "1" * 40, "signature_status": "G", "signer_fingerprint": "ABCDEF0123"}
    if kind == "bemenu":
        pinned = json.loads(sdk_manifest.read_text())["revision"]
        values.update(sdk_revision=sdk_revision or pinned, sdk_manifest_sha256=sha256(sdk_manifest.read_bytes()))
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
    return {"commit": commit, "sha256": values["binary_sha256"], "config_sha256": values.get("config_sha256")}


def rewrite_manifest(directory, **changes):
    """Replace manifest values in place (the manifest is unsigned)."""
    manifest = next(directory.glob("*.manifest"))
    lines = []
    for line in manifest.read_text().splitlines():
        key, value = line.split("=", 1)
        lines.append(f"{key}={changes.get(key, value)}")
    manifest.write_text("\n".join(lines) + "\n")


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
        (self.tools / "session").mkdir()
        shutil.copy2(repo / "tools/session/run_desktop_session.sh", self.tools / "session/run_desktop_session.sh")
        # The runner builds the recipe tool and host checker from an accepted
        # private CARGO_HOME (the fixture's cargo is a stub) and runs the real
        # binaries, which the gate supplies from its own build.
        (self.root / ".provision").mkdir()
        (self.root / ".provision/accepted").write_text(
            "url=fixture\nrev=fixture\ncargo_lock_sha256=fixture\ncargo_home=/nonexistent/fixture-cargo-home\n")
        shutil.copytree(repo / "tools/probes/lom_workload", self.tools / "probes/lom_workload",
                        ignore=shutil.ignore_patterns("__pycache__"))
        (self.tools / "fixtures").mkdir()
        for name in ("lom_panel_core.kdl", "lom_panel_desktop.kdl", "lom_workload_budgets.json"):
            shutil.copyfile(repo / "tools/fixtures" / name, self.tools / "fixtures" / name)
        (self.root / "pins/c-desktop-sdk").mkdir(parents=True)
        shutil.copyfile(repo / "pins/c-desktop-sdk/manifest.json", self.root / "pins/c-desktop-sdk/manifest.json")
        self.fakebin = self.base / "bin"
        self.fakebin.mkdir()
        # The explicit private build directory: fake build outputs live here.
        self.build = self.base / "build"
        release = self.build / "sophia-target/release"
        (release / "examples").mkdir(parents=True)
        integration = self.build / "integration-target/release"
        integration.mkdir(parents=True)
        for name, variable in (("xtask", "INTEGRATION_TEST_XTASK"),
                               ("active-session-preflight", "INTEGRATION_TEST_PREFLIGHT")):
            built = os.environ.get(variable, "")
            if not built.startswith("/") or not os.access(built, os.X_OK):
                raise RuntimeError(f"{variable} must name this repository's built {name} (absolute)")
            shutil.copy2(built, integration / name)
        # The explicit Sophia checkout: a real Git repository whose HEAD the
        # fixture pins (commit_sophia). The gate stages its exact tree.
        self.sophia = self.base / "sophia"
        (self.sophia / "tools/fixtures").mkdir(parents=True)
        (self.sophia / "tools/fixtures/native_launcher_core.kdl").write_text("fixture shared core catalog\n")
        (self.sophia / ".gitignore").write_text("/tools/lib/untracked-*.sh\n")
        self.artifacts = self.base / "artifacts"
        self.lom = write_artifact(self.artifacts / "lom", "lom")
        self.hagia = write_artifact(self.artifacts / "hagia", "hagia")
        self.script(self.fakebin / "tty", 'printf "%s\\n" "${TEST_TTY:-/dev/tty4}"')
        # This repository is not a Git checkout in the fixture; the Sophia
        # fixture is, and every other Git call reaches the real Git.
        self.script(self.fakebin / "git", f'''
if [[ "${{1:-}}" == -C && "${{2:-}}" == "$TEST_INTEGRATION_ROOT" ]]; then
    case "$*" in
        *"status --short"*) printf "%s" "${{TEST_DIRTY:-}}" ;;
        *"rev-parse HEAD"*) printf '%040d\\n' 1 ;;
        *"verify-commit HEAD"*) exit 0 ;;
        *) exit 99 ;;
    esac
    exit
fi
case "$*" in
    *"verify-commit "*) exit "${{TEST_BADSIG:-0}}" ;;
esac
exec {REAL_GIT} "$@"''')
        self.script(self.fakebin / "cargo", 'echo build >> "$TEST_TRACE"')
        self.wm_profile = self.base / "wm.kdl"
        self.wm_profile.write_text('schema 1\nshortcut { profile "operator"; bind "Super+4" "policy:focus-workspace" "7"; }\n')
        # Configuration executables are supplied effects in this launcher test.
        # Rust desktop_probe controls cover the real parser/composition policy.
        self.script(release / "sophia", '''
if [[ "$1 $2" == "session check-host" ]]; then
    [[ "$#" == 3 && "$3" == --tty=/dev/tty4 ]]
    echo host >> "$TEST_TRACE"
    exit 0
fi
[[ "$1" == config ]]
case "$2" in
  print-effective) [[ "$3" == --desktop-profile="$SOPHIA_DESKTOP_PROFILE" ]]; cat "$SOPHIA_DESKTOP_PROFILE" ;;
  check) [[ -f "${3#--desktop-profile=}" ]] ;;
  *) exit 99 ;;
esac''')
        self.script(release / "examples/desktop_profile_probe", '''
[[ "$#" == 2 ]]
cat "$1"
tail -n +2 "$2"''')
        self.script(self.tools / "lom_gpu_content_hardware_proof.sh", '''
echo proof >> "$TEST_TRACE"
[[ "$SOPHIA_LOM_GPU_PROOF_ARM" == 1 ]]
[[ "$SOPHIA_SOURCE" == /* && "$SOPHIA_GATE_BUILD_DIR" == /* && "$SOPHIA_LOM_ARTIFACT" == /* ]]
[[ -n "$SOPHIA_LOM_COMMIT" && -n "$SOPHIA_LOM_SHA256" && -n "$SOPHIA_LOM_CONFIG_SHA256" ]]
exit "${TEST_PROOF_STATUS:-0}"''')
        self.script(self.sophia / "tools/run_sophia_session.sh", WRAPPER_PROLOGUE + '''
echo session >> "$TEST_TRACE"
# Staged, never the operator's checkout: an input missing from the pinned
# tree (TEST_SOURCE_UNTRACKED) is unavailable here.
[[ -z "${TEST_SOURCE_UNTRACKED:-}" ]] || source "$(dirname "$0")/lib/untracked-input.sh"
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
        subprocess.run([REAL_GIT, "init", "-q", str(self.sophia)], check=True)
        self.commit_sophia()
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
        # Hermetic: no ambient SOPHIA_* (an operator's session variables would
        # otherwise reach the recipes).
        ambient = {k: v for k, v in os.environ.items() if not k.startswith("SOPHIA_")}
        self.env = {**ambient, "PATH": str(self.fakebin) + ":/usr/bin:/bin",
                    "TEST_INTEGRATION_ROOT": str(self.root),
                    "SOPHIA_SOURCE": str(self.sophia), "SOPHIA_GATE_BUILD_DIR": str(self.build),
                    "SOPHIA_LOM_NATIVE_GATE_ARM": "1",
                    "SOPHIA_DESKTOP_PROFILE": str(self.wm_profile),
                    "SOPHIA_LOM_NATIVE_EVIDENCE_DIR": str(self.evidence),
                    "TEST_TRACE": str(self.base / "trace"), "TEST_HOST": str(self.base / "host.log"),
                    "TEST_CLIENT": str(self.base / "client.log")}
        self.use_artifact("lom", self.artifacts / "lom", self.lom)
        self.use_artifact("hagia", self.artifacts / "hagia", self.hagia)
        for name in ("SOPHIA_LOM_CORE_CONFIG", "DISPLAY", "WAYLAND_DISPLAY"):
            self.env.pop(name, None)

    def use_artifact(self, kind, directory, identity):
        prefix = f"SOPHIA_{kind.upper()}_"
        self.env[prefix + "ARTIFACT"] = str(directory)
        self.env[prefix + "COMMIT"] = identity["commit"]
        self.env[prefix + "SHA256"] = identity["sha256"]
        if identity["config_sha256"] not in (None, "none"):
            self.env[prefix + "CONFIG_SHA256"] = identity["config_sha256"]

    def commit_sophia(self, pin=True):
        """Commit the fixture Sophia checkout and pin its HEAD, as pins/ does for real."""
        git = [REAL_GIT, "-C", str(self.sophia), "-c", "user.name=Fixture", "-c", "user.email=f@example.com",
               "-c", "commit.gpgsign=false"]
        subprocess.run(git + ["add", "-A"], check=True)
        subprocess.run(git + ["commit", "-q", "--allow-empty", "-m", "fixture"], check=True)
        if pin:
            rev = subprocess.run([REAL_GIT, "-C", str(self.sophia), "rev-parse", "HEAD"],
                                 capture_output=True, text=True, check=True).stdout.strip()
            (self.root / "pins/sophia.toml").write_text(
                f'url = "https://github.com/sophia-org/sophia.git"\nrev = "{rev}"\n')

    def script(self, path, body):
        path.write_text("#!/usr/bin/env bash\nset -euo pipefail\n" + body + "\n")
        path.chmod(0o700)

    def run_launcher(self, **env):
        return subprocess.run(["bash", str(self.tools / "run.sh")], env={**self.env, **env},
                              capture_output=True, text=True, timeout=20, umask=0o002)

    def assert_refused_before_build(self, message, **env):
        shutil.rmtree(self.evidence, ignore_errors=True)
        (self.base / "trace").unlink(missing_ok=True)
        result = self.run_launcher(**env)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(message, result.stderr)
        self.assertFalse((self.base / "trace").exists())

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
        self.assertEqual((self.base / "trace").read_text().splitlines(), ["build", "build", "build", "proof", "host", "session"])
        self.assertEqual((self.evidence / "wm-profile.kdl").read_bytes(), self.wm_profile.read_bytes())
        self.assertIn('bind "Super+4" "policy:focus-workspace" "7"', (self.evidence / "desktop.kdl").read_text())
        manifest = (self.evidence / "identity.manifest").read_text()
        for field in ("wm_profile_sha256=", "probe_overrides_sha256=", "integration_commit=",
                      f"lom_commit={self.lom['commit']}", f"hagia_commit={self.hagia['commit']}"):
            self.assertIn(field, manifest)
        # Builds and staged Sophia files live only in the private build directory.
        self.assertTrue((self.build / "sophia-tree/tools/run_sophia_session.sh").is_file())
        self.assertFalse((self.sophia / "target").exists())
        self.assertFalse((self.root / "target").exists())

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
        for name in ("SOPHIA_SOURCE", "SOPHIA_GATE_BUILD_DIR", "SOPHIA_LOM_ARTIFACT", "SOPHIA_LOM_COMMIT",
                     "SOPHIA_LOM_SHA256", "SOPHIA_LOM_CONFIG_SHA256",
                     "SOPHIA_HAGIA_ARTIFACT", "SOPHIA_HAGIA_COMMIT", "SOPHIA_HAGIA_SHA256"):
            with self.subTest(name=name):
                env = {k: v for k, v in self.env.items() if k != name}
                result = subprocess.run(["bash", str(self.tools / "run.sh")], env=env,
                                        capture_output=True, text=True, timeout=20)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(f"{name} is required", result.stderr)
                self.assertFalse((self.base / "trace").exists())
                self.assertFalse(self.evidence.exists())

    def test_build_directory_must_be_explicit_private_and_outside_sources(self):
        for message, value in {"must be absolute": "build",
                               "outside every source tree": str(self.sophia / "build"),
                               "outside every source tree ": str(self.root / "target")}.items():
            with self.subTest(value=value):
                self.assert_refused_before_build(message.strip(), SOPHIA_GATE_BUILD_DIR=value)

    def test_sophia_source_must_be_the_pinned_signed_clean_revision(self):
        with self.subTest("unsigned"):
            self.assert_refused_before_build("is not a good signed commit", TEST_BADSIG="1")
        with self.subTest("dirty"):
            (self.sophia / "tools/fixtures/native_launcher_core.kdl").write_text("drift\n")
            self.assert_refused_before_build("Sophia checkout must be clean")
            subprocess.run([REAL_GIT, "-C", str(self.sophia), "checkout", "-q", "--", "."], check=True)
        with self.subTest("another revision"):
            (self.sophia / "tools/fixtures/native_launcher_core.kdl").write_text("next revision\n")
            self.commit_sophia(pin=False)
            self.assert_refused_before_build("is not the pinned revision")

    def test_staged_tree_must_hash_to_the_pinned_tree(self):
        # An export-ignore attribute makes the archive differ from the tree.
        (self.sophia / ".gitattributes").write_text("tools/fixtures/* export-ignore\n")
        self.commit_sophia()
        self.assert_refused_before_build("staged Sophia tree")

    def test_symlinked_staging_destination_is_refused_before_any_change(self):
        outside = self.base / "outside"
        (outside / "nested").mkdir(parents=True)
        sentinel = outside / "nested/sentinel"
        sentinel.write_text("sentinel\n")
        sentinel.chmod(0o640)
        os.utime(sentinel, (1_000_000_000, 1_000_000_000))
        outside.chmod(0o500)
        before = [(p, p.read_bytes() if p.is_file() else None, p.lstat().st_mode, p.lstat().st_mtime_ns)
                  for p in (outside, outside / "nested", sentinel)]
        self.build.mkdir(parents=True, exist_ok=True)
        (self.build / "sophia-tree").symlink_to(outside)
        try:
            self.assert_refused_before_build("staged tree destination is a symlink")
            after = [(p, p.read_bytes() if p.is_file() else None, p.lstat().st_mode, p.lstat().st_mtime_ns)
                     for p in (outside, outside / "nested", sentinel)]
            self.assertEqual(before, after)
        finally:
            outside.chmod(0o700)

    def test_symlinked_build_directory_is_refused(self):
        real = self.base / "real-build"
        real.mkdir()
        link = self.base / "linked-build"
        link.symlink_to(real)
        self.assert_refused_before_build("must be owned by this user", SOPHIA_GATE_BUILD_DIR=str(link))
        self.assertEqual(list(real.iterdir()), [])

    def test_a_verified_staged_tree_is_reused_read_only(self):
        self.assertEqual(self.run_launcher().returncode, 0)
        staged = self.build / "sophia-tree/tools/run_sophia_session.sh"
        identity = (staged.stat().st_ino, staged.stat().st_mtime_ns)
        self.assertEqual(staged.stat().st_mode & 0o222, 0)
        result = self.run_launcher(SOPHIA_LOM_NATIVE_EVIDENCE_DIR=str(self.base / "evidence-again"))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((staged.stat().st_ino, staged.stat().st_mtime_ns), identity)

    def test_a_modified_staged_tree_is_replaced_not_reused(self):
        self.assertEqual(self.run_launcher().returncode, 0)
        staged = self.build / "sophia-tree/tools/run_sophia_session.sh"
        staged.parent.chmod(0o700)
        staged.chmod(0o700)
        with staged.open("a") as script:
            script.write("# tampered\n")
        result = self.run_launcher(SOPHIA_LOM_NATIVE_EVIDENCE_DIR=str(self.base / "evidence-again"))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("# tampered", staged.read_text())

    def test_an_input_outside_the_pinned_tree_is_unavailable(self):
        # Ignored and untracked: the checkout stays clean, but the pinned tree
        # lacks it, so the staged session runner cannot read it.
        (self.sophia / "tools/lib").mkdir(parents=True)
        (self.sophia / "tools/lib/untracked-input.sh").write_text("true\n")
        result = self.run_launcher(TEST_SOURCE_UNTRACKED="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.evidence / "native-outcome.txt").read_text(), "native_exit_status=1\n")

    def test_artifact_binding_failures_stop_before_build(self):
        other = write_artifact(self.base / "other", "lom", label="another signed revision")
        tamper = self.base / "tampered"
        shutil.copytree(self.artifacts / "lom", tamper)
        (tamper / "lom").write_text("substituted binary\n")
        # Binary and unsigned manifest replaced together: self-consistent, but
        # not the operator's expected digest.
        swapped = self.base / "swapped"
        shutil.copytree(self.artifacts / "lom", swapped)
        (swapped / "lom").write_text("substituted binary\n")
        rewrite_manifest(swapped, binary_sha256=sha256(b"substituted binary\n"))
        config = self.base / "config-swapped"
        shutil.copytree(self.artifacts / "lom", config)
        (config / "config.kdl").write_text("substituted configuration\n")
        rewrite_manifest(config, config_sha256=sha256(b"substituted configuration\n"))
        cases = {
            "commit mismatch": {"SOPHIA_LOM_COMMIT": other["commit"]},
            "lom binary SHA-256 is not the expected one": {"SOPHIA_LOM_ARTIFACT": str(tamper)},
            "lom manifest binary SHA-256 is not the expected one": {"SOPHIA_LOM_ARTIFACT": str(swapped)},
            "lom manifest configuration is not the expected one": {"SOPHIA_LOM_ARTIFACT": str(config)},
            "product": {"SOPHIA_HAGIA_ARTIFACT": str(self.artifacts / "lom"),
                        "SOPHIA_HAGIA_COMMIT": self.lom["commit"]},
            "absolute directory": {"SOPHIA_LOM_ARTIFACT": "artifacts/lom"},
            "64 lowercase hex": {"SOPHIA_LOM_SHA256": "LOM"},
        }
        for message, env in cases.items():
            with self.subTest(message=message):
                self.assert_refused_before_build(message, **env)

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
