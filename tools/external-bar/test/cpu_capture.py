"""Static Lom controller fixtures; no native host, display, or GPU is launched."""
import hashlib
import importlib.util
import json
from pathlib import Path
import time
import threading
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


c = load('cpu_controller', ROOT/'cpu/capture.py')
gpu = load('gpu_fixture', ROOT/'test/gpu_capture.py')


def line(tag, **row):
    return tag+' '+' '.join(f'{k}={v}' for k,v in row.items())+'\n'


def static_log(checksum, arm='direct'):
    text = line('sophia_live_native_head', schema=2,status='ready',output=7,head=9,connector='DP-1',
                connector_id=93,mode='1920x1080',refresh_millihz=60000,mirrored='false')
    text += line('sophia_shell_component',schema=1,status='negotiated',slot=0,role='bar',connection_epoch=1,
                 content_grant_epoch=1,revision=10,gpu_mode='denied',gpu_grant_epoch=0,device_major=0,device_minor=0)
    text += 'lom_shell_transport schema=1 wire=9p2000.L revision=10 epoch=1\n'+c.static.STARTUP[arm]+'\n'
    for i in range(9):
        text += line('sophia_shell_indicator_state',schema=1,connection_epoch=1,indicator_generation=2,
                     output=7,indicator=i+1,action=i+11,slot=i,state_bits=int(i==0),entries=9)
    text += line('sophia_live_shell_content',schema=1,status='outputs',facts_generation=1,outputs=1,
                 connection_epoch=1,content_grant_epoch=1)
    for status in ['prepared','presented']:
        row = dict(schema=1,status=status,output=7,candidate_generation=1,staging_bytes=0,
                   resident_bytes=184320,retiring_bytes=0,backing_bytes=184320)
        if status == 'presented':
            row.update(presentation_epoch=4,connection_epoch=1,content_grant_epoch=1)
        text += line('sophia_live_shell_content',**row)
    text += line('lom_panel_candidate',schema=1,status='presented',connection_epoch=1,content_grant_epoch=1,
                 output=7,candidate_generation=1,presentation_epoch=4,indicator_generation=2,width=1920,
                 height=24,bytes=184320,checksum=checksum,raster_source='rendered')
    common = dict(status='active',requested='exact_nearest',effective='exact_nearest',
                  alpha_mode='premultiplied',source='1920x24',target='1920x24')
    prefix = '2026-10-30T12:00:00.123Z  INFO sophia_renderer_native_egl::gl: '
    text += prefix+line('sophia_native_composition_sampling',schema=3,output=7,head=9,
                        scene_generation=1,frame='1920x1080',**common)
    return text.encode()


