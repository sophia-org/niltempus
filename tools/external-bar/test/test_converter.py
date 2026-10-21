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
