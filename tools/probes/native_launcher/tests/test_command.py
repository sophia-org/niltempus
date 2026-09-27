# Provenance: moved from Sophia tools/probes/native_launcher/tests/test_command.py at 9fcaec782ce4fe9978568c0466ee17a78b3d4571 (Sophia rule 13).
"""Supplied builds/VT/native effects; real profile generator, artifact intake and transcript verifier."""
from pathlib import Path
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve()
ROOT = HERE.parents[4]
sys.path.insert(0, str(ROOT / 'tools/probes/lom_workload/tests'))
import test_launcher as panel_tests
from test_native_verify import transcript


class ComponentCommandTests(panel_tests.LauncherTests):
    def setUp(self):
        super().setUp()
        shutil.copytree(ROOT / 'tools/probes/native_launcher', self.tools / 'probes/native_launcher',
                        ignore=shutil.ignore_patterns('__pycache__'))
        self.script(self.sophia / 'target/release/examples/desktop_profile_probe', """
[[ "$#" == 2 || ( "$#" == 3 && "$3" == --require-launcher-binding ) ]]
cat "$1"
tail -n +2 "$2"
""")
        self.bemenu_commit = panel_tests.write_artifact(
            self.artifacts / 'bemenu', 'bemenu', self.root / 'pins/c-desktop-sdk/manifest.json')
        self.env['SOPHIA_BEMENU_ARTIFACT'] = str(self.artifacts / 'bemenu')
        self.env['SOPHIA_BEMENU_COMMIT'] = self.bemenu_commit
        # Keep the panel path exactly as the inherited controls require.
        prior = (self.sophia / 'tools/run_sophia_session.sh').read_text().splitlines()[2:]
        self.script(self.sophia / 'tools/run_sophia_session.sh', '''
if [[ "$#" == 2 ]]; then
    [[ "$1" == --max-runtime-ms=90000 && "$2" == --wm-process="$SOPHIA_HAGIA_BIN" ]]
    [[ "$SOPHIA_SESSION_STARTUP" == none ]]
    [[ "$SOPHIA_REQUIRE_LOCAL_VT" == true && "$SOPHIA_MANAGE_KEYD" == true ]]
    echo component-session >> "$TEST_TRACE"
    mkdir -p "$SOPHIA_DIAGNOSTIC_DIR"
    cp "$TEST_COMPONENT_HOST" "$SOPHIA_DIAGNOSTIC_DIR/events.0.log"
    printf '%s\\n' 'sophia_tty_recovery schema=3 termios_restored=true done=true' \\
      'sophia_tty_recovery_verification schema=1 keyd_restored=true' > "$SOPHIA_DIAGNOSTIC_DIR/recovery.log"
    if [[ "${TEST_CHANGE_BEMENU:-no}" == yes ]]; then
        echo changed >> "$SOPHIA_LOM_NATIVE_EVIDENCE_DIR/bemenu-sophia"
    fi
    exit "${TEST_SESSION_STATUS:-0}"
fi
''' + '\n'.join(prior))
        self.pin_shared()
        self.component_host = self.base / 'component-host.log'
        self.component_host.write_text('\n'.join(transcript()) + '\n')
        self.env['TEST_COMPONENT_HOST'] = str(self.component_host)

    def run_components(self, **env):
        return subprocess.run(['bash', str(self.tools / 'run.sh'), 'launcher'],
                              env={**self.env, **env}, capture_output=True, text=True, timeout=10)

    def test_component_mode_uses_roles_and_real_transcript_verifier(self):
        result = self.run_components()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((self.evidence / 'bemenu-sophia').read_bytes(),
                         (self.artifacts / 'bemenu/bemenu-sophia').read_bytes())
        self.assertIn('component-session', (self.base / 'trace').read_text())
        profile = (self.evidence / 'desktop.kdl').read_text()
        self.assertIn('shell-component "panel" "bar"', profile)
        self.assertIn('shell-component "menu" "application-launcher"', profile)
        self.assertIn('bind "Super+4"', profile)
        self.assertIn(f'bemenu_commit={self.bemenu_commit}', (self.evidence / 'identity.manifest').read_text())
        self.assertIn(str(self.evidence / 'bemenu-sophia'), (self.evidence / 'inputs.sha256').read_text())
        self.assertFalse((self.evidence / 'workload-verification.json').exists())
        self.assertIn('"status": "pass"', (self.evidence / 'launcher-verification.json').read_text())

    def test_component_sdk_pin_mismatch_refuses_before_build(self):
        with (self.root / 'pins/c-desktop-sdk/manifest.json').open('a') as manifest:
            manifest.write(' ')
        result = self.run_components()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('SDK manifest differs', result.stderr)
        self.assertFalse((self.base / 'trace').exists())

    def test_component_binary_change_refuses(self):
        self.assertNotEqual(self.run_components(TEST_CHANGE_BEMENU='yes').returncode, 0)
        self.assertFalse((self.evidence / 'launcher-verification.json').exists())

    def test_component_missing_presentation_refuses(self):
        self.component_host.write_text('\n'.join(v for v in transcript() if not ('status=presented' in v and 'connection_epoch=2' in v)))
        self.assertNotEqual(self.run_components().returncode, 0)
        self.assertIn('"status": "fail"', (self.evidence / 'launcher-verification.json').read_text())

    def test_component_failed_preflight_never_starts(self):
        self.assertNotEqual(self.run_components(TEST_PROOF_STATUS='1').returncode, 0)
        self.assertNotIn('component-session', (self.base / 'trace').read_text())
