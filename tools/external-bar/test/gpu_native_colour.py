"""Device-free native-wrapper controls; no compositor, GPU or VT is opened."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


w = load('native_colour', ROOT/'gpu/run_colour_external.py')
receipts = load('receipts_fixture', ROOT/'test/gpu_receipts.py')


def pin(path):
    path = Path(path).resolve()
    return dict(executable=str(path), sha256=hashlib.sha256(path.read_bytes()).hexdigest())


def niri_output():
    return {'DP-1': {'current_mode': 0, 'modes': [dict(width=1920, height=1080, refresh_rate=119880)],
                     'vrr_enabled': False, 'logical': dict(x=0, y=0, width=1920, height=1080, scale=1.0, transform='Normal')},
            'DP-2': {'current_mode': None}}


X_OUTPUT = '''Screen 0: minimum 320 x 200, current 1920 x 1080, maximum 16384 x 16384
DP-1 connected primary 1920x1080+0+0 (0x54) normal (normal left inverted right x axis y axis) 697mm x 392mm
    Transform:  1.000000 0.000000 0.000000
                0.000000 1.000000 0.000000
                0.000000 0.000000 1.000000
  1920x1080 (0x54) 296.703MHz +HSync +VSync *current
        h: width 1920 start 2008 end 2052 total 2200 skew 0 clock 134.87KHz
        v: height 1080 start 1084 end 1089 total 1125 clock 119.88Hz
DP-2 disconnected (normal left inverted right x axis y axis)
'''


class NativeColour(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.python = pin(sys.executable)

    def command(self, script, *, seconds=2, limit=1024, guard=lambda: None):
        return w.command([self.python['executable'], '-I', '-B', '-c', script], self.python, {},
                         self.root/'command', seconds, guard, limit)

    def test_real_helper_is_executed_and_reaped_with_private_receipt(self):
        self.assertEqual(self.command("print('capture')"), b'capture\n')
        receipt = json.loads((self.root/'command.json').read_text())
        self.assertEqual(receipt['rc'], 0)
        self.assertFalse(receipt['forced'])
        self.assertEqual((self.root/'command.stderr').read_bytes(), b'')

    def test_helpers_refuse_stderr_and_nonzero(self):
        for script in ("import sys; print('bad',file=sys.stderr)", 'raise SystemExit(7)'):
            with self.subTest(script=script):
                for p in self.root.iterdir(): p.unlink()
                with self.assertRaises(ValueError): self.command(script)
                self.assertTrue((self.root/'command.json').exists())

    def test_both_streams_are_drained_without_deadlock_and_bounded(self):
        with self.assertRaisesRegex(ValueError, 'output limit'):
            self.command("import os; os.write(2,b'e'*70000); os.write(1,b'o'*70000)", limit=100)
        self.assertTrue(json.loads((self.root/'command.json').read_text())['forced'])

    def test_timeout_kills_child_and_guard_failure_is_not_ignored(self):
        start = time.monotonic()
        with self.assertRaisesRegex(ValueError, 'deadline'):
            self.command('import time; time.sleep(30)', seconds=.05)
        self.assertLess(time.monotonic()-start, 3)
        receipt = json.loads((self.root/'command.json').read_text())
        self.assertEqual(receipt['rc'], -signal.SIGKILL)
        for p in self.root.iterdir(): p.unlink()
        def refused(): raise ValueError('identity lost')
        with self.assertRaisesRegex(ValueError, 'identity lost'):
            self.command('import time; time.sleep(30)', guard=refused)

    def test_pin_mismatch_refuses_before_spawn(self):
        wrong = dict(self.python, sha256='0'*64)
        with self.assertRaisesRegex(ValueError, 'changed executable'):
            w.spawn(wrong, [wrong['executable'], '-c', 'raise SystemExit(0)'])
        with self.assertRaisesRegex(ValueError, 'argv differs'):
            w.spawn(self.python, ['/bin/false'])

    def test_regular_bounded_reads_refuse_symlink_and_fifo(self):
        p = self.root/'image'; p.write_bytes(b'abc')
        self.assertEqual(w.read(p, 3), b'abc')
        with self.assertRaises(ValueError): w.read(p, 2)
        alias = self.root/'alias'; alias.symlink_to(p)
        with self.assertRaises(OSError): w.read(alias, 3)
        fifo = self.root/'fifo'; os.mkfifo(fifo)
        with self.assertRaises(ValueError): w.read(fifo, 3)

    def test_inotify_requires_completed_target_not_create_or_other_file(self):
        directory = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY)
        notification = w.h.c.watch(directory)
        self.addCleanup(os.close, directory); self.addCleanup(os.close, notification)
        writer = (self.root/'capture.png').open('wb')
        self.addCleanup(writer.close)
        (self.root/'other').write_bytes(b'done')
        with self.assertRaisesRegex(ValueError, 'did not finish'):
            w.completed_file(notification, 'capture.png', time.monotonic()+.02, lambda: None)
        writer.write(b'image'); writer.close()
        w.completed_file(notification, 'capture.png', time.monotonic()+1, lambda: None)

    def test_niri_readback_requires_exact_device_gate_output_shape(self):
        self.assertEqual(w.outputs.niri(json.dumps(niri_output()))['refresh_millihz'], 119880)
        changes = [('vrr_enabled', True), ('current_mode', True), ('current_mode', -1)]
        for key, value in changes:
            data = niri_output(); data['DP-1'][key] = value
            with self.subTest(key=key, value=value), self.assertRaises(ValueError): w.outputs.niri(json.dumps(data))
        for key, value in [('scale', 2), ('scale', True), ('transform', '90'), ('x', 1)]:
            data = niri_output(); data['DP-1']['logical'][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError): w.outputs.niri(json.dumps(data))
        for key, value in [('refresh_rate', 120000), ('width', True), ('height', 720)]:
            data = niri_output(); data['DP-1']['modes'][0][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError): w.outputs.niri(json.dumps(data))
        data = niri_output(); data['DP-2']['current_mode'] = 0
        with self.assertRaises(ValueError): w.outputs.niri(json.dumps(data))
        with self.assertRaises(ValueError): w.outputs.niri('{"DP-1":{},"DP-1":{}}')

    def test_x_readback_requires_exact_mode_transform_and_single_head(self):
        self.assertEqual(w.outputs.xlibre(X_OUTPUT)['refresh_hz'], 119.88)
        mutations = [('119.88Hz', '120.00Hz'), ('primary 1920x1080+0+0', 'primary 1920x1080+1+0'),
                     ('(0x54) normal', '(0x54) left'), ('Transform:  1.000000', 'Transform:  2.000000'),
                     ('DP-2 disconnected', 'DP-2 connected 1920x1080+1920+0'), ('*current', ''),
                     ('current 1920 x 1080', 'current 3840 x 1080')]
        for old, new in mutations:
            with self.subTest(old=old), self.assertRaises(ValueError): w.outputs.xlibre(X_OUTPUT.replace(old, new))

    def test_colour_journal_cannot_enter_timed_parser(self):
        for transport in ('wayland', 'x11'):
            text = receipts.fixture(transport).replace('vello_shade_mod128_opaque_rgb', 'diagnostic_colour_hold_121').replace('frames=2 warmup=1', 'frames=3 warmup=0')
            source = w.journal.TRANSPORTS[transport][2]
            row = f'2\t40\t50\t60\t30\t{source}\n'
            text = text.replace(row, row+f'3\t70\t80\t90\t30\t{source}\n')
            self.assertEqual(w.journal.qualify_colour(text, transport)['client_pid'], 1234)
            with self.assertRaises(ValueError): w.journal.qualify(text, transport, 3, 0)
            with self.assertRaises(ValueError): w.journal.qualify_colour(text.replace('released=3', 'released=2'), transport)

    def test_recipe_fixed_config_and_pin_set(self):
        pins = {name: self.python for name in ('server', 'output', 'client', 'capture', 'bridge')}
        config = self.root/'niri.kdl'
        config.write_bytes((ROOT/'gpu/niri.kdl').read_bytes()); config.chmod(0o600)
        pins['config'] = pin(config)
        recipe = dict(schema=1, stack='niri', pins=pins, display=None)
        w.validate(recipe)
        for change in [dict(schema=True), dict(display=1), dict(stack='gpu'), dict(extra=1)]:
            with self.subTest(change=change), self.assertRaises(ValueError): w.validate(dict(recipe, **change))
        bad = copy.deepcopy(recipe); bad['pins']['config'] = self.python
        with self.assertRaisesRegex(ValueError, 'configuration'): w.validate(bad)
        long_root = self.root/('long-run-'*10)
        with patch.object(w.os, 'ttyname', return_value='/dev/tty4'), self.assertRaisesRegex(ValueError, 'path too long'):
            w.run(recipe, long_root)
        self.assertFalse(long_root.exists())

    def test_environment_drops_render_and_display_overrides(self):
        with patch.dict(os.environ, {'LD_PRELOAD':'foreign','VK_ICD_FILENAMES':'foreign','NIRI_SOCKET':'foreign','DISPLAY':':1'}):
            env = w.environment(self.root)
        for name in ('LD_PRELOAD','VK_ICD_FILENAMES','NIRI_SOCKET','DISPLAY'):
            self.assertNotIn(name, env)

    def test_numbered_drm_nodes_must_match_the_physical_mapping(self):
        minors = dict(card0=0, renderD128=128, card1=1, renderD129=129)
        mappings = {name: ('0000:03:00.0' if minor in (0,128) else '0000:16:00.0') for name,minor in minors.items()}
        class Info:
            st_mode = stat.S_IFCHR | 0o600
            def __init__(self, minor): self.st_rdev = os.makedev(226, minor)
        def node(path): return Info(minors[path.name])
        def physical(path, strict=False): return Path('/sys/devices')/mappings[path.parent.name]
        with patch.object(Path, 'lstat', node), patch.object(Path, 'resolve', physical):
            self.assertEqual(w.drm_layout()['card0']['pci'], '0000:03:00.0')
            mappings['card0'] = '0000:16:00.0'
            with self.assertRaisesRegex(ValueError, 'probe order'): w.drm_layout()
            mappings['card0'] = '0000:03:00.0'; minors['renderD128'] = 129
            with self.assertRaisesRegex(ValueError, 'probe order'): w.drm_layout()

    def test_live_listener_is_owned_not_a_probe_connection(self):
        class Server:
            pid = os.getpid()
            def check(self): pass
        listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.addCleanup(listener.close)
        path = self.root/'wayland-0'; listener.bind(str(path)); listener.listen(1)
        self.assertIn(str(path), w.owned_listeners(Server()))
        self.assertIsNone(w.endpoints(Server(), 'niri', self.root, None))
        ipc = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); self.addCleanup(ipc.close)
        ipc_path = self.root/'niri.test.sock'; ipc.bind(str(ipc_path)); ipc.listen(1)
        self.assertEqual(w.endpoints(Server(), 'niri', self.root, None), dict(WAYLAND_DISPLAY=str(path), NIRI_SOCKET=str(ipc_path)))

    def test_x_filesystem_and_abstract_listener_both_belong_to_server(self):
        class Server:
            pid = 123
        class Info:
            st_mode = stat.S_IFSOCK | 0o600
            st_uid = os.getuid()
        path = '/tmp/.X11-unix/X99'
        names = [path, '@'+path]
        def rows(_pid): return [['','','','','','','',name] for name in names]
        with patch.object(w, 'owned_listeners', return_value=names.copy()) as owned, patch.object(w, 'listener_rows', side_effect=rows), patch.object(w.os, 'lstat', return_value=Info()):
            self.assertEqual(w.endpoints(Server(), 'xlibre', self.root, 99), {'DISPLAY': ':99'})
            owned.return_value = [path]
            with self.assertRaisesRegex(ValueError, 'abstract X'): w.endpoints(Server(), 'xlibre', self.root, 99)
            names.append(path)
            with self.assertRaisesRegex(ValueError, 'replaced'): w.endpoints(Server(), 'xlibre', self.root, 99)

    def test_wrapper_sequence_with_fixture_processes_and_private_captures(self, bad_journal=False, stack='niri', crash_server=False):
        # Real subprocesses, sockets, pidfds, inotify, pipe drains and cleanup;
        # compositor/device behavior and capture pixels are explicit stand-ins.
        pixels = load('pixel_fixture', ROOT/'test/gpu_pixels.py')
        for pattern in (1, 2):
            (self.root/f'pattern-{pattern}.pam').write_bytes(pixels.image(pattern))
        config_name = 'niri.kdl' if stack == 'niri' else 'xorg.conf'
        config = self.root/config_name; config.write_bytes((ROOT/'gpu'/config_name).read_bytes()); config.chmod(0o600)
        pins = {name: self.python for name in ('server', 'output', 'client', 'capture', 'bridge')}
        pins['config'] = pin(config)
        if stack == 'xlibre': pins['xauth'] = self.python
        recipe = dict(schema=1, stack=stack, pins=pins, display=None if stack == 'niri' else 99)
        transport = 'wayland' if stack == 'niri' else 'x11'
        source = w.journal.TRANSPORTS[transport][2]
        text = receipts.fixture(transport).replace('vello_shade_mod128_opaque_rgb', 'diagnostic_colour_hold_121').replace('frames=2 warmup=1', 'frames=3 warmup=0')
        row = f'2\t40\t50\t60\t30\t{source}\n'
        text = text.replace(row, row+f'3\t70\t80\t90\t30\t{source}\n')
        if bad_journal:
            text = text.replace('1\t10\t20\t30\t', '1\t10\t20\t31\t')
        original = w.spawn
        executed = []
        capture_count = 0
        server_pid = None
        def fixture_spawn(pin_value, argv, **kwargs):
            nonlocal capture_count, server_pid
            executed.append(argv)
            if argv[1:2] in (['validate'], ['-f'], ['--output']):
                script = 'pass'
            elif argv[1:2] == [':99']:
                script = 'import signal; signal.pause()'
            elif argv[1:2] == ['--query']:
                script = "print('DP-1 connected primary 1920x1080+0+0\\nDP-2 disconnected')"
            elif argv[1:] == ['--current', '--verbose']:
                script = 'print('+repr(X_OUTPUT)+')'
            elif argv[1:2] == ['--config']:
                script = """import os,socket,time,signal
