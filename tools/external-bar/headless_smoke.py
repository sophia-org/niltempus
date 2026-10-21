#!/usr/bin/env python3
"""Device-hidden Wayland protocol smoke; never native performance evidence."""
import argparse
import os
from pathlib import Path
import signal
import subprocess
import time


def inside(args):
    runtime = Path('/tmp/runtime')
    env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), WLR_BACKENDS='headless',
               WLR_HEADLESS_OUTPUTS='1', WLR_RENDERER='pixman', WLR_LIBINPUT_NO_DEVICES='1')
    for key in ('DISPLAY', 'WAYLAND_DISPLAY', 'DBUS_SESSION_BUS_ADDRESS'):
        env.pop(key, None)
    with (runtime/'compositor.log').open('w') as log:
        server = subprocess.Popen([args.compositor, '-no-xwayland', '-c', 'true'], env=env,
                                  stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            deadline = time.monotonic() + 8
            sockets = []
            while time.monotonic() < deadline and server.poll() is None:
                sockets = [p for p in runtime.glob('wayland-*') if p.is_socket()]
                if sockets:
                    break
                time.sleep(.05)
            if len(sockets) != 1:
                raise RuntimeError('headless compositor did not expose exactly one socket')
            env['WAYLAND_DISPLAY'] = sockets[0].name
            with (runtime/'client.log').open('w') as client_log:
                result = subprocess.run([args.client, str(runtime/'journal.tsv'), '--frames', '8',
                                         '--warmup', '2', '--buffers', '2', '--lifecycle', args.lifecycle], env=env,
                                        stdout=client_log, stderr=subprocess.STDOUT, timeout=12)
            return result.returncode
        finally:
            if server.poll() is None:
                os.killpg(server.pid, signal.SIGTERM)
                try:
                    server.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    os.killpg(server.pid, signal.SIGKILL)
                    server.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compositor', required=True)
    parser.add_argument('--client', required=True)
    parser.add_argument('--out', type=Path)
    parser.add_argument('--inside', action='store_true')
    parser.add_argument('--lifecycle', choices=('fresh', 'reuse'), default='fresh')
    args = parser.parse_args()
    if args.inside:
        return inside(args)
    if args.out is None:
        parser.error('--out is required')
    args.out.mkdir(parents=True, exist_ok=False, mode=0o700)
    # Private /dev has no DRM/input nodes, /run has no session sockets, all
    # namespaces are isolated. Only this fresh directory is writable.
    command = ['bwrap', '--unshare-all', '--die-with-parent', '--ro-bind', '/', '/',
               '--dev', '/dev', '--proc', '/proc', '--tmpfs', '/tmp', '--tmpfs', '/run',
               '--bind', str(args.out.resolve()), '/tmp/runtime', '--chdir', '/tmp/runtime',
               'python3', str(Path(__file__).resolve()), '--inside',
               '--compositor', str(Path(args.compositor).resolve()),
               '--client', str(Path(args.client).resolve()), '--lifecycle', args.lifecycle]
    return subprocess.run(command, timeout=30).returncode


if __name__ == '__main__':
    raise SystemExit(main())
