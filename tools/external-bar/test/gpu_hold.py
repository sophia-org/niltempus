"""Process/socket hold fixtures, with no protected launch, Sophia session or GPU."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import select
import signal
import socket
import subprocess
import sys
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


h = load('hold', ROOT / 'gpu/hold_sophia.py')
w = load('wrapper', ROOT / 'gpu/run_hold_sophia.py')
fixture = load('capture_fixture', ROOT / 'test/gpu_capture.py')

PEER = r'''
import json, socket, sys
endpoint, log, mode = sys.argv[1:]
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(5)
s.connect(endpoint)
reader = s.makefile('rb')
for g in (1,2,3):
    with open(log, 'ab') as f:
        for status in ('prepared','presented'):
            row = dict(schema=1,status=status,output=7,candidate_generation=g,
                       staging_bytes=0,resident_bytes=184320,retiring_bytes=0,backing_bytes=184320)
            if status == 'presented':
                row.update(presentation_epoch=g*10,connection_epoch=2,content_grant_epoch=3)
            f.write(('sophia_live_shell_content '+' '.join(f'{k}={v}' for k,v in row.items())+'\n').encode())
    value = dict(schema=1,status='ready',generation=g,pattern=2 if g==2 else 1,
                 connection_epoch=2,content_grant_epoch=3,output=7,shell_output_generation=90,
                 presentation_epoch=g*10,nonce=f'{g:032x}')
    data = (json.dumps(value)+'\n').encode()
    if mode=='early': data += data
    s.sendall(data)
    line = reader.readline(1025)
    if not line: sys.exit(0)
    ack=json.loads(line)
    assert ack==dict(schema=1,status='ack',generation=g,nonce=value['nonce'])
    print(f'ACK{g}',flush=True)
s.sendall(b'{"schema":1,"status":"done","generation":3}\n')
assert reader.read(1)==b''
'''


class Hold(unittest.TestCase):
    def setUp(self):
        self.f = fixture.Fixture()
        self.addCleanup(self.f.close)
        self.shell = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.shell_path = self.f.root / 'shell.sock'
        self.shell.bind(str(self.shell_path))
        self.shell_path.chmod(0o600)
        self.shell.listen(1)
        self.addCleanup(self.shell.close)
        self.hint = f'gpu_colour_hold schema=1 status=endpoint_hint shell_socket={self.shell_path}\n'.encode()
        self.f.log.write_bytes(self.f.prefix + self.hint)
        self.pin = {key: self.f.pin[key] for key in ('executable','sha256')}
        self.expected = {key: self.f.expected[key] for key in
                         ('schema','session','session_log','delivery_directory','output_generation')}
        self.expected.update(peer=self.pin, supervisor=self.pin)

    def connect_peer(self, mode='normal'):
        listener = h.Listener(self.shell_path)
        wrapper = 'import subprocess,sys; sys.exit(subprocess.call([sys.executable,"-I","-B","-c",sys.argv[1],*sys.argv[2:]]))'
        child = subprocess.Popen([sys.executable,'-I','-B','-c',wrapper,PEER,
                                  str(self.f.root/'hold.sock'),str(self.f.log),mode],
                                 stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        def cleanup():
            stdout, stderr = child.communicate(timeout=5)
            self.output = stdout
            self.assertEqual(child.returncode, 0, stderr.decode())
        self.addCleanup(cleanup)
        self.addCleanup(listener.close)
        self.assertTrue(select.select([listener.socket],[],[],3)[0])
        connection, _ = listener.socket.accept()
        connection.setblocking(False)
        self.addCleanup(connection.close)
        peer = h.Peer(connection,self.f.session,self.pin,self.pin)
        self.addCleanup(peer.close)
        return listener, connection, peer, child

    def test_listener_is_published_listening_then_removed(self):
        endpoint = h.shell_endpoint(self.f.log.read_bytes(), self.f.session)
        self.assertEqual(endpoint, self.shell_path)
        listener = h.Listener(endpoint)
        try:
            self.assertFalse((self.f.root/'hold.pending').exists())
            with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as client:
                client.connect(str(self.f.root/'hold.sock'))
                connection,_=listener.socket.accept()
                connection.close()
            with self.assertRaises(ValueError): h.Listener(endpoint)
        finally: listener.close()
        self.assertFalse((self.f.root/'hold.sock').exists())
        with self.assertRaises(ValueError):
            h.shell_endpoint(self.f.log.read_bytes().replace(str(endpoint).encode(),b'/tmp/foreign/shell.sock'),self.f.session)

    def test_kernel_peer_pin_and_three_exact_capture_acks(self):
        _,connection,peer,_=self.connect_peer()
        peer.check()
        calls=[]
        def capture(expected,evidence,*_):
            calls.append(expected['hold'])
            self.assertEqual(expected['output_generation'],1) # Not shell facts generation90.
            self.assertFalse(select.select([connection],[],[],0)[0])
            return 'a'*64
        with patch.object(h,'run_capture',side_effect=capture):
            result=h.drive(connection,peer,self.expected,self.f.evidence)
        self.assertEqual([x['generation'] for x in calls],[1,2,3])
        self.assertEqual([x['pattern'] for x in calls],[1,2,1])
        self.assertEqual(len(result['records']),3)
        self.assertEqual(result['peer_pid'],peer.pid)
        # A different artifact or ancestry is not accepted on the same connection.
        wrong=dict(self.pin,sha256='0'*64)
        with self.assertRaises(ValueError): h.Peer(connection,self.f.session,wrong,self.pin)
        old=peer.starttime
        peer.starttime+=1
        with self.assertRaises(ValueError): peer.check()
        peer.starttime=old

    def test_restart_during_capture_refuses_before_ack(self):
        _, connection, peer, child = self.connect_peer()
        def capture(*_):
            with self.f.log.open('ab') as log:
                log.write(self.hint)
            return 'a' * 64
        with patch.object(h, 'run_capture', side_effect=capture):
            with self.assertRaisesRegex(ValueError, 'restarted'):
                h.drive(connection, peer, self.expected, self.f.evidence)
        self.assertFalse(select.select([child.stdout], [], [], 0)[0])

    def test_peer_log_refuses_restart_refusal_and_changed_grant(self):
        log = self.hint
        hold = {'connection_epoch': 2, 'content_grant_epoch': 3}
        h.peer_log(log, hold)
        for extra in (self.hint, b'gpu_colour_hold_refused schema=1 error=test\n',
                      b'sophia_live_shell_content content_grant_epoch=4\n',
                      b'sophia_live_shell_content connection_epoch=4\n'):
            with self.subTest(extra=extra), self.assertRaises(ValueError):
                h.peer_log(log + extra, hold)
        self.shell_path.chmod(0o666)
        with self.assertRaisesRegex(ValueError, 'unsafe shell endpoint'):
            h.shell_endpoint(self.f.log.read_bytes(), self.f.session)

    def test_capture_refusal_sends_no_ack(self):
        _,connection,peer,child=self.connect_peer()
        with patch.object(h,'run_capture',side_effect=ValueError('capture refused')) as capture:
            with self.assertRaisesRegex(ValueError,'capture refused'):
                h.drive(connection,peer,self.expected,self.f.evidence)
            self.assertEqual(capture.call_count,1)
        self.assertFalse(select.select([child.stdout],[],[],0)[0])

    def test_early_or_duplicate_ready_refuses_without_capture(self):
        _,connection,peer,_=self.connect_peer('early')
        with patch.object(h,'run_capture') as capture:
            with self.assertRaisesRegex(ValueError,'early hold records'):
                h.drive(connection,peer,self.expected,self.f.evidence)
            capture.assert_not_called()

    def test_real_helper_refusal_is_private_and_never_acks(self):
        _,connection,peer,child=self.connect_peer()
        # The fixture Session executable is Python, not Sophia: its exact launch
        # command refuses. Exercise the real helper/receipt path, without a GPU.
        with self.assertRaisesRegex(ValueError,'capture helper refused'):
            h.drive(connection,peer,self.expected,self.f.evidence)
        attempt=self.f.evidence/'1'
        self.assertTrue((attempt/'capture/refused.json').exists())
        self.assertTrue((attempt/'capture/launch.stderr').read_bytes())
        self.assertFalse(select.select([child.stdout],[],[],0)[0])

    def test_ready_exact_types_identity_and_order(self):
        value=dict(schema=1,status='ready',generation=1,pattern=1,connection_epoch=2,
                   content_grant_epoch=3,output=7,shell_output_generation=90,presentation_epoch=10,nonce='a'*32)
        self.assertEqual(h.ready(value,1,None)['generation'],1)
        for key,bad in [('schema',True),('schema',1.0),('generation',2),('pattern',2),('nonce','A'*32),('output',0)]:
            with self.subTest(key=key),self.assertRaises(ValueError): h.ready(dict(value,**{key:bad}),1,None)
        second=dict(value,generation=2,pattern=2,presentation_epoch=20,nonce='b'*32)
        h.ready(second,2,value)
        for key in ('connection_epoch','content_grant_epoch','output','shell_output_generation'):
            with self.subTest(key=key),self.assertRaises(ValueError): h.ready(dict(second,**{key:99}),2,value)
        for key in ('nonce','presentation_epoch'):
            with self.assertRaises(ValueError): h.ready(dict(second,**{key:value[key]}),2,value)

    def test_signal_refusal_unlinks_endpoint_before_owner_shutdown(self):
        expected=self.f.root/'driver-input.json'
        expected.write_text(json.dumps(self.expected))
        directory=h.c.private_directory(str(self.f.root))
        notification=h.c.watch(directory)
        child=subprocess.Popen([sys.executable,'-I','-B',str(ROOT/'gpu/hold_sophia.py'),
                                '--expected',str(expected),'--evidence',str(self.f.root/'driver')],
                               stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        try:
            deadline=time.monotonic()+3
            while not (self.f.root/'hold.sock').exists():
                self.assertTrue(select.select([notification],[],[],max(0,deadline-time.monotonic()))[0])
                os.read(notification,65536)
            child.send_signal(signal.SIGTERM)
            _,stderr=child.communicate(timeout=3)
            self.assertEqual(child.returncode,1,stderr.decode())
            self.assertFalse((self.f.root/'hold.sock').exists())
            self.assertFalse((self.f.root/'hold.pending').exists())
            self.assertTrue((self.f.root/'driver/refused.json').exists())
            self.f.session.check()
        finally:
            if child.poll() is None: child.kill()
            child.communicate(timeout=3)
            os.close(notification)
            os.close(directory)

    def test_wrapper_stops_owned_host_after_atomic_terminal(self):
        recipe=dict(schema=1,host=self.pin,peer=self.pin,supervisor=self.pin,
                    argv=['session','run'],output_generation=1)
        real=subprocess.Popen
        children=[]
        fake_driver=r'''
import json,os,select,sys
from pathlib import Path
root=Path(sys.argv[1]); expected=json.loads((root/'expected.json').read_text())
fd=os.pidfd_open(expected['session']['pid'])
d=root/'driver'; d.mkdir(mode=0o700)
p=d/'captures_verified.partial'; p.write_text(json.dumps(dict(status='captures_verified',records=[1,2,3])))
p.rename(d/'captures_verified.json')
assert select.select([fd],[],[],5)[0]
'''
        def popen(argv,**kwargs):
            if len(children)==0:
                command=[sys.executable,'-I','-B','-c','import signal; signal.pause()']
            else:
                command=[sys.executable,'-I','-B','-c',fake_driver,str(self.f.root/'wrapper')]
            child=real(command,**kwargs); children.append(child); return child
        with patch.object(w.subprocess,'Popen',side_effect=popen):
            result=w.run(recipe,self.f.root/'wrapper')
        self.assertEqual(result['status'],'driver_completed')
        self.assertEqual(result['host_exit'],-signal.SIGTERM)
        self.assertTrue(all(child.poll() is not None for child in children))
        with self.assertRaises(FileExistsError): w.run(recipe,self.f.root/'wrapper')
        self.assertFalse((self.f.root/'wrapper/refused.json').exists())


if __name__=='__main__': unittest.main()
