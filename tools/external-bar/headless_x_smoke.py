#!/usr/bin/env python3
"""Device-hidden dummy X protocol smoke, never native performance evidence."""
import argparse
import os
from pathlib import Path
import signal
import subprocess
import time

CONFIG = '''Section "ServerFlags"
 Option "AutoAddDevices" "false"
 Option "AutoAddGPU" "false"
 Option "AutoEnableDevices" "false"
 Option "DontVTSwitch" "true"
EndSection
Section "Device"
 Identifier "dummy"
 Driver "dummy"
 VideoRam 16384
EndSection
Section "Monitor"
 Identifier "screen"
 HorizSync 20-100
 VertRefresh 20-100
 Modeline "1920x1080" 148.5 1920 2008 2052 2200 1080 1084 1089 1125
EndSection
Section "Screen"
 Identifier "screen"
 Device "dummy"
 Monitor "screen"
 DefaultDepth 24
 SubSection "Display"
  Depth 24
  Modes "1920x1080"
 EndSubSection
EndSection
'''


def stop(process):
    if process is not None and process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()


def inside(args):
    runtime = Path('/tmp/runtime')
    env = dict(os.environ, DISPLAY=':91', XDG_RUNTIME_DIR=str(runtime))
    for key in ('WAYLAND_DISPLAY', 'XAUTHORITY', 'DBUS_SESSION_BUS_ADDRESS'):
        env.pop(key, None)
    (runtime/'xorg.conf').write_text(CONFIG)
    server = compositor = None
    with (runtime/'server.log').open('w') as log, (runtime/'compositor.log').open('w') as comp_log:
        try:
            server = subprocess.Popen([
                args.server, ':91', '-config', str(runtime/'xorg.conf'),
                '-logfile', str(runtime/'Xorg.log'), '-nolisten', 'tcp',
                '-noreset', '-novtswitch', '-ac'], env=env,
                stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            deadline = time.monotonic() + 8
            while not Path('/tmp/.X11-unix/X91').is_socket():
                if server.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError('isolated dummy server did not become ready')
                time.sleep(.05)
            if args.compositor:
                compositor = subprocess.Popen([args.compositor, '-n'], env=env,
                    stdout=comp_log, stderr=subprocess.STDOUT, start_new_session=True)
                time.sleep(.3)
                if compositor.poll() is not None:
                    raise RuntimeError('test compositor exited before client')
            with (runtime/'client.log').open('w') as client_log:
                result = subprocess.run([
                    args.client, '--journal', str(runtime/'journal.tsv'),
                    '--frames', '1' if args.workload == 'idle' else '8', '--warmup', '0' if args.workload == 'idle' else '2', '--buffers', '2',
                    '--lifecycle', args.lifecycle, '--workload', args.workload,
                    '--idle-sec', str(args.idle_sec), '--interval-ms', '25'], env=env,
                    stdout=client_log, stderr=subprocess.STDOUT, timeout=max(12, args.idle_sec + 8))
            return result.returncode
        finally:
            stop(compositor)
            stop(server)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--server', required=True, help='unprivileged X server binary, not setuid wrapper')
    parser.add_argument('--client', required=True)
    parser.add_argument('--compositor')
    parser.add_argument('--out', type=Path)
    parser.add_argument('--inside', action='store_true')
    parser.add_argument('--lifecycle', choices=('fresh', 'reuse'), default='fresh')
    parser.add_argument('--workload', choices=('fresh', 'bursty', 'retained', 'idle'), default='fresh')
    parser.add_argument('--idle-sec', type=int, default=1, help='short protocol smoke, not the 60s gate')
    args = parser.parse_args()
    if args.inside:
        return inside(args)
    if args.out is None:
        parser.error('--out is required')
    args.out.mkdir(parents=True, exist_ok=False, mode=0o700)
    command = ['bwrap', '--unshare-all', '--die-with-parent', '--ro-bind', '/', '/',
               '--dev', '/dev', '--proc', '/proc', '--tmpfs', '/tmp', '--tmpfs', '/run',
               '--bind', str(args.out.resolve()), '/tmp/runtime', '--chdir', '/tmp/runtime',
               'python3', str(Path(__file__).resolve()), '--inside',
               '--server', str(Path(args.server).resolve()),
               '--client', str(Path(args.client).resolve()), '--lifecycle', args.lifecycle, '--workload', args.workload, '--idle-sec', str(args.idle_sec)]
    if args.compositor:
        command.extend(['--compositor', str(Path(args.compositor).resolve())])
    return subprocess.run(command, timeout=max(30, args.idle_sec + 20)).returncode


if __name__ == '__main__':
    raise SystemExit(main())
