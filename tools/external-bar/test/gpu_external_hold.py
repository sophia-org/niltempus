"""Real C socket holds, synthetic event dispatch and pixels; no native renderer."""
import importlib.util
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]

def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module

c = load('control', ROOT/'gpu/colour_control.py')
f = load('fixture', ROOT/'test/gpu_pixels.py')

class Hold(unittest.TestCase):
    def start(self, frames='3'):
        parent, child = socket.socketpair()
        read, write = os.pipe()
        process = subprocess.Popen([str(ROOT/'build/gpu-hold-client-test'),str(child.fileno()),str(read),frames],
                                   pass_fds=(child.fileno(),read),stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        child.close(); os.close(read)
        pidfd = os.pidfd_open(process.pid)
        def cleanup():
            parent.close(); os.close(write); os.close(pidfd)
            if process.poll() is None:
                process.kill()
            process.communicate(timeout=3)
        self.addCleanup(cleanup)
        return parent,write,process,pidfd

    def test_three_holds_service_events_before_ack_and_pixels(self):
        parent,write,process,pidfd=self.start()
        def check():
            if process.poll() is not None: raise ValueError('fixture exited')
        def capture(generation,pattern):
            os.write(write,b'R')
            self.assertTrue(select.select([process.stdout],[],[],2)[0])
            self.assertEqual(process.stdout.readline(),f'dispatch={generation}\n'.encode())
            self.assertFalse(select.select([parent],[],[],0)[0])
            return f.image(pattern)
        result=c.run(parent,pidfd,capture,check)
        self.assertEqual(result['status'],'pixel_sequence_verified')
        self.assertEqual([r['ready']['pattern'] for r in result['records']],[1,2,1])
        self.assertEqual(process.wait(timeout=2),0)

    def test_wrong_pixels_never_ack(self):
        parent,_,process,pidfd=self.start()
        with self.assertRaises(ValueError): c.run(parent,pidfd,lambda *_:f.image(2),lambda:None)
        self.assertIsNone(process.poll())
        self.assertFalse(select.select([parent],[],[],0)[0])

    def test_stale_or_extra_ack_refuses(self):
        for mode in ('nonce','generation','extra'):
            with self.subTest(mode=mode):
                parent,_,process,pidfd=self.start()
                value=c.receive(parent,pidfd,time.monotonic()+2)
                ack=dict(schema=1,status='ack',generation=1,nonce=value['nonce'])
                if mode=='nonce': ack['nonce']='0'*32
                if mode=='generation': ack['generation']=2
                if mode=='extra': ack['extra']=1
                parent.sendall((json.dumps(ack,separators=(',',':'))+'\n').encode())
                self.assertEqual(process.wait(timeout=2),2)

    def test_partial_ack_does_not_advance(self):
        parent,_,process,pidfd=self.start()
        value=c.receive(parent,pidfd,time.monotonic()+2)
        ack=json.dumps(dict(schema=1,status='ack',generation=1,nonce=value['nonce']),separators=(',',':')).encode()+b'\n'
        parent.sendall(ack[:-1])
        self.assertFalse(select.select([parent],[],[],0.02)[0])
        parent.sendall(ack[-1:])
        self.assertEqual(c.receive(parent,pidfd,time.monotonic()+2)['generation'],2)

    def test_bad_config_and_eof_refuse(self):
        _,_,process,_=self.start('4')
        self.assertEqual(process.wait(timeout=2),2)
        parent,_,process,pidfd=self.start()
        c.receive(parent,pidfd,time.monotonic()+2)
        parent.shutdown(socket.SHUT_RDWR)
        self.assertEqual(process.wait(timeout=2),2)

    def test_ready_parser_refuses_aliases_stale_and_unknown(self):
        value=dict(schema=1,status='ready',generation=1,pattern=1,presented_ns=100,nonce='a'*32)
        c.ready(value,1,0,set())
        for key,bad in [('schema',True),('schema',1.0),('generation',2),('pattern',2),('presented_ns',0),('nonce','A'*32),('extra',1)]:
            with self.subTest(key=key),self.assertRaises(ValueError): c.ready(dict(value,**{key:bad}),1,0,set())
        with self.assertRaises(ValueError): c.ready(value,1,0,{'a'*32})
        with self.assertRaises(ValueError): c.ready(value,1,100,set())

    def test_diagnostic_clients_refuse_before_display_or_gpu(self):
        for name,base in [('wl-panel-gpu-hold',['/dev/null','--output','DP-1']),
                          ('x-panel-gpu-hold',['--journal','/dev/null'])]:
            with self.subTest(name=name):
                parent,child=socket.socketpair()
                try:
                    command=[str(ROOT/'build'/name),*base,'--frames','4','--warmup','0',
                             '--buffers','2','--lifecycle','fresh','--hold-fd',str(child.fileno())]
                    result=subprocess.run(command,pass_fds=(child.fileno(),),capture_output=True,timeout=3,
                                          env={**os.environ,'DISPLAY':'invalid','WAYLAND_DISPLAY':'/absent'})
                    self.assertEqual(result.returncode,2)
                    self.assertIn(b'requires three frames',result.stderr)
                finally: parent.close();child.close()

if __name__=='__main__': unittest.main()
