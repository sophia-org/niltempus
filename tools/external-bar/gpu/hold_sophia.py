"""Untimed same-namespace READY/capture/ACK driver. No native qualification claim."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import select
import signal
import socket
import stat
import struct
import subprocess
import sys
import time

spec = importlib.util.spec_from_file_location('capture', Path(__file__).with_name('capture_sophia.py'))
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)
r = c.r
SO_PEERPIDFD = 77  # Linux asm-generic/socket.h; no fallback to PID-only authority.
READY = {'schema', 'status', 'generation', 'pattern', 'connection_epoch',
         'content_grant_epoch', 'output', 'shell_output_generation', 'presentation_epoch', 'nonce'}


def pin_artifact(pin):
    if type(pin) is not dict or pin.keys() != {'executable', 'sha256'}:
        raise ValueError('invalid executable pin')
    fd = os.open(r.absolute(pin['executable']), os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    with os.fdopen(fd, 'rb') as file:
        info = os.fstat(file.fileno())
        if (not stat.S_ISREG(info.st_mode) or info.st_mode & 0o022
                or hashlib.file_digest(file, 'sha256').hexdigest() != r.digest(pin['sha256'])):
            raise ValueError('unsafe or mismatched executable')
        return info.st_dev, info.st_ino


def process(pid):
    fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    if fields[0] in ('Z', 'X'):
        raise ValueError('process exited')
    return int(fields[1]), int(fields[19])


class Peer:
    def __init__(self, connection, session, peer_pin, supervisor_pin):
        self.fd = struct.unpack('i', connection.getsockopt(socket.SOL_SOCKET, SO_PEERPIDFD, 4))[0]
        self.supervisor_fd = None
        try:
            os.set_inheritable(self.fd, False)
            self.pid, uid, _ = struct.unpack('3i', connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
            pid_line = [line for line in Path(f'/proc/self/fdinfo/{self.fd}').read_text().splitlines() if line.startswith('Pid:')]
            if uid != os.getuid() or self.pid <= 0 or pid_line != [f'Pid:\t{self.pid}']:
                raise ValueError('socket pidfd/credentials disagree')
            self.supervisor, self.starttime = process(self.pid)
            self.supervisor_fd = os.pidfd_open(self.supervisor)
            _, self.supervisor_starttime = process(self.supervisor)
            self.peer_identity = pin_artifact(peer_pin)
            self.supervisor_identity = pin_artifact(supervisor_pin)
            self.session = session
            self.check()
        except BaseException:
            self.close()
            raise

    def check(self):
        self.session.check()
        if select.select([self.fd, self.supervisor_fd], [], [], 0)[0]:
            raise ValueError('peer or supervisor exited')
        if (process(self.pid) != (self.supervisor, self.starttime)
                or process(self.supervisor) != (self.session.pid, self.supervisor_starttime)):
            raise ValueError('peer ancestry/starttime changed')
        children = Path(f'/proc/{self.supervisor}/task/{self.supervisor}/children').read_text().split()
        if children != [str(self.pid)]:
            raise ValueError('supervisor does not have exactly this child')
        for pid, identity in ((self.pid, self.peer_identity), (self.supervisor, self.supervisor_identity)):
            info = os.stat(f'/proc/{pid}/exe')
            if (info.st_dev, info.st_ino) != identity:
                raise ValueError('peer or supervisor executable changed')

    def close(self):
        os.close(self.fd)
        if self.supervisor_fd is not None:
            os.close(self.supervisor_fd)


def shell_endpoint(log, session):
    hints = [line for line in log.decode().splitlines() if 'gpu_colour_hold ' in line]
    prefix = 'gpu_colour_hold schema=1 status=endpoint_hint shell_socket='
    if len(hints) != 1 or not hints[0].startswith(prefix):
        raise ValueError('need one diagnostic peer hint')
    owned = set()
    for fd in Path(f'/proc/{session.pid}/fd').iterdir():
        try:
            owned.add(os.readlink(fd))
        except FileNotFoundError:
            pass
    rows = [line.split(maxsplit=7) for line in Path(f'/proc/{session.pid}/net/unix').read_text().splitlines()[1:]]
    paths = [row[7] for row in rows if len(row) == 8 and row[3:6] == ['00010000', '0001', '01']
             and Path(row[7]).name == 'shell.sock' and f'socket:[{row[6]}]' in owned]
    if len(paths) != 1 or hints[0][len(prefix):] != paths[0]:
        raise ValueError('hint does not identify the sole Session shell listener')
    endpoint = Path(r.absolute(paths[0]))
    info = endpoint.lstat()
    if not stat.S_ISSOCK(info.st_mode) or stat.S_IMODE(info.st_mode) != 0o600 or info.st_uid != os.getuid():
        raise ValueError('unsafe shell endpoint')
    return endpoint


class Listener:
    def __init__(self, endpoint):
        self.directory = c.private_directory(str(endpoint.parent))
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.names = []
        self.identity = None
        try:
            # Refuse reuse; use dirfd paths for both bind and atomic publication.
            if any(name in os.listdir(self.directory) for name in ('hold.sock', 'hold.pending')):
                raise ValueError('hold endpoint already exists')
            private = f'/proc/self/fd/{self.directory}/hold.pending'
            self.socket.bind(private)
            self.names.append('hold.pending')
            info = os.stat('hold.pending', dir_fd=self.directory, follow_symlinks=False)
            self.identity = info.st_dev, info.st_ino
            os.chmod('hold.pending', 0o600, dir_fd=self.directory)
            self.socket.listen(1)
            # renameat2 NOREPLACE: a concurrent creator is a refusal, never overwritten.
            libc = c.ctypes.CDLL(None, use_errno=True)
            if libc.renameat2(self.directory, b'hold.pending', self.directory, b'hold.sock', 1) != 0:
                raise OSError(c.ctypes.get_errno(), 'publish hold listener')
            self.names = ['hold.sock']
            self.socket.setblocking(False)
        except BaseException:
            self.close()
            raise

    def close(self):
        self.socket.close()
        try:
            for name in self.names:
                info = os.stat(name, dir_fd=self.directory, follow_symlinks=False)
                if (info.st_dev, info.st_ino) != self.identity:
                    raise ValueError('hold endpoint replaced; refusing foreign unlink')
                os.unlink(name, dir_fd=self.directory)
        finally:
            os.close(self.directory)


def peer_log(log, hold=None):
    lines = log.decode().splitlines()
    hints = [line for line in lines if 'gpu_colour_hold ' in line]
    if len(hints) != 1 or not hints[0].startswith('gpu_colour_hold schema=1 status=endpoint_hint shell_socket='):
        raise ValueError('diagnostic peer restarted or hint changed')
    if any('gpu_colour_hold_refused' in line for line in lines):
        raise ValueError('diagnostic peer refused')
    if hold:
        for line in lines:
            if line.startswith('sophia_live_shell_content '):
                for key in ('connection_epoch', 'content_grant_epoch'):
                    values = [part[len(key)+1:] for part in line.split() if part.startswith(key + '=')]
                    if values and values != [str(hold[key])]:
                        raise ValueError('diagnostic peer grant changed')


def wait(fds, session, deadline):
    session.check()
    left = deadline - time.monotonic()
    if left <= 0:
        raise ValueError('hold driver deadline')
    ready = select.select([*fds, session.fd], [], [], left)[0]
    session.check()
    if not ready:
        raise ValueError('hold driver deadline')
    return ready


def line(connection, peer, deadline):
    data = bytearray()
    while True:
        peer.check()
        wait([connection, peer.fd], peer.session, deadline)
        peer.check()
        try:
            part = connection.recv(1025)
        except BlockingIOError:
            continue
        if not part:
            raise ValueError('peer EOF before sequence completion')
        data.extend(part)
        if len(data) > 1024:
            raise ValueError('hold record too large')
        if b'\n' in data:
            if data.index(b'\n') != len(data) - 1:
                raise ValueError('multiple or early hold records')
            return r.record(data)


def ready(value, generation, previous):
    if type(value) is not dict or value.keys() != READY:
        raise ValueError('unexpected READY fields')
    for key in READY - {'status', 'nonce'}:
        r.uint(value[key], True)
    if (value['schema'] != 1 or value['status'] != 'ready' or value['generation'] != generation
            or value['pattern'] != (1, 2, 1)[generation - 1]
            or type(value['nonce']) is not str or re.fullmatch('[0-9a-f]{32}', value['nonce']) is None):
        raise ValueError('invalid or stale READY')
    if previous and (any(value[key] != previous[key] for key in
                        ('connection_epoch', 'content_grant_epoch', 'output', 'shell_output_generation'))
                     or value['presentation_epoch'] <= previous['presentation_epoch']
                     or value['nonce'] == previous['nonce']):
        raise ValueError('READY grant/output changed or epoch/challenge did not advance')
    return {key: value[key] for key in r.HOLD}


def run_capture(expected, evidence, connection, peer):
    path = evidence / 'expected.json'
    path.write_text(json.dumps(expected) + '\n')
    output = evidence / 'capture'
    command = [sys.executable, '-I', '-B', str(Path(__file__).with_name('capture_sophia.py')),
               '--expected', str(path), '--evidence', str(output)]
    child = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
    fd = None
    buffers = {child.stdout.fileno(): bytearray(), child.stderr.fileno(): bytearray()}
    opened = set(buffers)
    deadline = time.monotonic() + 77
    try:
        fd = os.pidfd_open(child.pid)
        for pipe in opened:
            os.set_blocking(pipe, False)
        exited = False
        while opened or not exited:
            peer.check()
            events = wait([*opened, *([] if exited else [fd]), connection, peer.fd], peer.session, deadline)
            if connection in events or peer.fd in events:
                raise ValueError('peer sent data or exited before capture ACK')
            for item in events:
                if item == fd:
                    exited = True
                elif item in opened:
                    part = os.read(item, 4097)
                    buffers[item].extend(part)
                    if len(buffers[item]) > 4096:
                        raise ValueError('capture helper output exceeds bound')
                    if not part:
                        opened.remove(item)
        if child.wait() != 0 or bytes(buffers[child.stdout.fileno()]) != b'capture_verified\n' or buffers[child.stderr.fileno()]:
            raise ValueError('capture helper refused; no ACK')
        with (output / 'result.json').open('rb') as file:
            data = file.read(65537)
        if len(data) > 65536 or (output / 'refused.json').exists():
            raise ValueError('invalid helper terminal files')
        result = r.record(data)
        if result['status'] != 'capture_verified' or result['hold'] != expected['hold']:
            raise ValueError('capture result does not name this hold')
        return hashlib.sha256(data).hexdigest()
    finally:
        if child.returncode is None:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait(timeout=2)
        try:
            for name, pipe in (('helper.stdout', child.stdout), ('helper.stderr', child.stderr)):
                (evidence / name).write_bytes(bytes(buffers[pipe.fileno()])[:4097])
        finally:
            if fd is not None:
                os.close(fd)
            child.stdout.close()
            child.stderr.close()


def drive(connection, peer, expected, evidence):
    previous = None
    records = []
    for generation in (1, 2, 3):
        value = line(connection, peer, time.monotonic() + 30)
        hold = ready(value, generation, previous)
        if any(record['ready']['nonce'] == value['nonce'] for record in records):
            raise ValueError('reused READY challenge')
        log, _ = c.log_bytes(expected['session_log'])
        if value['output'] != int(r.output_mapping(log)['output']):
            raise ValueError('READY names a different output')
        peer_log(log, hold)
        r.held(log, hold, value['output'])
        capture = {key: expected[key] for key in
                   ('schema', 'session', 'session_log', 'delivery_directory', 'output_generation')}
        capture['hold'] = hold
        attempt = evidence / str(generation)
        attempt.mkdir(mode=0o700)
        result_hash = run_capture(capture, attempt, connection, peer)
        peer.check()
        if select.select([connection], [], [], 0)[0]:
            raise ValueError('peer advanced or closed before ACK')
        log, _ = c.log_bytes(expected['session_log'])
        peer_log(log, hold)
        r.held(log, hold, value['output'])
        ack = {'schema': 1, 'status': 'ack', 'generation': generation, 'nonce': value['nonce']}
        data = (json.dumps(ack, separators=(',', ':')) + '\n').encode()
        if connection.send(data) != len(data):
            raise ValueError('partial ACK; do not retry')
        records.append({'ready': value, 'capture_result_sha256': result_hash, 'ack': ack})
        previous = value
    done = line(connection, peer, time.monotonic() + 5)
    if type(done) is not dict or done != {'schema': 1, 'status': 'done', 'generation': 3} or any(type(done[key]) is not int for key in ('schema', 'generation')):
        raise ValueError('missing exact DONE')
    log, _ = c.log_bytes(expected['session_log'])
    peer_log(log, previous)
    return {'schema': 1, 'status': 'captures_verified', 'records': records,
            'capture_output_incarnation': expected['output_generation'],
            'peer_pid': peer.pid, 'peer_starttime': peer.starttime,
            'limits': 'Same-user custody and pinned binaries; peer checks controller UID only. '
            'READY.shell_output_generation is ContentOutputId.generation; capture_output_incarnation is '
            'a separately declared OutputCaptureFrameInfo incarnation, verified by the capture helper. '
            'The head log supplies no native incarnation; equality of these domains is not assumed. '
            'Sophia captures its imported copy, not ongoing producer-buffer scanout. '
            'No native output/cleanup/performance qualification.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--expected', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    def interrupted(_signal, _frame):
        raise InterruptedError('wrapper stopped the hold driver')
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    os.umask(0o077)
    evidence = Path(r.absolute(str(args.evidence)))
    evidence.mkdir(mode=0o700)
    session = listener = peer = connection = None
    notification = directory = None
    try:
        with args.expected.open('rb') as file:
            data = file.read(16385)
        if len(data) > 16384:
            raise ValueError('driver input too large')
        expected = r.record(data)
        if (type(expected) is not dict or expected.keys() != {'schema', 'session', 'session_log',
                'delivery_directory', 'output_generation', 'peer', 'supervisor'}
                or type(expected['schema']) is not int or expected['schema'] != 1):
            raise ValueError('invalid driver input')
        r.uint(expected['output_generation'], True)
        pin_artifact(expected['peer'])
        pin_artifact(expected['supervisor'])
        session = c.Session(expected['session'])
        directory = c.private_directory(str(Path(expected['session_log']).parent))
        notification = c.watch(directory)
        deadline = time.monotonic() + 30
        while True:
            try:
                log, _ = c.log_bytes(expected['session_log'])
            except ValueError as error:
                if str(error) != 'no bounded complete Session log prefix' or Path(expected['session_log']).stat().st_size > c.LIMIT:
                    raise
                log = b''
            if b'gpu_colour_hold ' in log:
                endpoint = shell_endpoint(log, session)
                break
            wait([notification], session, deadline)
            try:
                os.read(notification, 65536)
            except BlockingIOError:
                pass
        listener = Listener(endpoint)
        wait([listener.socket], session, deadline)
        connection, _ = listener.socket.accept()
        connection.setblocking(False)
        peer = Peer(connection, session, expected['peer'], expected['supervisor'])
        result = drive(connection, peer, expected, evidence)
        listener.close()  # Before the wrapper stops Session or its endpoint directory.
        listener = None
        terminal = evidence / 'captures_verified.partial'
        with terminal.open('x') as file:
            file.write(json.dumps(result, indent=2) + '\n')
            file.flush()
            os.fsync(file.fileno())
        terminal.rename(evidence / 'captures_verified.json')
        # The wrapper owns shutdown. Keep the peer channel alive until Session exits.
        deadline = time.monotonic() + 30
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                raise ValueError('wrapper did not stop Session after captures')
            events = select.select([session.fd, notification], [], [], left)[0]
            if notification in events:
                os.read(notification, 65536)
            log, _ = c.log_bytes(expected['session_log'])
            peer_log(log, result['records'][-1]['ready'])
            if session.fd in events:
                break
        (evidence / 'result.json').write_text(json.dumps({'schema': 1, 'status': 'session_exited_after_captures'}) + '\n')
    except (OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError) as error:
        (evidence / 'refused.json').write_text(json.dumps({'schema': 1, 'status': 'refused', 'reason': str(error), 'retry': False}) + '\n')
        raise SystemExit(1)
    finally:
        if listener is not None:
            listener.close()
        if connection is not None:
            connection.close()
        if peer is not None:
            peer.close()
        if notification is not None:
            os.close(notification)
        if directory is not None:
            os.close(directory)
        if session is not None:
            session.close()


if __name__ == '__main__':
    main()
