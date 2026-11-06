"""Dual-accounting fixtures and identity boundaries; no graphics or native Session."""
import copy
import contextlib
import io
import json
import os
import subprocess
import sys
import time
from types import SimpleNamespace
from pathlib import Path
import unittest
from unittest.mock import patch
sys.path.insert(0, os.path.dirname(__file__))
import cpuwin as c
from test_cpuwin import Case, HEAD, EXPECT, MS, samples

class ProcessClock(Case):
    def capture(self):
        h=copy.deepcopy(HEAD);h.update(version=2,cpu_accounting='process-clock')
        for p in h['procs'].values():p['clock_resolution_ns']=1
        rows=samples()
        for row in rows:
            for p in row['procs'].values():p['process_cpu_ns']=p['ticks']*10*MS
        return h,rows
    def reduce(self,h,rows,mode='process-clock'):
        return c.analyze_run(dict(name='r',arm='x',samples=self.write(rows,header=h),
            start_frame=120,end_frame=2120,anchor_start_ns=105*MS,anchor_end_ns=505*MS,
            expect=EXPECT,cpu_accounting=mode))
    def test_dual_bounds(self):
        h,r=self.capture();x=self.reduce(h,r)
        self.assertEqual(x['cpu_accounting'],'process-clock')
        for n,p in x['procs'].items():
            old=x['tick_crosscheck_per_proc'][n]
            self.assertEqual(p['lower_ns'],old['lower_ns']+20*MS)
            self.assertEqual(p['upper_ns'],old['upper_ns']-20*MS)
    def test_refusals(self):
        for kind in ['mode','resolution','missing','negative','regression','disagree']:
            h,r=self.capture()
            if kind=='mode':h['cpu_accounting']='other'
            if kind=='resolution':h['procs']['session']['clock_resolution_ns']=0
            if kind=='missing':del r[20]['procs']['session']['process_cpu_ns']
            if kind=='negative':r[20]['procs']['session']['process_cpu_ns']=-1
            if kind=='regression':r[20]['procs']['session']['process_cpu_ns']=0
            if kind=='disagree':
                for row in r:row['procs']['session']['process_cpu_ns']*=10
            with self.subTest(kind=kind),self.assertRaises(c.Refused):self.reduce(h,r)
        h,r=self.capture()
        with self.assertRaises(c.Refused):self.reduce(h,r,'ticks')
    def test_identity_around_clock(self):
        p=dict(pid=42,fd=7,comm='p',starttime=1,clock=-7)
        for after in [('p',2,0,0),('replacement',1,0,0)]:
            with patch.object(c.os,'pread',return_value=b'ignored'),patch.object(c,'parse_stat',side_effect=[('p',1,0,0),after]),patch.object(c.time,'clock_gettime_ns',return_value=5),self.assertRaises(c.Refused):c.process_read(p,True)
        with patch.object(c.os,'pread',side_effect=[b'ignored',ProcessLookupError()]),patch.object(c,'parse_stat',return_value=('p',1,0,0)),patch.object(c.time,'clock_gettime_ns',return_value=5),self.assertRaises(OSError):c.process_read(p,True)
    def test_foreign_threads_and_exit(self):
        code='''import time,threading
print('ready',flush=True)
input()
def burn():
 end=time.thread_time_ns()+10_000_000
 while time.thread_time_ns()<end:pass
start=time.process_time_ns()
ts=[threading.Thread(target=burn) for _ in range(2)]
for t in ts:t.start()
for t in ts:t.join()
print(time.process_time_ns()-start,flush=True)
input()
'''
        with subprocess.Popen([sys.executable,'-I','-B','-c',code],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True) as child:
            self.assertEqual(child.stdout.readline().strip(),'ready')
            fd=os.open(f'/proc/{child.pid}/stat',os.O_RDONLY)
            try:
                comm,start,_,_=c.parse_stat(os.pread(fd,4096,0).decode());clock,res=c.process_clock(child.pid)
                p=dict(pid=child.pid,fd=fd,comm=comm,starttime=start,clock=clock)
                a=c.process_read(p,True)['process_cpu_ns']
                time.sleep(0.025)
                self.assertLess(c.process_read(p,True)['process_cpu_ns']-a,20*MS)
                child.stdin.write('go\n');child.stdin.flush()
                own=int(child.stdout.readline());b=c.process_read(p,True)['process_cpu_ns']
                self.assertGreaterEqual(b-a,own);self.assertGreaterEqual(own,20*MS);self.assertGreaterEqual(res,1)
                child.stdin.write('exit\n');child.stdin.flush();child.wait(timeout=5)
                with self.assertRaises(OSError):c.process_read(p,True)
            finally:
                os.close(fd)
                if child.poll() is None:
                    child.kill()
                child.wait(timeout=5)
    def test_sampler_cli_emits_version_two_without_dropping_ticks(self):
        _,start,_,_=c.parse_stat(Path(f'/proc/{os.getpid()}/stat').read_text())
        output=Path(self.dir.name)/'live.jsonl'
        result=subprocess.run([sys.executable,'-I','-B',c.__file__,'sample',
            '--out',str(output),'--seconds','0.05','--cpu-accounting','process-clock',
            '--pid',f'parent={os.getpid()}:{start}'],capture_output=True,text=True,timeout=5)
        self.assertEqual(result.returncode,0,result.stderr)
        header,rows,footer=c.load(output)
        self.assertEqual(header['version'],2)
        self.assertEqual(footer['stop'],'seconds')
        self.assertGreaterEqual(len(rows),2)
        for row in rows:
            self.assertIn('ticks',row['procs']['parent'])
            self.assertIn('process_cpu_ns',row['procs']['parent'])

    def test_mixed_accounting_plans_refuse_before_loading(self):
        plan=Path(self.dir.name)/'mixed.json'
        plan.write_text(json.dumps({'runs':[{'cpu_accounting':'ticks'},
            {'cpu_accounting':'process-clock'}]}))
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(c.cmd_analyze(SimpleNamespace(plan=plan)),1)

if __name__=='__main__':unittest.main()
