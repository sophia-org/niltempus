"""One untimed, same-namespace Sophia capture of a declared held candidate.

Run as a sibling of Session inside its wrapper, never through nsenter. A parent
hold driver calls this once per READY (patterns 1,2,1), sends ACK only after exit
zero, and waits before stopping Session. This is not a native launch recipe.
"""
import argparse
import ctypes
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import select
import selectors
import signal
import stat
import subprocess
import time


def local(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


r, pixels = local('capture_receipt'), local('pixels')
DEADLINE = 75
LAUNCH_TIMEOUT = 35
RECEIPT_TIMEOUT = 30
LIMIT = 64 * 1024 * 1024


def snapshot(pid):
    text = Path(f'/proc/{pid}/stat').read_text()
    tail = text.rsplit(')', 1)[1].split()
    if tail[0] in ('Z', 'X'):
        raise ValueError('Session has exited')
    return int(tail[19])


def namespace_ids(pid):
    return {name: (os.stat(f'/proc/{pid}/ns/{name}').st_dev,
                   os.stat(f'/proc/{pid}/ns/{name}').st_ino) for name in ('user', 'mnt', 'pid')}


class Session:
    def __init__(self, expected):
        if type(expected) is not dict or expected.keys() != {'pid', 'starttime', 'executable', 'sha256'}:
            raise ValueError('invalid Session pin')
        self.pid = r.uint(expected['pid'], True)
        self.starttime = r.uint(expected['starttime'], True)
        self.executable = r.absolute(expected['executable'])
        self.sha256 = r.digest(expected['sha256'])
        self.fd = os.pidfd_open(self.pid)
        try:
            fd = os.open(self.executable, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
            with os.fdopen(fd, 'rb') as file:
                info = os.fstat(file.fileno())
                if not stat.S_ISREG(info.st_mode) or info.st_mode & 0o022:
                    raise ValueError('unsafe executable pin')
                self.identity = (info.st_dev, info.st_ino)
                if hashlib.file_digest(file, 'sha256').hexdigest() != self.sha256:
                    raise ValueError('Session executable hash mismatch')
            self.check()
        except BaseException:
            self.close()
            raise

    def check(self):
        if (select.select([self.fd], [], [], 0)[0] or snapshot(self.pid) != self.starttime
                or os.stat(f'/proc/{self.pid}').st_uid != os.getuid()
                or namespace_ids(self.pid) != namespace_ids('self')):
            raise ValueError('Session identity, user or namespaces differ')
        live = os.stat(f'/proc/{self.pid}/exe')
        path = os.stat(self.executable, follow_symlinks=False)
        if (live.st_dev, live.st_ino) != self.identity or (path.st_dev, path.st_ino) != self.identity:
            raise ValueError('Session executable identity changed')

    def close(self):
        os.close(self.fd)


def private_directory(path):
    fd = os.open(r.absolute(path), os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    info = os.fstat(fd)
    if info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700:
        os.close(fd)
        raise ValueError('capture directory must be owned and mode 0700')
    return fd


def log_bytes(path):
    parent, name = os.path.split(r.absolute(path))
    fd = os.open(parent, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        # Logs may grow; read a bounded complete prefix, then validate again after capture.
        child = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC, dir_fd=fd)
        with os.fdopen(child, 'rb') as file:
            info = os.fstat(file.fileno())
            if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid()
                    or stat.S_IMODE(info.st_mode) != 0o600 or info.st_nlink != 1 or info.st_size > LIMIT):
                raise ValueError('unsafe or oversized Session log')
            data = file.read(LIMIT + 1)
            if len(data) > LIMIT or b'\n' not in data:
                raise ValueError('no bounded complete Session log prefix')
            return data[:data.rfind(b'\n') + 1], (info.st_dev, info.st_ino)
    finally:
        os.close(fd)


def launch_socket(log, session):
    prefix = 'sophia_developer_launch schema=1 status=enabled admission=host-admin socket='
    lines = [line for line in log.decode('utf-8').splitlines()
             if 'sophia_developer_launch' in line and 'status=enabled' in line]
    if len(lines) != 1 or not lines[0].startswith(prefix):
        raise ValueError('expected one exact enabled launch endpoint')
    path = r.absolute(lines[0][len(prefix):])
    before = os.stat(path, follow_symlinks=False)
    if not stat.S_ISSOCK(before.st_mode) or before.st_uid != os.getuid():
        raise ValueError('invalid launch socket')
    parent = os.stat(os.path.dirname(path), follow_symlinks=False)
    if not stat.S_ISDIR(parent.st_mode) or parent.st_uid != os.getuid() or stat.S_IMODE(parent.st_mode) != 0o700:
        raise ValueError('launch socket directory is not private')
    rows = [line.split(maxsplit=7) for line in Path(f'/proc/{session.pid}/net/unix').read_text().splitlines()[1:]]
    matches = [row for row in rows if len(row) == 8 and row[7] == path
               and row[3:6] == ['00010000', '0001', '01']]
    if len(matches) != 1:
        raise ValueError('expected one stream listener for the launch path')
    inode = r.uint(int(matches[0][6]), True)
    found = False
    for entry in Path(f'/proc/{session.pid}/fd').iterdir():
        try:
            found |= os.readlink(entry) == f'socket:[{inode}]'
        except FileNotFoundError:
            pass
    if not found:
        raise ValueError('launch listener does not belong to the pinned Session')
    after = os.stat(path, follow_symlinks=False)
    if (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
        raise ValueError('launch socket changed')
    return path, (before.st_dev, before.st_ino)


def launch(argv, session, deadline, evidence):
    # Only this launch-client group is ours to kill. Session and its reaper retain
    # observer custody; an indeterminate client exit is never an automatic retry.
    child = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, start_new_session=True)
    output = {child.stdout.fileno(): bytearray(), child.stderr.fileno(): bytearray()}
    child_fd = None
    try:
        child_fd = os.pidfd_open(child.pid)
        with selectors.DefaultSelector() as selector:
            selector.register(session.fd, selectors.EVENT_READ, 'host')
            selector.register(child_fd, selectors.EVENT_READ, 'child')
            for fd in output:
                os.set_blocking(fd, False)
                selector.register(fd, selectors.EVENT_READ, 'pipe')
            opened = set(output)
            exited = False
            while opened or not exited:
                session.check()
                left = deadline - time.monotonic()
                if left <= 0:
                    raise ValueError('capture launch deadline; do not retry')
                events = selector.select(left)
                if not events:
                    raise ValueError('capture launch deadline; do not retry')
                for key, _ in events:
                    if key.data == 'host':
                        raise ValueError('Session exited during launch')
                    if key.data == 'child':
                        exited = True
                        selector.unregister(child_fd)
                        continue
                    data = os.read(key.fd, 4097)
                    if data:
                        output[key.fd].extend(data)
                        if len(output[key.fd]) > 4096:
                            raise ValueError('launch output exceeds bound')
                    else:
                        selector.unregister(key.fd)
                        opened.remove(key.fd)
            code = child.wait()
        stdout, stderr = (bytes(output[p.fileno()]) for p in (child.stdout, child.stderr))
        if code != 0 or stderr:
            raise ValueError('capture launch refused or indeterminate; do not retry')
        value = r.record(stdout)
        if (type(value) is not dict or value.keys() != {'version', 'request', 'status', 'process'}
                or type(value['version']) is not int or value['version'] != 1
                or type(value['request']) is not int or value['request'] != 1
                or value['status'] != 'started'):
            raise ValueError('capture launch did not report exact readiness')
        r.uint(value['process'], True)
        return value
    except BaseException:
        if child.returncode is None:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        child.wait(timeout=2)
        raise
    finally:
        try:
            for name, pipe in (('launch.stdout', child.stdout), ('launch.stderr', child.stderr)):
                with (evidence / name).open('xb') as file:
                    file.write(bytes(output[pipe.fileno()])[:4097])
        finally:
            if child_fd is not None:
                os.close(child_fd)
            child.stdout.close()
            child.stderr.close()


def watch(directory):
    libc = ctypes.CDLL(None, use_errno=True)
    libc.inotify_init1.argtypes, libc.inotify_init1.restype = [ctypes.c_int], ctypes.c_int
    libc.inotify_add_watch.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_uint32]
    libc.inotify_add_watch.restype = ctypes.c_int
    fd = libc.inotify_init1(os.O_NONBLOCK | os.O_CLOEXEC)
    if fd < 0:
        raise OSError(ctypes.get_errno(), 'inotify_init1')
    # Bind the already-open directory, not a path that could be replaced.
    if libc.inotify_add_watch(fd, f'/proc/self/fd/{directory}'.encode(), 0x100 | 0x80 | 0x8 | 0x400 | 0x800) < 0:
        os.close(fd)
        raise OSError(ctypes.get_errno(), 'inotify_add_watch')
    return fd


def capture(expected, session, directory, deadline, evidence):
    before_log, log_identity = log_bytes(expected['session_log'])
    mapping = r.output_mapping(before_log)
    expected = dict(expected, output=int(mapping['output']))
    r.held(before_log, expected['hold'], expected['output'])
    previous = r.inventory(directory)
    if (len(previous) != expected['hold']['generation'] - 1
            or any(names != {'pam', 'json', 'stderr'} for names in previous.values())):
        raise ValueError('attempt sequence is incomplete, reused or refused')
    previous_receipts = []
    for ordinal, generation in enumerate(sorted(previous), 1):
        prior = dict(expected, hold=dict(expected['hold'], pattern=(1, 2, 1)[ordinal - 1]))
        verified = r.verify(directory, generation, prior, session.identity, pixels.check)
        r.advance(previous_receipts[-1]['receipt'] if previous_receipts else None, verified['receipt'])
        previous_receipts.append(verified)
    endpoint, endpoint_identity = launch_socket(before_log, session)
    notification = watch(directory)
    try:
        argv = [session.executable, 'launch', '--socket', endpoint, '--recipe', 'capture', '--',
                session.executable, 'capture', str(expected['output']), '--diagnostic-stream']
        started = launch(argv, session, min(deadline, time.monotonic() + LAUNCH_TIMEOUT), evidence)
        deadline = min(deadline, time.monotonic() + RECEIPT_TIMEOUT)
        while True:
            session.check()
            groups = r.inventory(directory)
            if any(groups.get(key) != value for key, value in previous.items()):
                raise ValueError('prior delivery changed')
            fresh = groups.keys() - previous.keys()
            if len(fresh) > 1:
                raise ValueError('ambiguous capture attempts')
            if fresh:
                generation = next(iter(fresh))
                if generation <= max(previous, default=0) or 'refused' in groups[generation]:
                    raise ValueError('capture refused or stale')
                if 'json' in groups[generation]:
                    result = r.verify(directory, generation, expected, session.identity, pixels.check)
                    r.advance(previous_receipts[-1]['receipt'] if previous_receipts else None, result['receipt'])
                    after_log, after_identity = log_bytes(expected['session_log'])
                    if after_identity != log_identity or not after_log.startswith(before_log):
                        raise ValueError('Session log changed or was truncated')
                    r.held(after_log, expected['hold'], expected['output'])
                    if r.output_mapping(after_log) != mapping:
                        raise ValueError('output mapping changed')
                    if r.inventory(directory) != {**previous, generation: {'pam', 'json', 'stderr'}}:
                        raise ValueError('unexpected concurrent capture')
                    for prior in previous_receipts:
                        old = prior['receipt']['launch_generation']
                        if (hashlib.sha256(r.read_at(directory, f'capture-{old}.json', 4096)).hexdigest() != prior['receipt_sha256']
                                or hashlib.sha256(r.read_at(directory, f'capture-{old}.pam', 1920 * 1080 * 4 + 256)).hexdigest() != prior['receipt']['pam_sha256']
                                or r.read_at(directory, f'capture-{old}.stderr', 0) != b''):
                            raise ValueError('prior terminal receipt changed')
                    current = os.stat(endpoint, follow_symlinks=False)
                    if (current.st_dev, current.st_ino) != endpoint_identity:
                        raise ValueError('launch endpoint changed')
                    session.check()
                    return {'schema': 1, 'status': 'capture_verified', 'hold': expected['hold'],
                            'launch': started, 'delivery': result, 'output_mapping': mapping,
                            'launch_receipt_join': 'one serialized new directory receipt; transaction 1 is only a format check; no monitor/inner-PID equality',
                            'session_log_before_sha256': hashlib.sha256(before_log).hexdigest(),
                            'session_log_after_sha256': hashlib.sha256(after_log).hexdigest(),
                            'session': {'pid': session.pid, 'starttime': session.starttime,
                                        'namespaces': namespace_ids(session.pid)},
                            'limits': 'Custody-only private receipt plus owner-held candidate and pixels. '
                            'Monitor PID is not observer identity. No equality between capture frame and '
                            'shell presentation epoch; no native device, timing or release qualification.'}
            left = deadline - time.monotonic()
            if left <= 0 or not select.select([notification, session.fd], [], [], max(left, 0))[0]:
                raise ValueError('capture receipt deadline; do not retry')
            try:
                os.read(notification, 65536)
            except BlockingIOError:
                pass
    finally:
        os.close(notification)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--expected', required=True, type=Path)
    parser.add_argument('--evidence', required=True, type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    evidence = Path(r.absolute(str(args.evidence)))
    evidence.mkdir(mode=0o700)  # One attempt: no reuse, overwrite or retry.
    try:
        with args.expected.open('rb') as file:
            data = file.read(16385)
        if len(data) > 16384:
            raise ValueError('expected-input bound exceeded')
        expected = r.record(data)
        if (type(expected) is not dict or expected.keys() != {'schema', 'session', 'session_log',
                'delivery_directory', 'output_generation', 'hold'}
                or type(expected['schema']) is not int or expected['schema'] != 1):
            raise ValueError('unexpected controller input')
        r.uint(expected['output_generation'], True)
        (evidence / 'expected.json').write_bytes(data)
        session = Session(expected['session'])
        try:
            directory = private_directory(expected['delivery_directory'])
            try:
                result = capture(expected, session, directory, time.monotonic() + DEADLINE, evidence)
            finally:
                os.close(directory)
        finally:
            session.close()
        (evidence / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print('capture_verified')
    except (OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError) as error:
        (evidence / 'refused.json').write_text(json.dumps({'schema': 1, 'status': 'refused',
                                                      'reason': str(error), 'retry': False}) + '\n')
        raise SystemExit(1)


if __name__ == '__main__':
    main()
