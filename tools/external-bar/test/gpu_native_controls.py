"""Real child/process plumbing with synthetic diagnostic records; no display/GPU."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


w = load('native_controls_wrapper', ROOT/'gpu/run_colour_external.py')
h = load('native_controls_handoff', ROOT/'gpu/handoff.py')
fixtures = load('native_controls_fixtures', ROOT/'test/gpu_receipts.py')


def pin(path):
    return dict(executable=str(path), sha256=hashlib.sha256(path.read_bytes()).hexdigest())


class NativeControls(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.python = pin(Path(sys.executable).resolve())
        self.recipe = dict(stack='niri', controls=dict(delay=self.python, invalid=self.python))

    def run_control(self, kind, *, bad='', guard=lambda: None, stack='niri'):
        transport = 'wayland' if stack == 'niri' else 'x11'
        text = (fixtures.fixture(transport)
                .replace('vello_shade_mod128_opaque_rgb', 'diagnostic_release_delay_100ms')
                .replace('frames=2 warmup=1', 'frames=1 warmup=0')
                .replace('rendered=3 released=3', 'rendered=1 released=1')
                .replace('ok=3 violations=0', 'ok=1 violations=0')
                .replace(f'2\t40\t50\t60\t30\t{w.journal.TRANSPORTS[transport][2]}\n', ''))
        if kind == 'delay':
            trace = ''.join(f'gpu_release_delay schema=1 phase={phase} token=7 monotonic_ns={t}\n'
                            for phase,t in zip(('held','resume','released'), (1000000000,1100000000,1100000001)))
        else:
            error = 'code=7 major=0 minor=0' if stack == 'niri' else 'code=2 major=145 minor=7'
            trace = ('gpu_invalid_import schema=1 status=injected generation=1 token=7 bytes=184320\n'
                     f'gpu_invalid_import_refused schema=1 transport={transport} token=7 {error} object=73 released=0\n')
        script = ('import os,sys,time; from pathlib import Path; time.sleep(.2); '
                  f'Path({str(self.root/kind/"journal.tsv")!r}).write_text({text!r}.replace("pid 1234",f"pid {{os.getpid()}}")); '
                  f'sys.stderr.write({trace!r}); sys.stderr.flush(); '
                  f'raise SystemExit({0 if kind=="delay" else 42})')
        if bad == 'status': script = script.replace('raise SystemExit(', 'raise SystemExit(1 or ')
        if bad == 'stdout': script = 'print("unexpected",flush=True); '+script
        if bad == 'pid': script = script.replace('f"pid {os.getpid()}"', '"pid 999999"')
        if bad == 'trace': script = script.replace('token=7', 'token=0')
        if bad == 'flood': script = 'import os,time; os.write(2,b"e"*1100000); time.sleep(30)'
        if bad == 'hang': script = 'import time; time.sleep(30)'
        original = w.spawn
        def synthetic(pin, argv, **kwargs):
            return original(pin, [pin['executable'],'-I','-B','-c',script], **kwargs)
        with patch.object(w,'spawn',side_effect=synthetic):
            return w.run_control(dict(self.recipe,stack=stack),kind,self.root/kind,{},guard,
                                 lambda label: dict(output='DP-1',width=1920,height=1080))

    def test_both_controls_bind_real_children_and_saved_proof(self):
        rows = [self.run_control(kind) for kind in ('delay','invalid')]
        result = dict(client=dict(pid=os.getpid()),server=dict(pid=os.getppid()),controls=rows)
        paths = h.qualify_external_controls(self.recipe,result,self.root)
        self.assertEqual(len(paths),11)
        for row in rows:
            self.assertFalse(json.loads((self.root/row['kind']/'cleanup.json').read_text())['forced'])
        for change in ('missing','order','pid','pin','trace','cleanup'):
            saved = self.root/'invalid/stderr'
            original = saved.read_bytes()
            recipe = copy.deepcopy(self.recipe); altered = copy.deepcopy(result)
            cleanup = self.root/'invalid/cleanup.json'; original_cleanup = cleanup.read_bytes()
            try:
                if change == 'missing': altered['controls'].pop()
                if change == 'order': altered['controls'].reverse()
                if change == 'pid': altered['controls'][1]['client']['pid']=rows[0]['client']['pid']
                if change == 'pin': recipe['controls']['invalid']['sha256']='0'*64
                if change == 'trace': saved.write_bytes(original.replace(b'released=0',b'released=1'))
                if change == 'cleanup': cleanup.write_text(json.dumps(dict(pid=rows[1]['client']['pid'],rc=42,forced=True)))
                with self.subTest(change=change), self.assertRaises(ValueError):
                    h.qualify_external_controls(recipe,altered,self.root)
            finally:
                saved.write_bytes(original); cleanup.write_bytes(original_cleanup)

    def test_x_control_arguments_and_receipts(self):
        for kind in ('delay','invalid'):
            result = self.run_control(kind,stack='xlibre')
            self.assertEqual(result['argv'][1],'--journal')
            self.assertEqual(result['proof']['transport'],'x11')

    def test_wrong_exit_stdout_identity_or_trace_refuses(self):
        for bad in ('status','stdout','pid','trace'):
            with self.subTest(bad=bad):
                old = self.root; self.root=old/bad; self.root.mkdir()
                try:
                    with self.assertRaises(ValueError): self.run_control('delay',bad=bad)
                    self.assertFalse((self.root/'delay/result.json').exists())
                    self.assertTrue((self.root/'delay/cleanup.json').exists())
                finally: self.root=old

    def test_guard_failure_and_output_limit_reap_owned_child(self):
        for bad in ('hang','flood'):
            old=self.root; self.root=old/bad; self.root.mkdir()
            calls=0
            def guard():
                nonlocal calls
                calls+=1
                if bad=='hang' and calls>3: raise ValueError('server changed')
            try:
                with self.assertRaises(ValueError): self.run_control('delay',bad=bad,guard=guard)
                cleanup=json.loads((self.root/'delay/cleanup.json').read_text())
                self.assertEqual(cleanup['rc'],-signal.SIGTERM)
                self.assertFalse((self.root/'delay/result.json').exists())
            finally: self.root=old

    def test_control_pins_are_exact_and_distinct(self):
        pins={key:self.python for key in ('server','client','capture','output','bridge')}
        pins['config']=pin(ROOT/'gpu/niri.kdl')
        recipe=dict(schema=1,stack='niri',display=None,pins=pins)
        w.validate(recipe)
        controls={}
        for kind in ('delay','invalid'):
            path=self.root/kind; path.write_text(kind); path.chmod(0o600); controls[kind]=pin(path)
        w.validate(dict(recipe,controls=controls))
        for change in ({},dict(delay=controls['delay']),dict(delay=self.python,invalid=controls['invalid']),
                       dict(delay=controls['delay'],invalid=controls['delay'])):
            with self.assertRaises(ValueError): w.validate(dict(recipe,controls=change))


if __name__=='__main__':
    unittest.main()