class Controller(unittest.TestCase):
    def setUp(self):
        self.f = gpu.Fixture()
        self.addCleanup(self.f.close)
        self.pam = gpu.image(1)
        rgba = memoryview(self.pam)[len(c.pixels.HEADER):len(c.pixels.HEADER)+184320]
        bgra = bytearray(rgba)
        bgra[0::4],bgra[2::4] = rgba[2::4],rgba[0::4]
        self.checksum = c.pixels.fnv64(bgra)
        self.expected = {k:v for k,v in self.f.expected.items() if k != 'hold'}
        self.expected['arm'] = 'direct'
        self.prefix = f'sophia_developer_launch schema=1 status=enabled admission=host-admin socket={self.f.endpoint}\n'.encode()
        self.f.log.write_bytes(self.prefix+static_log(self.checksum))

    def publish(self, change=None, stderr=b'', generation=3):
        receipt = dict(schema=1,status='delivered',binding='launch_custody_not_per_message',
                       launch_generation=generation,transaction=1,connector=c.r.CONNECTOR,namespace=6,
                       peer_pid=100,peer_starttime=900,exe_dev=self.f.session.identity[0],
                       exe_ino=self.f.session.identity[1],request=1,output=7,output_generation=1,
                       presentation_generation=123,width=1920,height=1080,
                       pam_sha256=hashlib.sha256(self.pam).hexdigest())
        receipt.update(change or {})
        gpu.write(self.f.delivery/f'capture-{generation}.stderr',stderr)
        for suffix,data in [('pam',self.pam),('json',json.dumps(receipt).encode())]:
            target = self.f.delivery/f'capture-{generation}.{suffix}'
            partial = self.f.delivery/f'capture-{generation}.{suffix}.partial'
            gpu.write(partial,data)
            self.assertFalse(target.exists())
            partial.rename(target)

    def capture(self, launch, seconds=2):
        with patch.object(c.c,'launch',side_effect=launch) as call:
            try:
                return c.capture(self.expected,self.f.session,self.f.directory,
                                 time.monotonic()+seconds,self.f.evidence)
            finally:
                self.assertEqual(call.call_count,1)
                self.assertEqual(call.call_args.args[0], [self.f.session.executable, 'launch',
                    '--socket', str(self.f.endpoint), '--recipe', 'capture', '--',
                    self.f.session.executable, 'capture', '7', '--diagnostic-stream'])

    def test_one_static_capture_and_separate_generation_domains(self):
        result = self.capture(lambda *args:self.publish())
        self.assertEqual(result['status'],'static_capture_verified')
        self.assertEqual(result['capture']['receipt']['launch_generation'],3)
        self.assertEqual(result['scene']['candidate']['presentation_epoch'],'4')
        self.assertEqual(result['capture']['receipt']['presentation_generation'],123)
        self.assertEqual(result['capture']['pixels']['produced_fnv64'],self.checksum)
        self.assertEqual((self.f.evidence/'before.log').read_bytes(),(self.f.evidence/'after.log').read_bytes())

    def test_global_launch_generation_can_exceed_capture_budget(self):
        result = self.capture(lambda *args:self.publish(generation=41))
        self.assertEqual(result['capture']['receipt']['launch_generation'],41)

    def test_delivery_after_empty_inventory_wakes_through_inotify(self):
        empty = threading.Event()
        actual = c.r.inventory
        inventories = 0
        def inventory(directory):
            nonlocal inventories
            inventories += 1
            groups = actual(directory)
            if inventories > 1 and not groups:
                empty.set()
            return groups
        def publish():
            if empty.wait(2):
                self.publish()
        worker = threading.Thread(target=publish)
        with patch.object(c.r,'inventory',side_effect=inventory):
            try:
                result = self.capture(lambda *args:worker.start())
            finally:
                worker.join(3)
        self.assertFalse(worker.is_alive())
        self.assertTrue(empty.is_set())
        self.assertEqual(result['capture']['receipt']['launch_generation'],3)

    def test_two_capture_groups_refuse(self):
        def publish(*args):
            self.publish()
            gpu.write(self.f.delivery/'capture-41.stderr',b'')
        with self.assertRaisesRegex(ValueError,'unexpected capture attempt'):
            self.capture(publish)

    def test_filename_and_receipt_generation_must_match(self):
        with self.assertRaisesRegex(ValueError,'identity mismatch'):
            self.capture(lambda *args:self.publish(dict(launch_generation=41)))

    def test_refused_group_refuses(self):
        with self.assertRaisesRegex(ValueError,'private capture refused'):
            self.capture(lambda *args:gpu.write(self.f.delivery/'capture-41.refused',b'refused'))

    def test_terminal_partial_refuses(self):
        def publish(*args):
            self.publish()
            gpu.write(self.f.delivery/'capture-3.pam.partial',b'')
        with self.assertRaisesRegex(ValueError,'delivery is incomplete or refused'):
            self.capture(publish)

    def test_first_observed_generation_cannot_change_or_disappear(self):
        select = c.select.select
        def wake(read, write, error, timeout):
            return ([read[0]],[],[]) if len(read) == 2 else select(read,write,error,timeout)
        for changed in [{41:{'stderr'}}, {}, {3:{'stderr'},41:{'stderr'}}]:
            with self.subTest(changed=changed), patch.object(c.r,'inventory',side_effect=[
                    {}, {3:{'stderr'}}, changed, AssertionError('should have refused')]), patch.object(c.select,'select',side_effect=wake):
                with self.assertRaisesRegex(ValueError,'unexpected capture attempt'):
                    self.capture(lambda *args:None)

    def test_final_inventory_still_has_one_complete_group(self):
        def publish(*args):
            self.publish()
        actual = c.r.inventory
        calls = 0
        def inventory(directory):
            nonlocal calls
            calls += 1
            groups = actual(directory)
            # Initial, watch-loop, verifier before/after reads, then final controller recheck.
            if calls == 5:
                groups[41] = {'stderr'}
            return groups
        with patch.object(c.r,'inventory',side_effect=inventory):
            with self.assertRaisesRegex(ValueError,'concurrent capture attempt'):
                self.capture(publish)

    def test_repeated_draws_do_not_invent_a_second_candidate(self):
        def drawn(*args):
            self.publish()
            row = next(x for x in self.f.log.read_bytes().splitlines(keepends=True)
                       if b'sophia_native_composition_sampling' in x)
            self.f.log.write_bytes(self.f.log.read_bytes()+row.replace(b'scene_generation=1',b'scene_generation=2'))
        self.assertEqual(self.capture(drawn)['status'],'static_capture_verified')

    def test_exact_tag_selection_ignores_longer_record_names(self):
        data = static_log(self.checksum)
        longer = (b'sophia_shell_component_catalog schema=1 status=built generation=1 entries=0\n'
                  b'sophia_shell_components_shutdown schema=1 status=quiescent\n'
                  b'sophia_live_native_head_page_flip schema=1 status=test\n')
        self.assertEqual(c.static.snapshot(data+longer,'direct'),c.static.snapshot(data,'direct'))
        embedded = b'untrusted '+next(x for x in data.splitlines(keepends=True)
                                     if x.startswith(b'lom_panel_candidate '))
        with self.assertRaisesRegex(ValueError,'embedded diagnostic'):
            c.static.snapshot(data+embedded,'direct')

    def test_copy_selection(self):
        self.expected['arm']='copy'
        self.f.log.write_bytes(self.prefix+static_log(self.checksum,'copy'))
        self.assertEqual(self.capture(lambda *args:self.publish())['arm'],'copy')

    def test_snapshot_mutations_refuse(self):
        data=static_log(self.checksum)
        mutations=[(b'rendered',b'reused'),(b'width=1920 height=24',b'width=1919 height=24'),
                   (b'gpu_mode=denied',b'gpu_mode=direct'),(b'refresh_millihz=60000',b'refresh_millihz=120000'),
                   (b'state_bits=1',b'state_bits=0'),(b'effective=exact_nearest',b'effective=bilinear'),
                   (b'alpha_mode=premultiplied',b'alpha_mode=straight'),(b'candidate_generation=1',b'candidate_generation=2')]
        for old,new in mutations:
            with self.subTest(old=old),self.assertRaises(ValueError):
                c.static.snapshot(data.replace(old,new),'direct')
        sampling = next(x for x in data.splitlines(keepends=True)
                        if b'sophia_native_composition_sampling' in x)
        for extra in [sampling.replace(b'schema=3', b'schema=2'),
                      sampling.replace(b'1920x24', b'1920x64'),
                      sampling.replace(b'output=7', b'output=8')]:
            with self.subTest(extra=extra), self.assertRaisesRegex(ValueError, 'sampling path'):
                c.static.snapshot(data+extra, 'direct')
        for tag in ['lom_panel_candidate','sophia_live_shell_content','sophia_shell_indicator_state']:
            extra=next(x for x in data.splitlines(keepends=True) if x.startswith(tag.encode()))
            with self.subTest(tag=tag),self.assertRaises(ValueError):
                c.static.snapshot(data+extra,'direct')

    def test_scene_changed_during_capture_refuses(self):
        def changed(*args):
            self.publish()
            self.f.log.write_bytes(self.f.log.read_bytes()+b'sophia_live_shell_content schema=1 status=invalidated\n')
        with self.assertRaisesRegex(ValueError,'not settled'):
            self.capture(changed)

    def test_native_incarnation_mismatch_refuses(self):
        with self.assertRaisesRegex(ValueError,'identity mismatch'):
            self.capture(lambda *args:self.publish(dict(output_generation=2)))

    def test_private_stderr_refuses(self):
        with self.assertRaisesRegex(ValueError,'oversized private capture file'):
            self.capture(lambda *args:self.publish(stderr=b'error'))

    def test_missing_terminal_receipt_times_out_without_retry(self):
        with self.assertRaisesRegex(ValueError,'deadline'):
            self.capture(lambda *args:None,seconds=0.01)

    def test_prior_attempt_refuses_before_launch(self):
        self.publish()
        with patch.object(c.c,'launch') as launch,self.assertRaisesRegex(ValueError,'unused'):
            c.capture(self.expected,self.f.session,self.f.directory,time.monotonic()+1,self.f.evidence)
        launch.assert_not_called()


if __name__ == '__main__':
    unittest.main()
