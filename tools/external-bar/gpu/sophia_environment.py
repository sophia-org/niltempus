"""Startup-only mode proof through xtask's credential + live TGID-pidfd receiver.

No ancillary decoding here. The reviewed receiver owns that boundary; its
private stdout is evidence only when bound to the pinned, supervised process.
"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time
from urllib.parse import quote_from_bytes, unquote_to_bytes

spec = importlib.util.spec_from_file_location('environment_external', Path(__file__).with_name('run_colour_external.py'))
external = importlib.util.module_from_spec(spec)
spec.loader.exec_module(external)
h = external.h
READY = b'{"schema":1,"status":"ready"}\n'
LIMIT = 3 * 65536
DURATION_MS = 15000


def number(text):
    if type(text) is not str or re.fullmatch(r'0|[1-9][0-9]*', text) is None:
        raise ValueError('noncanonical environment integer')
    return h.r.uint(int(text))


def decode(text):
    raw = unquote_to_bytes(text)
    if (not raw or quote_from_bytes(raw, safe='-._~') != text
            or any(b < 32 or b == 127 for b in raw)):
        raise ValueError('noncanonical renderer identity')
    return raw.decode('utf-8')


def qualify(data, host, start_usec, end_usec):
    h.r.uint(host['pid'], True)
    h.r.uint(start_usec, True)
    h.r.uint(end_usec, True)
    if len(data) > LIMIT or not data.startswith(READY) or not data.endswith(b'\n'):
        raise ValueError('incomplete environment receiver output')
    rows = data[len(READY):].splitlines()
    if len(rows) != 2:
        raise ValueError('exactly one output and renderer readback required')
    observed = []
    for index, raw in enumerate(rows):
        receipt = h.r.record(raw)
        if type(receipt) is not dict or receipt.keys() != {'pid', 'uid', 'gid', 'received_usec', 'record', 'sender_fdinfo'}:
            raise ValueError('unexpected environment receipt keys')
        for key in ('pid', 'uid', 'gid', 'received_usec'):
            h.r.uint(receipt[key])
        if (receipt['pid'], receipt['uid'], receipt['gid']) != (host['pid'], os.getuid(), os.getgid()):
            raise ValueError('environment sender is not the supervised host')
        if type(receipt['sender_fdinfo']) is not str or type(receipt['record']) is not str:
            raise ValueError('invalid receiver evidence types')
        # This retained text is not an independent identity proof. xtask checked
        # the actual handle against SCM_CREDENTIALS and polled it before output.
        pids = re.findall(r'^Pid:\s*([0-9]+)$', receipt['sender_fdinfo'], re.MULTILINE)
        if pids != [str(host['pid'])]:
            raise ValueError('retained pidfd evidence differs from host')
        tokens = receipt['record'].split(' ')
        tag = 'output' if index == 0 else 'renderer'
        if tokens.pop(0) != 'sophia_shell_environment_' + tag:
            raise ValueError('unexpected environment record order')
        values = {}
        for token in tokens:
            if token.count('=') != 1:
                raise ValueError('malformed environment field')
            key, value = token.split('=')
            if not key or not value or key in values:
                raise ValueError('empty or duplicate environment field')
            values[key] = value
        common = {'schema', 'pid', 'output', 'card', 'monotonic_usec'}
        extra = ({'connector', 'width', 'height', 'refresh_millihz',
                  'timing_refresh_millihz', 'scale_milli', 'vrr'} if index == 0
                 else {'driver', 'vendor', 'renderer', 'version'})
        if values.keys() != common | extra:
            raise ValueError('unexpected environment fields')
        for key in ('schema', 'pid', 'output', 'monotonic_usec'):
            number(values[key])
        if (values['schema'], values['pid'], values['output'], values['card']) != ('1', str(host['pid']), '1', 'card0'):
            raise ValueError('wrong environment output identity')
        stamp = number(values['monotonic_usec'])
        if not start_usec <= stamp <= receipt['received_usec'] <= end_usec:
            raise ValueError('environment readback is outside the live host interval')
        if index == 0:
            expected = dict(connector='DP-1', width='1920', height='1080', refresh_millihz='120000',
                            timing_refresh_millihz='119880', scale_milli='1000', vrr='off')
            if any(values[k] != v for k, v in expected.items()):
                raise ValueError('actual Sophia mode differs from the matched GPU recipe')
        else:
            if values['driver'] != 'amdgpu' or values['monotonic_usec'] != observed[0]['monotonic_usec']:
                raise ValueError('renderer and output are not one amdgpu readback')
            for key in ('vendor', 'renderer', 'version'):
                values[key] = decode(values[key])
            identity = ' '.join(values[k] for k in ('vendor', 'renderer', 'version')).lower()
            if any(word in identity for word in ('llvmpipe', 'softpipe', 'swrast', 'software')):
                raise ValueError('software renderer cannot qualify GPU mode')
            if receipt['received_usec'] < first_received:
                raise ValueError('receiver clock regressed')
        first_received = receipt['received_usec']
        observed.append(values)
    return dict(output=observed[0], renderer=observed[1])


class Collector:
    def __init__(self, pin, root):
        self.pin, self.root = pin, root
        self.socket = root/'environment.sock'
        if len(os.fsencode(self.socket)) > 107:
            raise ValueError('environment socket pathname exceeds sun_path')
        self.child = None
        self.files = []
        try:
            for name in ('environment.ndjson', 'environment.stderr'):
                self.files.append((root/name).open('xb'))
            self.argv = [pin['executable'], 'shell-timing', 'receive-environment',
                         str(self.socket), str(DURATION_MS), '--ready']
            self.child = external.spawn(pin, self.argv, stdin=subprocess.DEVNULL,
                stdout=self.files[0], stderr=self.files[1], start_new_session=True)
            self.identity = dict(pin, pid=self.child.pid, starttime=h.c.snapshot(self.child.pid))
            with_host = h.c.Session(self.identity)
            try:
                until = time.monotonic() + 5
                while True:
                    with_host.check()
                    data = self.read()
                    if data == READY:
                        break
                    if not READY.startswith(data) or self.child.poll() is not None or time.monotonic() >= until:
                        raise ValueError('environment receiver did not become ready')
                    time.sleep(.01)
            finally:
                with_host.close()
        except BaseException:
            self.close()
            raise

    def read(self):
        if external.read(self.root/'environment.stderr', 4096):
            raise ValueError('environment receiver stderr is nonempty')
        return external.read(self.root/'environment.ndjson', LIMIT)

    def finish(self, host, start_usec):
        bound = h.c.Session(host)
        try:
            until = time.monotonic() + DURATION_MS/1000 + 2
            while self.child.poll() is None:
                bound.check()
                self.read()
                if time.monotonic() >= until:
                    raise ValueError('environment receiver deadline')
                time.sleep(.05)
            bound.check()
            end_usec = time.monotonic_ns()//1000
            if self.child.returncode != 0:
                raise ValueError('environment receiver refused')
            data = self.read()
            observed = qualify(data, host, start_usec, end_usec)
            external.drm_layout()
            bound.check()
            result = dict(schema=1, status='startup_environment_verified', host=host,
                receiver=self.identity, receiver_argv=self.argv, receiver_exit=0,
                start_usec=start_usec, end_usec=end_usec, **observed,
                input_sha256=hashlib.sha256(data).hexdigest(),
                limits='One startup KMS/GL readback, not repeated per held frame. '
                       'Credentials and live TGID pidfd checked by the pinned receiver. '
                       'card0 PCI 0000:03:00.0 checked through sysfs; capture incarnation is a separate domain.')
            external.write(self.root/'environment.json', result)
            return result
        finally:
            bound.close()

    def close(self):
        if self.child is not None and self.child.poll() is None:
            os.killpg(self.child.pid, signal.SIGTERM)
            try:
                self.child.wait(timeout=2)
            except subprocess.TimeoutExpired:
                os.killpg(self.child.pid, signal.SIGKILL)
                self.child.wait(timeout=2)
        for file in self.files:
            file.close()
