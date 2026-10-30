"""Explicit untimed wrapper: sibling Session and hold driver, inside the PAM namespace.

This is not a sealed native recipe. A package must supply reviewed host argv,
profile, feature/build pins, actual device qualification and outer PAM cleanup.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import time

spec = importlib.util.spec_from_file_location('hold', Path(__file__).with_name('hold_sophia.py'))
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)


def validate(value):
    if (type(value) is not dict or value.keys() != {'schema', 'host', 'argv', 'peer', 'supervisor', 'output_generation'}
            or type(value['schema']) is not int or value['schema'] != 1):
        raise ValueError('invalid wrapper recipe')
    h.r.uint(value['output_generation'], True)
    for name in ('host', 'peer', 'supervisor'):
        h.pin_artifact(value[name])
    argv = value['argv']
    if (type(argv) is not list or argv[:2] != ['session', 'run'] or len(argv) > 64
            or any(type(arg) is not str or not arg or len(arg) > 4096
                   or any(ord(ch) < 32 or ord(ch) == 127 for ch in arg) for arg in argv)
            or any(arg.split('=')[0] == '--diagnostic-capture-directory' for arg in argv)):
        raise ValueError('wrapper requires bounded session run argv without a capture destination')


def stop(child):
    if child is None or child.poll() is not None:
        return False
    child.send_signal(signal.SIGTERM)
    try:
        child.wait(timeout=10)
        return False
    except subprocess.TimeoutExpired:
        os.killpg(child.pid, signal.SIGKILL)
        child.wait(timeout=2)
        return True


def run(recipe, root):
    validate(recipe)
    root.mkdir(mode=0o700)
    session_log = root / 'session.log'
    host = driver = None
    host_forced = False
    host_fd = driver_fd = directory = notification = None
    files = []
    try:
        log = session_log.open('xb')
        files.append(log)
        command = [recipe['host']['executable'], *recipe['argv'],
                   f'--diagnostic-capture-directory={root / "captures"}']
        host = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True)
        host_fd = os.pidfd_open(host.pid)
        expected = {'schema': 1, 'session': dict(recipe['host'], pid=host.pid, starttime=h.c.snapshot(host.pid)),
                    'session_log': str(session_log), 'delivery_directory': str(root / 'captures'),
                    'output_generation': recipe['output_generation'], 'peer': recipe['peer'], 'supervisor': recipe['supervisor']}
        path = root / 'expected.json'
        path.write_text(json.dumps(expected, indent=2) + '\n')
        directory = h.c.private_directory(str(root))
        notification = h.c.watch(directory)
        stdout, stderr = (root / 'driver.stdout').open('xb'), (root / 'driver.stderr').open('xb')
        files.extend((stdout, stderr))
        driver = subprocess.Popen([sys.executable, '-I', '-B', str(Path(__file__).with_name('hold_sophia.py')),
                                   '--expected', str(path), '--evidence', str(root / 'driver')],
                                  stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr, start_new_session=True)
        driver_fd = os.pidfd_open(driver.pid)
        # Watch the driver's directory once created; outer watch catches creation.
        driver_directory = driver_notification = None
        deadline = time.monotonic() + 300
        try:
            while True:
                if (root / 'driver').exists() and driver_directory is None:
                    driver_directory = h.c.private_directory(str(root / 'driver'))
                    driver_notification = h.c.watch(driver_directory)
                if (root / 'driver/refused.json').exists():
                    raise ValueError('driver refused; no retry')
                if (root / 'driver/captures_verified.json').exists():
                    with (root / 'driver/captures_verified.json').open('rb') as file:
                        terminal = h.r.record(file.read(65537))
                    if terminal['status'] != 'captures_verified' or len(terminal['records']) != 3:
                        raise ValueError('invalid driver terminal result')
                    break
                if session_log.stat().st_size > h.c.LIMIT or any(p.stat().st_size > 4096 for p in (root / 'driver.stdout', root / 'driver.stderr')):
                    raise ValueError('wrapper log bound exceeded')
                left = deadline - time.monotonic()
                if left <= 0:
                    raise ValueError('wrapper deadline')
                events = select.select([host_fd, driver_fd, notification, *([] if driver_notification is None else [driver_notification])], [], [], left)[0]
                if not events or host_fd in events or driver_fd in events:
                    raise ValueError('Session/driver exited before captures or timed out')
                for fd in events:
                    os.read(fd, 65536)
        finally:
            if driver_notification is not None:
                os.close(driver_notification)
                os.close(driver_directory)
        host_forced = stop(host)
        if host_forced:
            raise ValueError('Session needed forced termination')
        if driver.wait(timeout=5) != 0 or (root / 'driver/refused.json').exists():
            raise ValueError('driver did not observe clean handoff to Session exit')
        result = {'schema': 1, 'status': 'driver_completed', 'host_exit': host.returncode,
                  'driver_exit': driver.returncode, 'host_argv': command,
                  'limits': 'Local child exit only; full PAM/cgroup/device cleanup is a separate native gate.'}
        (root / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        return result
    except (OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError) as error:
        with (root / 'refused.json').open('x') as file:
            file.write(json.dumps({'schema': 1, 'status': 'refused', 'reason': str(error), 'retry': False}) + '\n')
        raise
    finally:
        # Give the driver a chance to remove its endpoint before owner shutdown.
        driver_forced = False
        if driver is not None and driver.poll() is None:
            driver_forced = stop(driver)
        host_forced = stop(host) or host_forced
        (root / 'cleanup.json').write_text(json.dumps({'schema': 1,
            'driver_forced': driver_forced, 'host_forced': host_forced,
            'driver_exit': None if driver is None else driver.returncode,
            'host_exit': None if host is None else host.returncode,
            'scope': 'owned direct children only; no full PAM/device cleanup claim'}) + '\n')
        for fd in (notification, directory, driver_fd, host_fd):
            if fd is not None:
                os.close(fd)
        for file in files:
            file.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--recipe', required=True, type=Path)
    parser.add_argument('--evidence', required=True, type=Path)
    args = parser.parse_args()
    def interrupted(_signal, _frame):
        raise InterruptedError('wrapper interrupted')
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    os.umask(0o077)
    root = Path(h.r.absolute(str(args.evidence)))
    try:
        with args.recipe.open('rb') as file:
            data = file.read(32769)
        if len(data) > 32768:
            raise ValueError('wrapper recipe too large')
        run(h.r.record(data), root)
    except (OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
