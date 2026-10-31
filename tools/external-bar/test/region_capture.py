"""Fixed phase contracts and real private receipt controls; no native launch."""
import importlib.util
import json
from pathlib import Path
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


c = load('sequence_controller', ROOT/'cpu/sequence_capture.py')
static = load('static_fixture', ROOT/'test/cpu_capture.py')
gpu = static.gpu
line = static.line


def phase_log(arm='regions-24', through=8, final=False):
    role, height = arm.split('-'); height = int(height)
    base = static.static_log('unavailable', f'hash-off-{height}', height).decode()
    omitted = ('lom_renderer ', 'lom_panel_candidate ')
    lines = [row for row in base.splitlines() if not row.startswith(omitted)
             and not (row.startswith('sophia_live_shell_content ') and 'status=outputs ' not in row)]
    text = '\n'.join(lines)+'\n'
    text += ('lom_renderer schema=1 renderer=vello-cpu source=owned-ring-fill gpu=not-used '
             f'diagnostic=cpu-region-sequence role={role}\n')
    text += line('lom_region_sequence', schema=1, source='fixture', role=role, phases=9, hold_ms=10000)
    names = ['initial', 'establish', 'seconds', 'workspace', 'minute', 'rename', 'restore', 'reestablish', 'seconds-again']
    for phase in range(through+1):
        common = dict(schema=1, source='fixture', role=role, phase=phase, name=names[phase])
        text += line('lom_region_phase', event='installed', **common)
        for status in ['prepared', 'presented']:
            row = dict(schema=1, status=status, output=7, candidate_generation=phase+1,
                       staging_bytes=0, resident_bytes=1920*height*4, retiring_bytes=0, backing_bytes=1920*height*4)
            if status == 'presented': row.update(presentation_epoch=100+phase, connection_epoch=1, content_grant_epoch=1)
            text += line('sophia_live_shell_content', **row)
        text += line('lom_panel_candidate', schema=2, status='presented', connection_epoch=1, content_grant_epoch=1,
            output=7, candidate_generation=phase+1, presentation_epoch=100+phase, indicator_generation=0,
            width=1920, height=height, bytes=1920*height*4, checksum='unavailable', raster_source='rendered')
        tiled = role == 'regions' and phase in [1, 2, 3, 4, 7, 8]
        uploads = {1:3, 2:1, 3:1, 4:2, 7:3, 8:1}[phase] if tiled else 1
        size = (640*uploads if tiled else 1920)*height*4
        if tiled:
            text += line('lom_panel_regions', schema=1, candidate_generation=phase+1, placements=3,
                         uploads=uploads, upload_bytes=size, retained=3-uploads)
        text += line('lom_region_phase', event='presented', **common, candidate_generation=phase+1,
                     placements=3 if tiled else 1, uploads=uploads, upload_bytes=size, regions_disabled=0)
    if final:
        text += line('lom_region_final', schema=1, source='fixture', role=role, status='complete', phases=9, presented=9)
    return text.encode()


class Records(unittest.TestCase):
    def test_controller_budget_covers_all_holds_and_slack(self):
        self.assertEqual(c.HOLD_SECONDS, 10)
        self.assertEqual(c.CONTROLLER_SECONDS, 9 * 10 + 30)

    def test_all_roles_heights_and_complete_prefix_boundaries(self):
        for arm in c.sequence.ARMS:
            data = phase_log(arm, final=True)
            state = c.sequence.state(data, arm, final=True)
            self.assertEqual(len(state['completed']), 9)
            for phase in range(9):
                prefix = phase_log(arm, through=phase)
                self.assertEqual(c.sequence.snapshot(prefix, arm, phase)['phase']['phase'], str(phase))
            lines = data.splitlines(keepends=True)
            start = next(i for i,row in enumerate(lines) if row.startswith(b'lom_region_phase '))
            for end in range(start, len(lines)):
                c.sequence.state(b''.join(lines[:end]), arm)

    def test_wrong_phase_counts_policy_order_and_predictions_refuse(self):
        data = phase_log()
        for old,new in [(b'source=fixture', b'source=live'), (b'indicator_generation=0', b'indicator_generation=2'),
                (b'checksum=unavailable', b'checksum=a7bbb8c840a7303c'), (b'role=regions', b'role=full'),
                (b'uploads=3', b'uploads=1'), (b'upload_bytes=61440', b'upload_bytes=184320'),
                (b'regions_disabled=0', b'regions_disabled=1'), (b'raster_source=rendered', b'raster_source=reused'),
                (b'presentation_epoch=100', b'presentation_epoch=999'), (b'height=24', b'height=512')]:
            with self.subTest(old=old), self.assertRaises(ValueError):
                c.sequence.state(data.replace(old,new,1), 'regions-24')
        for tag in ['lom_region_sequence', 'lom_region_phase', 'lom_panel_candidate', 'lom_panel_regions']:
            row = next(row for row in data.splitlines(keepends=True) if row.startswith(tag.encode()+b' '))
            for changed in (data.replace(row,b'',1), data+row, row+data.replace(row,b'',1)):
                with self.subTest(tag=tag), self.assertRaises(ValueError):
                    c.sequence.state(changed, 'regions-24')

    def test_final_requires_every_phase_and_capture_requires_exact_hold(self):
        data = phase_log(through=2)
        for phase in [0,1,3,8]:
            with self.assertRaises(ValueError): c.sequence.snapshot(data,'regions-24',phase)
        final = b'lom_region_final schema=1 source=fixture role=regions status=complete phases=9 presented=9\n'
        with self.assertRaises(ValueError): c.sequence.state(data+final,'regions-24',final=True)
        with self.assertRaises(ValueError): c.sequence.state(phase_log(),'regions-24',final=True)

    def test_wrong_group_or_extra_declaration_key_refuses(self):
        f = gpu.Fixture(); self.addCleanup(f.close)
        expected = {k:v for k,v in f.expected.items() if k != 'hold'}
        expected.update(arm='regions-24', group=0)
        self.assertEqual(c.declared(expected),(0,1,2))
        for group in [True,-1,3,'0',None]:
            with self.assertRaises(ValueError): c.declared(dict(expected,group=group))
        with self.assertRaises(ValueError): c.declared(dict(expected, phases=[0,1,2]))


