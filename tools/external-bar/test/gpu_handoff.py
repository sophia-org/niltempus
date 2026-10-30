"""Device-free handoff staging/receipt controls; never execute PAM or a VT."""

import ast
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import signal
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('handoff', ROOT/'gpu/handoff.py')
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)


class Handoff(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.base, self.run = self.root/'ssh', self.root/'run'
        self.seed = h.seed_files()
        self.original = {p: h.sha(h.SEED/p) for p in h.SEED_HASHES}
        self.artifact = self.root/'artifact'
        self.artifact.write_bytes(b'fixture executable')
        self.pin = dict(executable=str(self.artifact), sha256=h.sha(self.artifact))
        self.recipe = dict(schema=1, stack='niri', display=None,
                           pins={k: self.pin for k in ('server', 'client', 'capture', 'output', 'config', 'bridge')})

    def tearDown(self):
        self.assertEqual(self.original, {p: h.sha(h.SEED/p) for p in h.SEED_HASHES})

    def stage(self, **kwargs):
        # Only host closure and recipe validation are stand-ins; rendered scripts
        # are the exact pinned CPU02 templates and are never launched.
        inputs = {}
        for name in ('control', 'tty', 'python'):
            p = self.root/name
            p.write_bytes(name.encode())
            inputs[name] = p
        seed = dict(self.seed)
        seed['ssh-pins.json'] = json.dumps({str(p): h.sha(p) for p in inputs.values()})
        with patch.object(h, 'seed_files', return_value=seed), patch.object(h.w, 'validate'), \
             patch.object(h, 'CONTROL', inputs['control']), patch.object(h, 'TTY', inputs['tty']), \
             patch.object(h, 'PYTHON', str(inputs['python'])):
            return h.stage(kwargs.get('recipe', self.recipe), kwargs.get('destination', self.base),
                           kwargs.get('run', self.run))

    def seal(self):
        (self.base/'MANIFEST.sha256').write_text(''.join(
            h.sha(self.base/n)+'  '+n+'\n' for n in sorted(h.SEALED_FILES)))

    def gate(self):
        # Execute only the complete-seal/pin gate, before CONTROL import or any
        # function that could query seats, touch getty or launch openvt.
        prefix = (self.base/'ssh-run.py').read_text().split('CONTROL = Path(')[0]
        exec(compile(prefix, '<handoff-preimport-gate>', 'exec'), {'__file__': str(self.base/'ssh-run.py')})

    def test_stage_preserves_pam_and_only_renders_bounded_diagnostic_command(self):
        self.stage()
        self.assertEqual((self.base/'vt_session.py').read_text(), self.seed['vt_session.py'])
        self.assertFalse(self.run.exists())
        self.assertFalse((self.base/'MANIFEST.sha256').exists())
        self.assertEqual(set(p.name for p in self.base.iterdir()), h.SEALED_FILES)
        self.assertTrue(all(p.stat().st_mode & 0o777 == 0o600 for p in self.base.iterdir()))
        code = (self.base/'ssh-run.py').read_text()
        compile(code, 'ssh-run.py', 'exec')
        self.assertIn('SOPHIA_SCREEN_BOUND=360', code)
        self.assertIn('time.monotonic()+420', code)
        self.assertLess(code.index("attempt.json').open('x')"), code.index("'down', service"))
        self.assertLess(code.index('Pinned handoff input changed'), code.index('spec.loader.exec_module(control)'))
        command = json.loads((self.base/'native-command.json').read_text())
        self.assertEqual(command[1:4], ['-I', '-B', str(ROOT/'gpu/run_colour_external.py')])
        self.assertEqual(command[4:], ['--recipe', str(self.base/'recipe.json'), '--evidence', str(self.run)])
        self.assertNotIn('cpuwin', (self.base/'run.sh').read_text())

    def test_seal_refuses_absent_empty_missing_duplicate_or_changed_inputs(self):
        self.stage()
        with self.assertRaises(FileNotFoundError): self.gate()
        for content in ('', '0'*64+'  recipe.json\n', ('0'*64+'  recipe.json\n')*len(h.SEALED_FILES)):
            (self.base/'MANIFEST.sha256').write_text(content)
            with self.assertRaisesRegex(RuntimeError, 'Incomplete'): self.gate()
        self.seal()
        self.gate()
        with (self.base/'recipe.json').open('a') as f: f.write(' ')
        with self.assertRaises(subprocess.CalledProcessError): self.gate()

    def test_external_pin_drift_refuses_before_privileged_helper_import(self):
        self.stage(); self.seal()
        self.artifact.write_bytes(b'changed')
        with self.assertRaisesRegex(RuntimeError, 'Pinned handoff'): self.gate()

    def test_staging_is_fresh_short_and_disjoint(self):
        self.stage()
        with self.assertRaisesRegex(ValueError, 'fresh'): self.stage()
        with self.assertRaisesRegex(ValueError, 'separate'):
            self.stage(destination=self.root/'nested', run=self.root/'nested'/'run')
        with self.assertRaisesRegex(ValueError, 'short'):
            self.stage(destination=self.root/'another', run=self.root/('x'*90))

    def test_inherited_hash_drift_and_template_drift_refuse(self):
        bad = dict(self.seed)
        bad['ssh-pins.json'] = json.dumps({str(h.CONTROL): '0'*64, str(h.TTY): '0'*64, h.PYTHON: '0'*64})
        with patch.object(h.w, 'validate'), patch.object(h, 'seed_files', return_value=bad):
            with self.assertRaisesRegex(ValueError, 'drift'): h.stage(self.recipe, self.base, self.run)
        self.assertFalse(self.base.exists())
        with self.assertRaisesRegex(ValueError, 'template changed'): h.replace_once('x x', 'x', 'y')
        with patch.dict(h.SEED_HASHES, {'ssh-run.py': '0'*64}):
            with self.assertRaisesRegex(ValueError, 'seed hash'): h.seed_files()

    def test_consumed_attempt_refuses_check_before_seat_queries(self):
        self.stage(); (self.base/'attempt.json').write_text('{}')
        tree = ast.parse((self.base/'ssh-run.py').read_text())
        verify = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == 'verify')
        scope = dict(BASE=self.base, json=json, hashlib=hashlib, Path=Path)
        exec(compile(ast.Module(body=[verify], type_ignores=[]), '<verify>', 'exec'), scope)
        with self.assertRaisesRegex(RuntimeError, 'already consumed'): scope['verify']()

    def receipts(self):
        self.base.mkdir(); self.run.mkdir()
        values = {
            self.base/'recipe.json': self.recipe,
            self.base/'vt-session-result.json': dict(stage='pam_cleanup_done', timed_out=False,
                child_status=0, residual_pids=[], cleanup=dict(close=0, credentials=0, pam_end=0, tty_restored=True)),
            self.base/'remote-user-result.json': dict(exit_status=0),
            self.base/'outer-cleanup.json': dict(final_occupants=[], final_sessions=[], killed=[], killed_sessions=[]),
            self.run/'result.json': dict(status='colour_source_path_completed', stack='niri',
                client=dict(pid=42), server=dict(pid=41)),
            self.run/'cleanup.json': dict(children=[dict(pid=42, rc=0, forced=False, unexpected_server_exit=False),
                dict(pid=41, rc=-signal.SIGTERM, forced=False, unexpected_server_exit=False)]),
        }
        for path, value in values.items(): path.write_text(json.dumps(value))
        (self.base/'run-path.txt').write_text(str(self.run)+'\n')
        return values

    def test_cleanup_accepts_only_complete_outer_and_matching_local_receipts(self):
        self.receipts()
        result = h.qualify_cleanup(self.base)
        self.assertEqual(result['status'], 'handoff_cleanup_verified')
        self.assertEqual(len(result['input_sha256']), 7)
        (self.run/'refused.json').write_text('{}')
        with self.assertRaisesRegex(ValueError, 'wrapper refused'): h.qualify_cleanup(self.base)

    def test_cleanup_refuses_missing_failed_typed_and_foreign_evidence(self):
        values = self.receipts()
        cases = []
        for key, bad in [('stage', 'session_open'), ('timed_out', True), ('timed_out', 0),
                         ('child_status', False), ('residual_pids', [88])]:
            cases.append((self.base/'vt-session-result.json', lambda v,k=key,b=bad: v.update({k:b})))
        for key, bad in [('close', 1), ('credentials', False), ('pam_end', None), ('tty_restored', 1)]:
            cases.append((self.base/'vt-session-result.json', lambda v,k=key,b=bad: v['cleanup'].update({k:b})))
        cases.append((self.base/'remote-user-result.json', lambda v: v.update(exit_status=False)))
        for key in ('final_occupants','final_sessions','killed','killed_sessions'):
            cases.append((self.base/'outer-cleanup.json', lambda v,k=key: v.update({k:[42]})))
        cases.append((self.run/'result.json', lambda v: v.update(stack='xlibre')))
        cases.append((self.run/'result.json', lambda v: v['server'].update(pid=True)))
        cases.append((self.run/'result.json', lambda v: v['server'].update(pid=42)))
        for key, bad in [('pid', 99), ('rc', 1), ('rc', False), ('forced', True), ('unexpected_server_exit', True)]:
            cases.append((self.run/'cleanup.json', lambda v,k=key,b=bad: v['children'][1].update({k:b})))
        for path, mutate in cases:
            with self.subTest(path=path, mutate=mutate):
                value = copy.deepcopy(values[path]); mutate(value); path.write_text(json.dumps(value))
                with self.assertRaises(ValueError): h.qualify_cleanup(self.base)
                path.write_text(json.dumps(values[path]))
        for path in values:
            data=path.read_bytes(); path.unlink()
            with self.assertRaises(FileNotFoundError): h.qualify_cleanup(self.base)
            path.write_bytes(data)


if __name__ == '__main__':
    unittest.main()