from pathlib import Path
root=Path(os.environ['XDG_RUNTIME_DIR'])
a=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM); a.bind(str(root/'wayland-0'))
b=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM); b.bind(str(root/'niri.fixture.sock'))
time.sleep(.1) # bind-before-listen, with no later inotify event
a.listen(1); b.listen(1)
signal.pause()
"""
            elif '--hold-fd' in argv:
                fd = int(argv[argv.index('--hold-fd')+1])
                path = argv[1] if stack == 'niri' else argv[2]
                script = f"""import os,socket,json
from pathlib import Path
s=socket.socket(fileno={fd})
for g in (1,2,3):
 nonce=f'{{g:032x}}'
 value=dict(schema=1,status='ready',generation=g,pattern=(1,2,1)[g-1],presented_ns=30*g,nonce=nonce)
 s.sendall((json.dumps(value)+'\\n').encode())
 data=b''
 while not data.endswith(b'\\n'): data+=s.recv(1024)
 assert json.loads(data)==dict(schema=1,status='ack',generation=g,nonce=nonce)
Path({path!r}).write_text({text!r}.replace('# pid 1234',f'# pid {{os.getpid()}}'))
s.sendall(b'{{"schema":1,"status":"done","generation":3}}\\n')
"""
                if crash_server:
                    script += f'os.kill({server_pid}, 9)\n'
            elif argv[1:4] == ['msg', '--json', 'outputs']:
                self.assertEqual(kwargs['env']['RUST_LOG'], 'error')
                script = 'print('+repr(json.dumps(niri_output()))+')'
            elif '--path' in argv:
                self.assertEqual(kwargs['env']['RUST_LOG'], 'error')
                path = argv[argv.index('--path')+1]
                self.assertEqual(argv[-4:], ['--show-pointer','false','--write-to-disk','true'])
                script = f'from pathlib import Path; Path({path!r}).write_bytes(b"fixture PNG")'
            elif len(argv) == 2 and argv[1].endswith('.png'):
                generation = int(Path(argv[1]).stem.rsplit('-',1)[1])
                path = self.root/f'pattern-{(1,2,1)[generation-1]}.pam'
                script = f'import sys; from pathlib import Path; sys.stdout.buffer.write(Path({str(path)!r}).read_bytes())'
            elif len(argv) == 1 and stack == 'xlibre':
                path = self.root/f'pattern-{(1,2,1)[capture_count]}.pam'
                capture_count += 1
                script = f'import sys; from pathlib import Path; sys.stdout.buffer.write(Path({str(path)!r}).read_bytes())'
            else:
                raise AssertionError(argv)
            child = original(pin_value, [argv[0], '-I', '-B', '-c', script], **kwargs)
            if argv[1:2] in (['--config'], [':99']): server_pid = child.pid
            return child
        root = self.root/'run'
        original_endpoints, original_open = w.endpoints, os.open
        def fixture_endpoints(*args):
            return {'DISPLAY': ':99'} if stack == 'xlibre' else original_endpoints(*args)
        def opened(path, *args, **kwargs):
            self.assertNotEqual(str(path), '/tmp/.X11-unix')
            return original_open(path, *args, **kwargs)
        with patch.object(w, 'spawn', side_effect=fixture_spawn), patch.object(w.os, 'ttyname', return_value='/dev/tty4'), patch.object(w, 'device_check', return_value=['fixture-only']), patch.object(w, 'drm_layout', return_value={'fixture': True}), patch.object(w, 'endpoints', side_effect=fixture_endpoints), patch.object(w.os, 'open', side_effect=opened):
            if bad_journal:
                with self.assertRaisesRegex(ValueError, 'presentation differs'):
                    w.run(recipe, root)
                self.assertFalse((root/'result.json').exists())
                self.assertFalse(any(row['forced'] for row in json.loads((root/'cleanup.json').read_text())['children']))
                return
            if crash_server:
                with self.assertRaises(ValueError): w.run(recipe, root)
                self.assertFalse((root/'result.json').exists())
                self.assertTrue((root/'cleanup.json').exists())
                return
            result = w.run(recipe, root)
        self.assertEqual(result['status'], 'colour_source_path_completed')
        self.assertEqual(len(result['sequence']['records']), 3)
        self.assertEqual(result['journal']['client_pid'], result['client']['pid'])
        self.assertFalse(any(row['forced'] for row in json.loads((root/'cleanup.json').read_text())['children']))
        self.assertEqual(len([argv for argv in executed if '--path' in argv]), 3 if stack == 'niri' else 0)
        if stack == 'xlibre':
            self.assertEqual(len([argv for argv in executed if argv[1:] == ['--current','--verbose']]), 6)
        self.assertEqual(len(list(root.glob('output-*-qualified.json'))), 6)

    def test_wrapper_refuses_journal_not_joined_to_ready(self):
        self.test_wrapper_sequence_with_fixture_processes_and_private_captures(bad_journal=True)

    def test_x_wrapper_uses_cached_readback_without_existing_watch_directory(self):
        self.test_wrapper_sequence_with_fixture_processes_and_private_captures(stack='xlibre')

    def test_wrapper_refuses_server_crash_after_client_done(self):
        self.test_wrapper_sequence_with_fixture_processes_and_private_captures(crash_server=True)


if __name__ == '__main__':
    unittest.main()