class Capture(unittest.TestCase):
    def setUp(self):
        self.f=gpu.Fixture(); self.addCleanup(self.f.close)
        self.expected={k:v for k,v in self.f.expected.items() if k != 'hold'}
        self.expected.update(arm='regions-24',group=0)
        self.prefix=f'sophia_developer_launch schema=1 status=enabled admission=host-admin socket={self.f.endpoint}\n'.encode()

    def set_phase(self, phase):
        self.f.log.write_bytes(self.prefix+phase_log(self.expected['arm'], through=phase))

    def run_group(self, mutate=None):
        first=self.expected['group']*3; self.set_phase(first)
        real_read=c.c.log_bytes
        def read(path):
            done=sum((self.f.evidence/f'phase-{p}'/'result.json').exists() for p in range(first,first+3))
            self.set_phase(min(first+done,first+2))
            return real_read(path)
        launches=[]
        def launch(*args):
            launches.append(args)
            # The shared GPU fixture's second pattern has R == B everywhere.
            with patch.object(gpu, 'image', return_value=gpu.image(1)):
                self.f.publish(len(launches))
            if mutate: mutate(len(launches))
        with patch.object(c.c,'log_bytes',side_effect=read), patch.object(c.c,'launch',side_effect=launch):
            result=c.run(self.expected,self.f.session,self.f.directory,time.monotonic()+5,self.f.evidence)
        self.assertEqual(len(launches),3)
        return result

    def test_three_serial_receipts_keep_distinct_generation_domains(self):
        result=self.run_group()
        self.assertLessEqual(result['controller_started_monotonic_ns'],
                             result['controller_finished_monotonic_ns'])
        self.assertEqual(result['phases'],[0,1,2])
        for i,row in enumerate(result['captures']):
            self.assertEqual(row['capture']['receipt']['launch_generation'],41+i)
            self.assertEqual(row['scene']['candidate']['presentation_epoch'],str(100+i))
            self.assertIsNone(row['capture']['pixels']['produced_fnv64'])
            self.assertLessEqual(row['capture_started_ns'],row['capture_verified_ns'])

    def test_last_group_captures_only_designated_phases(self):
        self.expected['group']=2
        self.assertEqual(self.run_group()['phases'],[6,7,8])

    def test_middle_group_at_512_for_both_roles(self):
        for role in ['full','regions']:
            fixture=gpu.Fixture()
            try:
                self.f=fixture
                self.expected={k:v for k,v in fixture.expected.items() if k != 'hold'}
                self.expected.update(arm=f'{role}-512',group=1)
                self.prefix=f'sophia_developer_launch schema=1 status=enabled admission=host-admin socket={fixture.endpoint}\n'.encode()
                result=self.run_group()
                self.assertEqual(result['phases'],[3,4,5])
                self.assertTrue(all(row['capture']['pixels']['checked_pixels']==1920*512 for row in result['captures']))
            finally: fixture.close()

    def test_previous_receipt_bytes_cannot_change(self):
        def mutation(n):
            if n==2: (self.f.delivery/'capture-41.pam').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError,'prior delivery changed'): self.run_group(mutation)

    def test_phase_install_during_capture_refuses(self):
        self.set_phase(0)
        def launch(*args): self.f.publish(); self.set_phase(1)
        with patch.object(c.c,'launch',side_effect=launch), self.assertRaisesRegex(ValueError,'designated Presented hold'):
            c.run(self.expected,self.f.session,self.f.directory,time.monotonic()+2,self.f.evidence)

    def test_missed_phase_refuses_before_launch(self):
        self.set_phase(1)
        with patch.object(c.c,'launch') as launch, self.assertRaisesRegex(ValueError,'missed designated phase'):
            c.run(self.expected,self.f.session,self.f.directory,time.monotonic()+2,self.f.evidence)
        launch.assert_not_called()


if __name__ == '__main__': unittest.main()
