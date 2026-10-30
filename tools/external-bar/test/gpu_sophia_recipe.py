"""Fixed Sophia recipe, parser invocation and handoff joins without native launch."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


s = load('sophia_recipe_test', ROOT/'gpu/sophia_recipe.py')
h = load('handoff_test', ROOT/'gpu/handoff.py')
hf = load('handoff_fixture', ROOT/'test/gpu_handoff.py')
ef = load('environment_fixture', ROOT/'test/gpu_sophia_environment.py')


class SophiaRecipe(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.artifact = self.root/'artifact'
        self.artifact.write_bytes(b'not executed')
        self.pin = dict(executable=str(self.artifact), sha256=hashlib.sha256(self.artifact.read_bytes()).hexdigest())
        artifacts = {'host':dict(self.pin)}
        for key in s.ARTIFACTS-{'host'}:
            path=self.root/key;path.write_bytes(key.encode())
            artifacts[key]=dict(executable=str(path),sha256=hashlib.sha256(path.read_bytes()).hexdigest())
        self.inputs = dict(schema=1, artifacts=artifacts, output_generation=1)
        self.directory = self.root/'config'
        self.recipe = s.prepare(self.inputs, self.directory)

    def test_fixed_recipe_is_one_dma_bar_and_one_capture_observer(self):
        core = (self.directory/'core.kdl').read_text()
        desktop = (self.directory/'desktop.kdl').read_text()
        self.assertEqual(core.count('recipe '), 1)
        self.assertIn('profile="observer" developer-launch=#true', core)
        self.assertIn('capture-output "pci-0000:03:00.0/DP-1"', core)
        self.assertEqual(desktop.count('shell-component '), 1)
        self.assertIn('control "host-admin"', desktop)
        self.assertIn('gpu "direct"', desktop)
        self.assertIn('exclude-gpu "pci-0000:16:00.0"', desktop)
        peer = json.loads((self.directory/'peer.json').read_text())
        self.assertEqual((peer['frames'],peer['height'],peer['hold_msec'],peer['source'],peer['producer']),
                         (3,24,0,'dma_buf','gpu'))
        self.assertNotIn('dma_reuse', peer)
        self.assertNotIn('retire_on_present', peer)
        self.assertFalse(peer['capture_profile'])
        self.assertEqual(self.recipe['output_generation'], 1)
        self.assertIn('--max-runtime-ms=280000', self.recipe['argv'])
        self.assertFalse(any('retire-on-present' in a or 'diagnostic-capture-directory' in a for a in self.recipe['argv']))

    def test_missing_extra_wrong_type_and_artifact_drift_refuse(self):
        bad = []
        for key in s.ARTIFACTS:
            v=copy.deepcopy(self.inputs); del v['artifacts'][key]; bad.append(v)
        for key,value in [('schema',True),('schema',1.0),('output_generation',True),('output_generation',0)]:
            v=copy.deepcopy(self.inputs); v[key]=value; bad.append(v)
        v=copy.deepcopy(self.inputs); v['artifacts']['extra']=self.pin; bad.append(v)
        for v in bad:
            with self.assertRaises((ValueError, OSError)): s.validate(v)
        for pin in self.inputs['artifacts'].values():
            path=Path(pin['executable']);original=path.read_bytes();path.write_bytes(b'changed')
            with self.assertRaises(ValueError): s.verify(self.directory)
            path.write_bytes(original)

    def test_changed_configuration_and_symlink_refuse(self):
        for name in ('core.kdl','desktop.kdl','peer.json','recipe.json'):
            path=self.directory/name; original=path.read_bytes()
            path.write_bytes(original+b' ')
            with self.assertRaisesRegex(ValueError,'input changed'): s.verify(self.directory)
            path.unlink(); path.symlink_to(self.artifact)
            with self.assertRaises(OSError): s.verify(self.directory)
            path.unlink(); path.write_bytes(original)
        with self.assertRaises(FileExistsError): s.prepare(self.inputs,self.directory)

    def test_preflight_is_exact_validate_only_and_has_no_capture_directory(self):
        evidence=self.root/'preflight'
        def command(argv,pin,env,*args,**kwargs):
            self.assertEqual(argv[-1], '--validate-session-args')
            self.assertEqual(argv[-2], f'--diagnostic-capture-directory={evidence/"uncreated-captures"}')
            self.assertEqual(pin, self.pin)
            self.assertEqual([k for k in env if k.startswith('SOPHIA_')], ['SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE', 'SOPHIA_OUTPUT_ENVIRONMENT_ARM', 'SOPHIA_OUTPUT_ENVIRONMENT_SOCKET'])
            return b'sophia_live_session_args schema=1 status=accepted arguments=14\n'
        with patch.object(s.external,'command',side_effect=command): s.preflight(self.directory,evidence)
        self.assertFalse((evidence/'uncreated-captures').exists())
        self.assertEqual(json.loads((evidence/'result.json').read_text())['status'],'arguments_accepted')

    def test_preflight_refuses_partial_extra_or_wrong_count_and_side_effect(self):
        for i,value in enumerate((b'',b'accepted\n',b'sophia_live_session_args schema=1 status=accepted arguments=12\n',
            b'sophia_live_session_args schema=1 status=accepted arguments=14\nextra\n')):
            with patch.object(s.external,'command',return_value=value):
                with self.assertRaises(ValueError): s.preflight(self.directory,self.root/f'bad-{i}')
        evidence=self.root/'side-effect'
        def created(*_a,**_kw):
            (evidence/'uncreated-captures').mkdir()
            return b'sophia_live_session_args schema=1 status=accepted arguments=14\n'
        with patch.object(s.external,'command',side_effect=created):
            with self.assertRaises(ValueError): s.preflight(self.directory,evidence)

    def test_run_requires_attended_unprivileged_context_before_wrapper(self):
        with patch.object(s,'verify',return_value=self.recipe), patch.object(s.wrapper,'run') as run:
            with patch.object(s.os,'getuid',return_value=0):
                with self.assertRaises(ValueError): s.run(self.directory,self.root/'run')
            with patch.object(s.os,'ttyname',return_value='/dev/pts/1'):
                with self.assertRaises(ValueError): s.run(self.directory,self.root/'run')
            run.assert_not_called()

    def test_run_cleans_environment_and_preserves_signal_cleanup(self):
        before={sig:signal.getsignal(sig) for sig in (signal.SIGTERM,signal.SIGINT)}
        def interrupted(*args):
            self.assertNotIn('DRI_PRIME',os.environ)
            self.assertNotIn('LD_PRELOAD',os.environ)
            self.assertNotIn('SOPHIA_UNTRUSTED_SESSION_OUTPUT_LOG',os.environ)
            self.assertEqual(os.environ['SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE'],'1')
            signal.getsignal(signal.SIGTERM)(signal.SIGTERM,None)
        with patch.dict(os.environ,{'DRI_PRIME':'1','LD_PRELOAD':'fake','SOPHIA_UNTRUSTED_SESSION_OUTPUT_LOG':'bad'}), \
             patch.object(s.os,'ttyname',return_value='/dev/tty4'), patch.object(s.wrapper,'run',side_effect=interrupted):
            saved=dict(os.environ)
            with self.assertRaises(InterruptedError): s.run(self.directory,self.root/'run')
            self.assertEqual(dict(os.environ),saved)
        self.assertEqual({sig:signal.getsignal(sig) for sig in before},before)

    def test_sophia_stage_pins_configuration_and_only_runs_explicit_entry(self):
        fixture=hf.Handoff(); fixture.setUp()
        try:
            pins={str(h.CONTROL):h.sha(h.CONTROL),str(h.TTY):h.sha(h.TTY),h.PYTHON:h.sha(h.PYTHON)}
            seed=dict(fixture.seed);seed['ssh-pins.json']=json.dumps(pins)
            with patch.object(h,'seed_files',return_value=seed):
                h.stage_sophia(self.directory,fixture.base,fixture.run)
            argv=json.loads((fixture.base/'native-command.json').read_text())
            self.assertEqual(argv[3:], [str(ROOT/'gpu/sophia_recipe.py'),'run','--directory',str(self.directory),'--evidence',str(fixture.run)])
            pinned=json.loads((fixture.base/'ssh-pins.json').read_text())
            for path in self.directory.iterdir():self.assertEqual(pinned[str(path)],h.sha(path))
            self.assertEqual((fixture.base/'vt_session.py').read_text(),fixture.seed['vt_session.py'])
            self.assertFalse((fixture.base/'MANIFEST.sha256').exists())
            self.assertFalse(fixture.run.exists())
        finally:
            fixture.tearDown();fixture.doCleanups()

    def test_sophia_cleanup_requires_terminal_driver_and_exact_exits(self):
        fixture=hf.Handoff();fixture.setUp()
        try:
            fixture.receipts();base,run=fixture.base,fixture.run
            (base/'recipe.json').write_text(json.dumps(dict(schema=1,stack='sophia',directory=str(self.directory),wrapper=self.recipe)))
            result=dict(status='driver_completed',host_exit=0,driver_exit=0,
                        host_argv=[self.pin['executable'],*self.recipe['argv'],f'--diagnostic-capture-directory={run/"captures"}'])
            local=dict(driver_forced=False,host_forced=False,driver_exit=0,host_exit=0)
            (run/'driver').mkdir()
            values={run/'result.json':result,run/'cleanup.json':local,
                    run/'driver/result.json':dict(schema=1,status='session_exited_after_captures'),
                    run/'driver/captures_verified.json':dict(schema=1,status='captures_verified',records=[{}, {}, {}])}
            raw=ef.data()
            environment=dict(schema=1,status='startup_environment_verified',receiver_exit=0,
                host=dict(self.pin,pid=123,starttime=1),
                receiver=dict(self.recipe['environment_receiver'],pid=124,starttime=2),
                start_usec=100,end_usec=200,input_sha256=hashlib.sha256(raw).hexdigest(),
                **ef.e.qualify(raw,dict(pid=123),100,200))
            values[run/'environment.json']=environment
            (run/'environment.ndjson').write_bytes(raw)
            (run/'environment.stderr').write_bytes(b'')
            result['environment_sha256']=hashlib.sha256(json.dumps(environment).encode()).hexdigest()
            for path,value in values.items():path.write_text(json.dumps(value))
            self.assertEqual(h.qualify_cleanup(base)['stack'],'sophia')
            cases=[(run/'result.json',lambda v:v.update(host_exit=1)),
                   (run/'result.json',lambda v:v.update(environment_sha256='0'*64)),
                   (run/'environment.json',lambda v:v.update(receiver_exit=1)),
                   (run/'result.json',lambda v:v.update(driver_exit=False)),
                   (run/'result.json',lambda v:v.update(host_argv=[])),
                   (run/'cleanup.json',lambda v:v.update(host_forced=True)),
                   (run/'cleanup.json',lambda v:v.update(host_exit=False)),
                   (run/'driver/result.json',lambda v:v.update(status='running')),
                   (run/'driver/captures_verified.json',lambda v:v.update(records=[{},{}]))]
            for path,change in cases:
                value=copy.deepcopy(values[path]);change(value);path.write_text(json.dumps(value))
                with self.assertRaises(ValueError):h.qualify_cleanup(base)
                path.write_text(json.dumps(values[path]))
            for status in (1,False):
                for path in (run/'result.json',run/'cleanup.json'):
                    value=copy.deepcopy(values[path]);value['host_exit']=status;path.write_text(json.dumps(value))
                with self.assertRaises(ValueError):h.qualify_cleanup(base)
                for path in (run/'result.json',run/'cleanup.json'):path.write_text(json.dumps(values[path]))
            (run/'driver/refused.json').write_text('{}')
            with self.assertRaises(ValueError):h.qualify_cleanup(base)
        finally:
            fixture.tearDown();fixture.doCleanups()


if __name__ == '__main__':
    unittest.main()
