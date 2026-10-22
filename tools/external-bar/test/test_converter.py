import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CONVERTER = Path(__file__).resolve().parents[1]/'common/sophia2journal.py'


class Converter(unittest.TestCase):
    def fixture(self):
        identity = dict(connection_epoch=1, content_grant_epoch=2, output=3, candidate_generation=1)
        upload = dict(identity, resource_id=1, resource_generation=1, width=1920,
                      height=24, total_bytes=184320, begin_usec=10)
        permit = dict(generation=1, demand_usec=20, received_usec=25)
        offer = dict(identity, submitted_monotonic_usec=30)
        start = dict(schema=2, event='start', wire='files', source='sealed', probe_bytes=0, frames=1)
        events = [start, dict(schema=2,event='upload_offer',upload=upload),
                  dict(schema=2,event='permit',**permit), dict(schema=2,event='offer',**offer),
                  dict(schema=2,event='terminal',outcome='presented',observed_monotonic_usec=45,**identity),
                  dict(schema=2,event='end',offered=1,completed=1)]
        native = dict(schema=2, verdict='joined_only', costs=None,
                      samples=[dict(offer,heads=1,completed_monotonic_usec=40)],
                      journal=dict(schema=2,wire='files',source='sealed',probe_bytes=0,
                                   uploads=[dict(upload=upload)],permits=[permit],terminal_usec=[45]))
        return events, native

    def ring_fixture(self, workload):
        events, native = self.fixture()
        for event in events:
            event['schema'] = 5
        start = events[0]
        start.update(source='owned_ring', ring_slots=2, workload='fresh')
        metadata = native['journal']
        metadata.update(schema=5, source='owned_ring')  # Rust omits the default fresh workload.
        ready = dict(connection_epoch=1, content_grant_epoch=2, channel=1, ring=1,
                     ring_generation=1, slots=2, pixel_bytes=184320, admitted_usec=1, ready_usec=2)
        upload = events[1]['upload']
        offer = dict(schema=5, event='ring_offer', offer=1, slot=0, slot_generation=2,
                     fill_start_usec=5, upload=upload)
        accepted = dict(schema=5, event='ring_accepted', offer=1, resource_id=1,
                        resource_generation=1, accepted_usec=12)
        released = dict(schema=5, event='slot_released', offer=1, resource_id=1,
                        resource_generation=1, slot=0, slot_generation=2, released_usec=15)
        copied = {k: released[k] for k in ('offer','resource_id','resource_generation','slot','slot_generation','released_usec')}
        copied['fill_start_usec'] = 5
        metadata['uploads'][0]['accepted_usec'] = 12
        metadata['ring'] = dict(registration=ready, copies=[copied])
        events = [start, dict(schema=5, event='ring_ready', **ready), offer, accepted, released, *events[2:]]
        if workload == 'bursty':
            start['interval_msec'] = metadata['interval_msec'] = 250
            offer['due_usec'] = copied['due_usec'] = 4
            native['external_timing'] = dict(workload='bursty', interval_msec=250, warmup=0)
        elif workload == 'idle':
            native['external_timing'] = dict(workload='idle', seconds=60)
        elif workload == 'retained':
            start.update(workload='retained', retained_resources=8)
            metadata.update(workload='retained', retained_resources=8)
            ready['pixel_bytes'] = events[1]['pixel_bytes'] = 240*24*4
            pool, copies, uploads, placed = [], [], [], []
            for i in range(8):
                o, a, r = copy.deepcopy([offer, accepted, released])
                identity = dict(offer=i+1, resource_id=i+1, resource_generation=1)
                o.update(offer=i+1, slot=i%2, slot_generation=2*(i+1))
                o['upload'].update(resource_id=i+1, candidate_generation=0, width=240, total_bytes=240*24*4)
                a.update(identity)
                r.update(identity, slot=i%2, slot_generation=2*(i+1))
                pool.extend([o,a,r])
                copies.append(dict(identity, slot=i%2, slot_generation=2*(i+1), fill_start_usec=5, released_usec=15))
                uploads.append(dict(purpose='retained', upload=o['upload'], accepted_usec=12))
                placed.append(dict(resource_id=i+1,resource_generation=1,destination_x_px=i*240,destination_y_px=0))
            placement = dict(candidate_generation=1, placements=placed)
            metadata.update(uploads=uploads, placements=[placement])
            metadata['ring']['copies'] = copies
            events = [*events[:2], *pool, events[5], dict(schema=5,event='candidate_placements',**placement), *events[6:]]
        return events, native

    def run_converter(self, events, native, *args):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root/'client.ndjson').write_text(''.join(json.dumps(e)+'\n' for e in events))
            (root/'native-samples.json').write_text(json.dumps(native))
            output = root/'journal.tsv'
            result = subprocess.run([sys.executable,str(CONVERTER),str(root),'--out',str(output),*args],capture_output=True,text=True)
            return result, output.read_text() if output.exists() else ''

    def test_validated_timing_not_raw_replacements_or_fabricated_cpu(self):
        events, native = self.fixture()
        result, journal = self.run_converter(events, native)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('workload=fresh', journal)
        self.assertNotIn('# rusage', journal)
        changed = copy.deepcopy(events)
        changed[1]['upload']['begin_usec'] = 11
        self.assertNotEqual(self.run_converter(changed,native)[0].returncode,0)
        duplicate = events[:4] + [events[3]] + events[4:]
        self.assertNotEqual(self.run_converter(duplicate,native)[0].returncode,0)
        self.assertNotEqual(self.run_converter(events,native,'--idle-sec','60')[0].returncode,0)

    def test_ring_workloads_preserve_fill_schedule_and_custody_evidence(self):
        for workload in ('fresh','bursty','idle','retained'):
            with self.subTest(workload=workload):
                events,native=self.ring_fixture(workload)
                result,journal=self.run_converter(events,native)
                self.assertEqual(result.returncode,0,result.stderr)
                self.assertIn('workload='+workload,journal)
                self.assertIn('mode sophia_files_owned_ring',journal)
                if workload == 'bursty':
                    self.assertIn('fill=bursty_static_background_v1',journal)
                    self.assertIn('1\t5000\t30000\t40000\t4000\tsophia_kernel_flip',journal)
                changed=copy.deepcopy(events)
                next(e for e in changed if e['event']=='slot_released')['slot_generation'] += 2
                self.assertNotEqual(self.run_converter(changed,native)[0].returncode,0)
                if workload == 'bursty':
                    changed=copy.deepcopy(events)
                    next(e for e in changed if e['event']=='ring_offer')['due_usec'] += 1
                    self.assertNotEqual(self.run_converter(changed,native)[0].returncode,0)
                if workload == 'retained':
                    changed=copy.deepcopy(events)
                    next(e for e in changed if e['event']=='candidate_placements')['placements'][0]['resource_id'] = 9
                    self.assertNotEqual(self.run_converter(changed,native)[0].returncode,0)

    def test_bursty_does_not_invent_due_time_or_matching_damage_pattern(self):
        events, native = self.fixture()
        for event in events: event['schema']=4
        events[0]['interval_msec']=250
        native['journal'].update(schema=4,interval_msec=250)
        native['external_timing']=dict(workload='bursty',interval_msec=250,warmup=0)
        result, journal = self.run_converter(events,native)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertIn('workload=bursty',journal)
        self.assertIn('fill=shade_mod128_a255',journal)
        self.assertIn('\t-1\tsophia_kernel_flip',journal)
        self.assertIn('scheduled due times unavailable',journal)


if __name__ == '__main__': unittest.main()
