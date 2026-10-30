"""Mode/identity refusals and local collector lifecycle; no graphics or Session."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import socket
import array
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('sophia_environment_test', ROOT/'gpu/sophia_environment.py')
e = importlib.util.module_from_spec(spec)
spec.loader.exec_module(e)


def records(pid=123):
    output = (f'sophia_shell_environment_output schema=1 pid={pid} output=1 card=card0 connector=DP-1 '
        'width=1920 height=1080 refresh_millihz=120000 timing_refresh_millihz=119880 scale_milli=1000 vrr=off monotonic_usec=150')
    renderer = (f'sophia_shell_environment_renderer schema=1 pid={pid} output=1 card=card0 driver=amdgpu '
        'vendor=AMD renderer=AMD%20Radeon version=4.6 monotonic_usec=150')
    return [dict(pid=pid,uid=os.getuid(),gid=os.getgid(),received_usec=160+i,record=text,
                 sender_fdinfo=f'pos:\t0\nPid:\t{pid}\nNSpid:\t{pid}\n') for i,text in enumerate((output,renderer))]


def data(rows=None):
    return e.READY + b''.join(json.dumps(r).encode()+b'\n' for r in (records() if rows is None else rows))


class Environment(unittest.TestCase):
    @unittest.skipUnless(os.environ.get('GPU_ENVIRONMENT_TEST_XTASK'), 'requires the built receiver, no GPU')
    def test_real_receiver_credential_pidfd_and_wrapper_join(self):
        executable=str(Path(os.environ['GPU_ENVIRONMENT_TEST_XTASK']).resolve())
        pin=dict(executable=executable,sha256=hashlib.sha256(Path(executable).read_bytes()).hexdigest())
        python=str(Path(sys.executable).resolve())
        host=dict(executable=python,sha256=hashlib.sha256(Path(python).read_bytes()).hexdigest(),
                  pid=os.getpid(),starttime=e.h.c.snapshot(os.getpid()))
        for rights in (True,False):
            with self.subTest(rights=rights),tempfile.TemporaryDirectory() as tmp,patch.object(e,'DURATION_MS',300):
                root=Path(tmp)
                collector=e.Collector(pin,root)
                start=time.monotonic_ns()//1000
                try:
                    with socket.socket(socket.AF_UNIX,socket.SOCK_DGRAM) as sender:
                        sender.connect(str(collector.socket))
                        pidfd=os.pidfd_open(os.getpid())
                        try:
                            for row in records(os.getpid()):
                                raw=row['record'].replace('monotonic_usec=150',f'monotonic_usec={start}').encode()
                                control=[(socket.SOL_SOCKET,socket.SCM_RIGHTS,array.array('i',[pidfd]))] if rights else []
                                sender.sendmsg([raw],control)
                        finally:os.close(pidfd)
                    with patch.object(e.external,'drm_layout'):
                        if rights:
                            result=collector.finish(host,start)
                            self.assertEqual(result['output']['timing_refresh_millihz'],'119880')
                        else:
                            with self.assertRaises(ValueError):collector.finish(host,start)
                    self.assertEqual((root/'environment.json').exists(),rights)
                finally:collector.close()

    def test_actual_timing_and_separate_renderer_are_retained(self):
        value=e.qualify(data(),dict(pid=123),100,200)
        self.assertEqual(value['output']['timing_refresh_millihz'],'119880')
        self.assertEqual(value['output']['refresh_millihz'],'120000')
        self.assertEqual(value['renderer']['renderer'],'AMD Radeon')

    def test_identity_clock_and_receipt_mutations_refuse(self):
        mutations=[lambda r:r.update(pid=124),lambda r:r.update(uid=os.getuid()+1),
            lambda r:r.update(gid=os.getgid()+1),lambda r:r.update(pid=True),
            lambda r:r.update(received_usec=149),lambda r:r.update(received_usec=201),
            lambda r:r.update(sender_fdinfo='Pid:\t124\n'),lambda r:r.update(sender_fdinfo='Pid:\t123\nPid:\t123\n'),
            lambda r:r.update(extra=1),lambda r:r.update(record=None)]
        for mutate in mutations:
            rows=records();mutate(rows[0])
            with self.subTest(rows=rows),self.assertRaises((ValueError,TypeError)):
                e.qualify(data(rows),dict(pid=123),100,200)
        for host,start,end in ((dict(pid=True),100,200),(dict(pid=124),100,200),(dict(pid=123),151,200),
                               (dict(pid=123),100,160),(dict(pid=123),True,200)):
            with self.assertRaises(ValueError):e.qualify(data(),host,start,end)

    def test_mode_renderer_and_grammar_mutations_refuse(self):
        edits=[(0,'timing_refresh_millihz=119880','timing_refresh_millihz=120000'),
            (0,'refresh_millihz=120000','refresh_millihz=119880'),(0,'width=1920','width=01920'),
            (0,'height=1080','height=24'),(0,'scale_milli=1000','scale_milli=2000'),
            (0,'vrr=off','vrr=on'),(0,'connector=DP-1','connector=DP-4'),(0,'card=card0','card=card1'),
            (0,'output=1','output=2'),(0,'pid=123','pid=0123'),(0,'schema=1','schema=1 schema=1'),
            (0,'width=1920','width=1920 unknown=1'),(0,'width=1920',' width=1920'),
            (1,'monotonic_usec=150','monotonic_usec=151'),(1,'driver=amdgpu','driver=nouveau'),
            (1,'renderer=AMD%20Radeon','renderer=llvmpipe'),(1,'version=4.6','version=%34.6'),
            (1,'version=4.6','version=%0A'),(1,'vendor=AMD','vendor=')]
        for index,old,new in edits:
            rows=records();rows[index]['record']=rows[index]['record'].replace(old,new)
            with self.subTest(edit=(old,new)),self.assertRaises(ValueError):e.qualify(data(rows),dict(pid=123),100,200)
        rows=records();rows[1]['received_usec']=159
        with self.assertRaises(ValueError):e.qualify(data(rows),dict(pid=123),100,200)

    def test_partial_extra_duplicate_and_reordered_records_refuse(self):
        for raw in (data()[:-1],data()+b'\n',data()[len(e.READY):],data(records()[::-1]),data([]),
                    data([None,None]),data([[],[]]),data([True,True]),
                    data(records()[:1]),data(records()+records()),data().replace(b'"pid": 123',b'"pid":123,"pid":123',1)):
            with self.subTest(raw=raw),self.assertRaises(ValueError):e.qualify(raw,dict(pid=123),100,200)

    def test_local_collector_ready_failure_and_cleanup(self):
        # Real private child stdout and pidfd/exe checks; synthetic receipts are
        # intentionally not an authenticated receiver or protected-launch proof.
        executable=str(Path(sys.executable).resolve())
        pin=dict(executable=executable,sha256=hashlib.sha256(Path(executable).read_bytes()).hexdigest())
        real_spawn=subprocess.Popen
        for mode in ('good','stderr','missing','exit','ready_error'):
            with self.subTest(mode=mode),tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp)
                child_holder=[]
                script='import sys,time; sys.stdout.write('+repr(e.READY.decode())+'); sys.stdout.flush(); time.sleep(.05); '
                if mode=='ready_error':script='import sys; sys.stderr.write("bad"); sys.exit(1)'
                elif mode in ('good','stderr','exit'):
                    script+='sys.stdout.write('+repr(data(records(os.getpid()))[len(e.READY):].decode())+'); '
                    if mode=='stderr':script+='sys.stderr.write("bad")'
                    if mode=='exit':script+='sys.exit(1)'
                def spawn(_pin,_argv,**kw):
                    child=real_spawn([executable,'-I','-B','-c',script],**kw);child_holder.append(child);return child
                collector=None
                try:
                    with patch.object(e.external,'spawn',side_effect=spawn):
                        if mode=='ready_error':
                            with self.assertRaises(ValueError):e.Collector(pin,root)
                            continue
                        collector=e.Collector(pin,root)
                    host=dict(pin,pid=os.getpid(),starttime=e.h.c.snapshot(os.getpid()))
                    with patch.object(e.external,'drm_layout') as drm:
                        if mode=='good':
                            result=collector.finish(host,100)
                            self.assertEqual(result['status'],'startup_environment_verified')
                            drm.assert_called_once()
                        else:
                            with self.assertRaises(ValueError):collector.finish(host,100)
                            drm.assert_not_called()
                    self.assertEqual((root/'environment.json').exists(),mode=='good')
                finally:
                    if collector is not None:collector.close()
                    self.assertTrue(all(c.poll() is not None for c in child_holder))


if __name__=='__main__':unittest.main()
