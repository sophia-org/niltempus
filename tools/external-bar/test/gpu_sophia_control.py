"""Synthetic source records and real credential/pidfd channels; no Sophia/GPU launch."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def load(name,path):
    spec=importlib.util.spec_from_file_location(name,path)
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    return module


s=load('one_shot',ROOT/'gpu/control_sophia.py')
fixture=load('one_shot_capture_fixture',ROOT/'test/gpu_capture.py')


def ready(kind):
    return dict(schema=1,status='control_ready',kind=kind,connection_epoch=2,
                content_grant_epoch=3,output=7,shell_output_generation=90,nonce='1'*32)


def initial(kind,endpoint='/tmp/shell.sock'):
    mode='gpu_release_delay_mode schema=1 frames=1 delay_usec=100000 timed=0\n' if kind=='delay' else ''
    return (mode+'gpu_bar_dma_mode schema=1 mode=fresh\n'+
            f'gpu_control schema=1 status=endpoint_hint shell_socket={endpoint}\n').encode()


def body(kind):
    grant=dict(connection_epoch=2,content_grant_epoch=3,output=7)
    records=[dict(schema=2,event='start',frames=1,wire='files',probe_bytes=0,source='dma_buf'),
             dict(schema=2,event='upload_offer',upload=dict(**grant,candidate_generation=1,
                  resource_id=1,resource_generation=1,width=1920,height=24,total_bytes=184320,begin_usec=900000))]
    if kind=='delay':
        trace=''.join(f'gpu_release_delay schema=1 phase={phase} offer=9 connection_epoch=2 content_grant_epoch=3 resource=1 generation=1 monotonic_usec={clock}\n'
                      for phase,clock in zip(('held','resume','released'),(1000000,1100000,1100001)))
        records += [dict(schema=2,event='upload_accepted',resource_id=1,resource_generation=1,accepted_usec=1100002),
                    dict(schema=2,event='permit',generation=1,demand_usec=1100003,received_usec=1100004),
                    dict(schema=2,event='offer',**grant,candidate_generation=1,submitted_monotonic_usec=1100005),
                    dict(schema=2,event='terminal',**grant,candidate_generation=1,outcome='presented',kind=2,reason=0,observed_monotonic_usec=1100006),
                    dict(schema=2,event='end',offered=1,completed=1)]
    else:
        trace=('gpu_invalid_import schema=1 transport=sophia status=injected offer=9 connection_epoch=2 content_grant_epoch=3 resource=1 generation=1 bytes=184320\n'
               'gpu_invalid_import_refused schema=1 transport=sophia offer=9 connection_epoch=2 content_grant_epoch=3 resource=1 generation=1 status=3 reason=9 released=0\n')
    return (''.join(json.dumps(row,separators=(',',':'))+'\n' for row in records)+trace).encode()


PEER=r'''
import json,socket,sys
from pathlib import Path
endpoint,log,value,payload,mode=sys.argv[1:]
value=json.loads(value)
s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.settimeout(5);s.connect(endpoint)
reader=s.makefile('rb')
data=(json.dumps(value)+'\n').encode()
if mode=='duplicate': data+=data
s.sendall(data)
line=reader.readline(1025)
if not line: sys.exit(0)
assert json.loads(line)==dict(schema=1,status='ack',generation=1,nonce=value['nonce'])
with open(log,'ab') as f: f.write(payload.encode())
done=dict(schema=1,status='control_done',kind=value['kind'])
if mode=='wrong_done': done['kind']='foreign'
s.sendall((json.dumps(done)+'\n').encode())
assert reader.read(1)==b''
'''


class OneShot(unittest.TestCase):
    def setUp(self):
        self.f=fixture.Fixture();self.addCleanup(self.f.close)
        self.pin={key:self.f.pin[key] for key in ('executable','sha256')}

    def test_exact_source_records_for_both_controls(self):
        for kind in ('delay','invalid'):
            result=s.qualify(initial(kind)+body(kind),kind,ready(kind))
            self.assertEqual(result['status'],'release_application_delayed' if kind=='delay' else 'consumer_import_refused')
        malformed=s.qualify((initial('invalid')+body('invalid')).replace(b'reason=9',b'reason=3'),'invalid',ready('invalid'))
        self.assertEqual(malformed['reason'],3)
        self.assertIn('does not prove',malformed['limits'])

    def test_each_record_required_and_duplicate_or_foreign_refuses(self):
        for kind in ('delay','invalid'):
            log=initial(kind)+body(kind)
            for line in log.splitlines(keepends=True):
                for changed in (log.replace(line,b'',1),log+line):
                    with self.subTest(kind=kind,line=line),self.assertRaises(ValueError):
                        s.qualify(changed,kind,ready(kind))
            for extra in (b'gpu_colour_hold_refused error=x\n',b'gpu_release_delay_failed error=x\n',
                          b'gpu_invalid_import_failed error=x\n',b'gpu_bar_profile enabled=true\n'):
                with self.assertRaises(ValueError):s.qualify(log+extra,kind,ready(kind))

    def test_identity_status_schema_and_clock_mutations_refuse(self):
        common=[(b'"output":7',b'"output":8'),(b'connection_epoch=2',b'connection_epoch=3'),
                (b'"width":1920',b'"width":1'),(b'"frames":1',b'"frames":true'),
                (b'"schema":2',b'"schema":true'),(b'mode=fresh',b'mode=reuse')]
        for kind in ('delay','invalid'):
            changes=common+([(b'reason=9',b'reason=4'),(b'released=0',b'released=1'),(b'offer=9',b'offer=0'),
                             (b'status=3',b'status=2'),(b'bytes=184320',b'bytes=1')]
                            if kind=='invalid' else [(b'monotonic_usec=1100000',b'monotonic_usec=1000001'),
                             (b'"outcome":"presented"',b'"outcome":"rejected"'),
                             (b'"completed":1',b'"completed":0'),(b'"accepted_usec":1100002',b'"accepted_usec":1')])
            original=initial(kind)+body(kind)
            for old,new in changes:
                self.assertIn(old,original)
                with self.subTest(kind=kind,old=old),self.assertRaises(ValueError):
                    s.qualify(original.replace(old,new),kind,ready(kind))

    def test_ready_is_exact_and_kind_bound(self):
        original=ready('delay')
        for key in original:
            bad=dict(original);bad.pop(key)
            with self.assertRaises(ValueError):s.ready(bad,'delay')
        for key,value in [('schema',True),('kind','invalid'),('status','ready'),('nonce','z'*32),('output',0),('connection_epoch',1.0)]:
            with self.assertRaises(ValueError):s.ready(dict(original,**{key:value}),'delay')

    def connect(self,kind,mode='normal'):
        shell=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)
        endpoint=self.f.root/'shell.sock';shell.bind(str(endpoint));endpoint.chmod(0o600);shell.listen(1)
        self.addCleanup(shell.close)
        self.f.log.write_bytes(self.f.prefix+initial(kind,endpoint))
        self.assertEqual(s.h.shell_endpoint(self.f.log.read_bytes(),self.f.session,'gpu_control'),endpoint)
        listener=s.h.Listener(endpoint)
        wrapper='import subprocess,sys; sys.exit(subprocess.call([sys.executable,"-I","-B","-c",sys.argv[1],*sys.argv[2:]]))'
        child=subprocess.Popen([sys.executable,'-I','-B','-c',wrapper,PEER,str(self.f.root/'hold.sock'),str(self.f.log),
                                json.dumps(ready(kind)),body(kind).decode(),mode],stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        def cleanup():
            out,err=child.communicate(timeout=5);self.assertEqual(child.returncode,0,err.decode())
        self.addCleanup(cleanup);self.addCleanup(listener.close)
        self.assertTrue(select.select([listener.socket],[],[],3)[0])
        connection,_=listener.socket.accept();connection.setblocking(False);self.addCleanup(connection.close)
        peer=s.h.Peer(connection,self.f.session,self.pin,self.pin);self.addCleanup(peer.close)
        expected=dict(self.f.expected,peer=self.pin,supervisor=self.pin,control=kind)
        return connection,peer,expected

    def test_kernel_bound_delay_peer_stays_alive_after_done(self):
        connection,peer,expected=self.connect('delay')
        result=s.drive(connection,peer,expected,self.f.evidence)
        self.assertEqual(result['peer_pid'],peer.pid)
        self.assertEqual(result['proof']['status'],'release_application_delayed')
        self.assertFalse(select.select([peer.fd],[],[],0)[0])
        self.assertNotIn(b'"event":', (self.f.evidence/'before-ack.log').read_bytes())

    def test_kernel_bound_invalid_peer_has_no_presented_or_release(self):
        connection,peer,expected=self.connect('invalid')
        result=s.drive(connection,peer,expected,self.f.evidence)
        self.assertFalse(result['proof']['producer_release'])
        self.assertFalse(select.select([peer.fd],[],[],0)[0])

    def test_wrong_done_and_duplicate_ready_refuse(self):
        # One connection per test instance; exercise wrong DONE here, duplicate below.
        connection,peer,expected=self.connect('invalid','wrong_done')
        with self.assertRaisesRegex(ValueError,'DONE'):s.drive(connection,peer,expected,self.f.evidence)

    def test_duplicate_ready_refuses_before_ack(self):
        connection,peer,expected=self.connect('delay','duplicate')
        with self.assertRaisesRegex(ValueError,'early hold records'):s.drive(connection,peer,expected,self.f.evidence)
        self.assertFalse((self.f.evidence/'before-ack.log').exists())

    def test_work_before_authenticated_ack_refuses(self):
        connection,peer,expected=self.connect('invalid')
        with self.f.log.open('ab') as log:log.write(body('invalid'))
        with self.assertRaisesRegex(ValueError,'before authenticated ACK'):
            s.drive(connection,peer,expected,self.f.evidence)
        self.assertFalse((self.f.evidence/'before-ack.log').exists())

    def actual_driver_cli_and_wrapper(self,kind):
        w=load('one_shot_wrapper',ROOT/'gpu/run_hold_sophia.py')
        root=self.f.root/'wrapper'
        # The stand-in host owns the shell listener and a supervisor/peer tree.
        # The real driver CLI, credentials, pidfds, log parser and wrapper run.
        supervisor=r'''
import subprocess,sys
raise SystemExit(subprocess.call([sys.executable,'-I','-B','-c',sys.argv[1],*sys.argv[2:]]))
'''
        peer=PEER.replace("s.settimeout(5);s.connect(endpoint)","s.settimeout(5);\nimport time\nwhile not Path(endpoint).exists(): time.sleep(.01)\ns.connect(endpoint)")
        host=f'''
import os,signal,socket,subprocess,sys,time
from pathlib import Path
root=Path({str(root)!r})
shell=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)
endpoint=root/'shell.sock';shell.bind(str(endpoint));endpoint.chmod(0o600);shell.listen(1)
print({initial(kind).decode()!r}.replace('/tmp/shell.sock',str(endpoint)),end='',flush=True)
child=subprocess.Popen([sys.executable,'-I','-B','-c',{supervisor!r},{peer!r},str(root/'hold.sock'),str(root/'session.log'),{json.dumps(ready(kind))!r},{body(kind).decode()!r},'normal'],start_new_session=True)
stopped=False
def stop(*_):
 global stopped
 stopped=True
signal.signal(signal.SIGTERM,stop)
while not stopped: signal.pause()
os.killpg(child.pid,signal.SIGTERM);child.wait(timeout=3);shell.close()
'''
        original=subprocess.Popen
        def spawn(argv,**kwargs):
            if argv[1:3]==['session','run']:
                return original([sys.executable,'-I','-B','-c',host],**kwargs)
            return original(argv,**kwargs)
        def collector(_pin,root):
            from types import SimpleNamespace
            def finish(*_):
                (root/'environment.json').write_text('{}')
                return dict(output=dict(output='7'))
            return SimpleNamespace(socket=root/'environment.sock',finish=finish,close=lambda:None)
        recipe=dict(schema=1,host=self.pin,peer=self.pin,supervisor=self.pin,
                    environment_receiver=self.pin,output_generation=1,control=kind,
                    argv=['session','run','--output-environment-readback'])
        old=os.umask(0o077)
        try:
            with patch.object(w.subprocess,'Popen',side_effect=spawn),patch.object(w,'environment_collector',side_effect=collector):
                result=w.run(recipe,root)
        finally:os.umask(old)
        self.assertEqual(result['control'],kind)
        self.assertEqual(result['driver_exit'],0)
        proof=json.loads((root/'driver/control_verified.json').read_text())
        self.assertEqual(proof['proof']['status'],'consumer_import_refused' if kind=='invalid' else 'release_application_delayed')
        self.assertEqual(json.loads((root/'driver/result.json').read_text())['status'],'session_exited_after_control')
        self.assertFalse((root/'hold.sock').exists())

    def test_actual_invalid_driver_cli_and_wrapper_with_fixture_session_tree(self):
        self.actual_driver_cli_and_wrapper('invalid')

    def test_actual_delay_driver_cli_and_wrapper_with_fixture_session_tree(self):
        self.actual_driver_cli_and_wrapper('delay')


if __name__=='__main__':
    unittest.main()
