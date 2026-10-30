"""Untimed native colour wrapper. Run only inside the attended PAM/VT handoff.

Owns one compositor and one colour client; no sampler, retries or fallback.
Package/source/build pins and outer seat/cgroup cleanup are separate gates.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import select
import signal
import socket
import stat
import struct
import subprocess
import sys
import time

import importlib.util
spec = importlib.util.spec_from_file_location('colour', Path(__file__).with_name('colour_control.py'))
colour = importlib.util.module_from_spec(spec)
spec.loader.exec_module(colour)
h = colour.load('hold_sophia')
outputs = colour.load('external_output')
journal = colour.load('qualify')
stop = colour.load('run_hold_sophia').stop
MAX_LOG = 64 * 1024 * 1024
PAM_LIMIT = 1920 * 1080 * 4 + 1024


def write(path, value):
    with path.open('x') as file:
        json.dump(value, file, indent=2)
        file.write('\n')


def read(path, limit):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    with os.fdopen(fd, 'rb') as file:
        info = os.fstat(file.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_size > limit:
            raise ValueError('capture/log is not a bounded regular file')
        data = file.read(limit+1)
        if len(data) > limit:
            raise ValueError('capture/log limit')
        return data


def validate(recipe):
    if (type(recipe) is not dict or recipe.keys() != {'schema', 'stack', 'pins', 'display'}
            or type(recipe['schema']) is not int or recipe['schema'] != 1
            or recipe['stack'] not in ('niri', 'xlibre') or type(recipe['pins']) is not dict):
        raise ValueError('external recipe fields')
    names = {'server', 'client', 'capture', 'output', 'config', 'bridge'}
    if recipe['stack'] == 'xlibre':
        names.add('xauth')
        if type(recipe['display']) is not int or not 1 <= recipe['display'] <= 99:
            raise ValueError('private X display number')
    elif recipe['display'] is not None or recipe['pins'].get('output') != recipe['pins'].get('server'):
        raise ValueError('niri output command must be the pinned server binary')
    if recipe['pins'].keys() != names:
        raise ValueError('external artifact pin set')
    for pin in recipe['pins'].values():
        h.pin_artifact(pin)
    config = 'niri.kdl' if recipe['stack'] == 'niri' else 'xorg.conf'
    expected = hashlib.sha256(Path(__file__).with_name(config).read_bytes()).hexdigest()
    if recipe['pins']['config']['sha256'] != expected:
        raise ValueError('unreviewed native output/device configuration')


def child_limits():
    resource.setrlimit(resource.RLIMIT_FSIZE, (MAX_LOG, MAX_LOG))
    signal.signal(signal.SIGTTOU, signal.SIG_IGN)


def spawn(pin, argv, **kwargs):
    """Execute the opened, hashed inode, including for short-lived helpers."""
    if argv[0] != pin['executable']:
        raise ValueError('executable argv differs from pin')
    fd = os.open(h.r.absolute(pin['executable']), os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        with os.fdopen(os.dup(fd), 'rb') as file:
            info = os.fstat(fd)
            if (not stat.S_ISREG(info.st_mode) or info.st_mode & 0o022
                    or hashlib.file_digest(file, 'sha256').hexdigest() != h.r.digest(pin['sha256'])):
                raise ValueError('unsafe or changed executable')
        inherited = kwargs.pop('pass_fds', ())
        return subprocess.Popen(argv, executable=f'/proc/self/fd/{fd}', pass_fds=(*inherited, fd),
                                preexec_fn=child_limits, **kwargs)
    finally:
        os.close(fd)


def environment(runtime):
    env = {key: os.environ[key] for key in ('HOME', 'USER', 'LOGNAME', 'LANG', 'XDG_SESSION_ID', 'XDG_SEAT', 'XDG_VTNR')
           if key in os.environ}
    return dict(env, PATH='/usr/bin:/bin', LC_ALL='C', XDG_RUNTIME_DIR=str(runtime))


def listener_rows(pid):
    rows = [line.split(maxsplit=7) for line in Path(f'/proc/{pid}/net/unix').read_text().splitlines()[1:]]
    return [row for row in rows if len(row) == 8 and row[3:6] == ['00010000', '0001', '01']]


def owned_listeners(server):
    server.check()
    owned = set()
    for fd in Path(f'/proc/{server.pid}/fd').iterdir():
        try:
            owned.add(os.readlink(fd))
        except FileNotFoundError:
            pass
    return [row[7] for row in listener_rows(server.pid) if f'socket:[{row[6]}]' in owned]


def endpoints(server, stack, runtime, display):
    paths = owned_listeners(server)
    if stack == 'niri':
        wayland = [p for p in paths if Path(p).parent == runtime and re.fullmatch(r'wayland-\d+', Path(p).name)]
        ipc = [p for p in paths if Path(p).parent == runtime and re.fullmatch(r'niri\..+\.sock', Path(p).name)]
        if not wayland or not ipc:
            return None
        if len(wayland) != 1 or len(ipc) != 1:
            raise ValueError('ambiguous compositor listener')
        result = dict(WAYLAND_DISPLAY=wayland[0], NIRI_SOCKET=ipc[0])
    else:
        path = f'/tmp/.X11-unix/X{display}'
        if paths.count(path) != 1:
            return None
        result = dict(DISPLAY=f':{display}')
    namespace = [row[7] for row in listener_rows(server.pid)]
    checked = list(result.values()) if stack == 'niri' else [path]
    for path in checked:
        if namespace.count(path) != 1:
            raise ValueError('ambiguous or replaced compositor listener')
        info = os.lstat(path)
        if not stat.S_ISSOCK(info.st_mode) or info.st_uid != os.getuid():
            raise ValueError('unsafe compositor socket')
    # Linux Xtrans "local" prefers the abstract address; do not bind only
    # its filesystem fallback while accepting a foreign abstract listener.
    if stack == 'xlibre' and '@'+path in namespace:
        if namespace.count('@'+path) != 1 or paths.count('@'+path) != 1:
            raise ValueError('foreign or ambiguous abstract X listener')
    return result


def device_check(server):
    devices = set()
    for fd in Path(f'/proc/{server.pid}/fd').iterdir():
        try:
            info = fd.stat()
        except FileNotFoundError:
            continue
        if stat.S_ISCHR(info.st_mode) and os.major(info.st_rdev) == 226:
            devices.add((Path('/sys/dev/char') / f'226:{os.minor(info.st_rdev)}' / 'device').resolve(strict=True).name)
    if devices != {'0000:03:00.0'}:
        raise ValueError('compositor DRM device mismatch')
    return sorted(devices)


def drm_layout():
    """Numbered paths are valid only for this observed PCI mapping; never guess."""
    rows = {}
    for name, minor, pci in [('card0', 0, '0000:03:00.0'), ('renderD128', 128, '0000:03:00.0'),
                             ('card1', 1, '0000:16:00.0'), ('renderD129', 129, '0000:16:00.0')]:
        node = Path('/dev/dri')/name
        info = node.lstat()
        observed = (Path('/sys/class/drm')/name/'device').resolve(strict=True).name
        if (not stat.S_ISCHR(info.st_mode) or os.major(info.st_rdev) != 226
                or os.minor(info.st_rdev) != minor or observed != pci):
            raise ValueError('DRM node probe order or PCI mapping changed')
        rows[name] = dict(path=str(node), major=226, minor=minor, pci=observed)
    return rows


def command(argv, pin, env, destination, seconds, guard=lambda: None, limit=1024*1024):
    """Drain both streams concurrently, bounded in bytes and time; reap on every path."""
    deadline = time.monotonic()+seconds
    child = spawn(pin, argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                  env=env, start_new_session=True)
    buffers = {child.stdout: bytearray(), child.stderr: bytearray()}
    streams = list(buffers)
    rc = None
    try:
        while streams:
            guard()
            left = deadline-time.monotonic()
            if left <= 0:
                raise ValueError('capture/query deadline')
            # A short poll bounds detection of compositor/client failure as well.
            for stream in select.select(streams, [], [], min(left, .1))[0]:
                part = os.read(stream.fileno(), 65536)
                if not part:
                    streams.remove(stream)
                else:
                    buffers[stream].extend(part)
                    if len(buffers[stream]) > (limit if stream is child.stdout else 65536):
                        raise ValueError('capture/query output limit')
        rc = child.wait(timeout=max(.001, deadline-time.monotonic()))
        if rc != 0 or buffers[child.stderr]:
            raise ValueError('capture/query failed or wrote stderr')
        guard()
        return bytes(buffers[child.stdout])
    finally:
        forced = rc is None
        if forced:
            # Kill the still-owned group before reaping its leader, including
            # helpers whose descendants keep a pipe open after the leader exits.
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait(timeout=2)
        with destination.with_suffix('.stdout').open('xb') as file:
            file.write(buffers[child.stdout])
        with destination.with_suffix('.stderr').open('xb') as file:
            file.write(buffers[child.stderr])
        write(destination.with_suffix('.json'), dict(argv=argv, rc=child.returncode, forced=forced,
              stdout_bytes=len(buffers[child.stdout]), stderr_bytes=len(buffers[child.stderr])))
        child.stdout.close()
        child.stderr.close()


def completed_file(notification, name, deadline, guard):
    while True:
        guard()
        left = deadline-time.monotonic()
        if left <= 0:
            raise ValueError('screenshot did not finish writing')
        if not select.select([notification], [], [], min(left, .1))[0]:
            continue
        data = os.read(notification, 65536)
        cursor = 0
        while cursor < len(data):
            _, mask, _, size = struct.unpack_from('iIII', data, cursor)
            entry = data[cursor+16:cursor+16+size].split(b'\0', 1)[0]
            cursor += 16+size
            if mask & (0x4000 | 0x400 | 0x800):
                raise ValueError('screenshot watch overflow or directory moved')
            if entry == name.encode() and mask & (0x8 | 0x80):
                return


def run(recipe, root):
    validate(recipe)
    if os.getuid() == 0 or os.getuid() != os.geteuid():
        raise ValueError('run as the admitted unprivileged PAM user')
    tty = os.ttyname(0)
    if re.fullmatch(r'/dev/tty[1-9][0-9]*', tty) is None:
        raise ValueError('native diagnostic requires an attended text VT')
    runtime = root / 'runtime'
    if recipe['stack'] == 'niri' and len(os.fsencode(runtime))+40 > 107:
        raise ValueError('evidence path too long for niri Unix sockets; use a shorter private run path')
    root.mkdir(mode=0o700)
    runtime.mkdir(mode=0o700)
    pins, stack = recipe['pins'], recipe['stack']
    env = environment(runtime)
    children, identities, handles = [], [], []
    server = client = None
    endpoint = control = notification = None
    result = None
    deadline = time.monotonic()+300
    try:
        write(root/'recipe.json', recipe)
        write(root/'drm-layout.json', drm_layout())
        server_log = (root/'server.log').open('xb'); handles.append(server_log)
        config = pins['config']['executable']
        if stack == 'niri':
            command([pins['server']['executable'], 'validate', '--config', config], pins['server'], dict(env, RUST_LOG='error'), root/'config-validation', 10)
            argv = [pins['server']['executable'], '--config', config]
            env.update(XDG_SESSION_TYPE='wayland', RUST_LOG='error')
            watched = runtime
        else:
            display = recipe['display']
            if Path(f'/tmp/.X11-unix/X{display}').exists() or Path(f'/tmp/.X{display}-lock').exists():
                raise ValueError('X display already occupied')
            env.update(DISPLAY=f':{display}', XAUTHORITY=str(root/'Xauthority'))
            (root/'Xauthority').touch(mode=0o600)
            # Cookie is an authority secret, so never put its argv in a receipt.
            with (root/'xauth.stderr').open('xb') as err:
                auth = spawn(pins['xauth'], [pins['xauth']['executable'], '-f', env['XAUTHORITY'], 'add', env['DISPLAY'], '.', os.urandom(16).hex()],
                             stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=err, env=env, start_new_session=True)
                try:
                    if auth.wait(timeout=10) != 0 or err.tell() != 0:
                        raise ValueError('private Xauthority setup failed')
                finally:
                    stop(auth)
            (root/'xorg.conf.d').mkdir(mode=0o700)
            prefix = Path(pins['server']['executable']).parents[1]
            argv = [pins['server']['executable'], env['DISPLAY'], 'vt'+tty[8:], '-keeptty', '-seat', 'seat0',
                    '-nolisten', 'tcp', '-noreset', '-auth', env['XAUTHORITY'], '-modulepath', str(prefix/'lib/xorg/modules/xlibre-25'),
                    '-config', config, '-configdir', str(root/'xorg.conf.d'), '-logfile', str(root/'Xorg.log')]
            watched = None
        if watched is not None:
            watch_dir = os.open(watched, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
            try:
                notification = h.c.watch(watch_dir)
            finally:
                os.close(watch_dir)
        server_env = dict(env, RUST_LOG='warn,niri::backend::tty=info') if stack == 'niri' else env
        server = spawn(pins['server'], argv, stdin=None, stdout=server_log, stderr=server_log, env=server_env, process_group=0)
        children.append(server)
        server_pin = dict(pins['server'], pid=server.pid, starttime=h.c.snapshot(server.pid))
        bound_server = h.c.Session(server_pin); identities.append(bound_server)
        startup = time.monotonic()+20
        while True:
            endpoint = endpoints(bound_server, stack, runtime, recipe['display'])
            if endpoint is not None:
                break
            left = startup-time.monotonic()
            if left <= 0:
                raise ValueError('compositor listener startup deadline')
            # bind creates a filesystem event before listen; listen has no
            # inotify event. Recheck the kernel listener table under the bound.
            select.select([bound_server.fd, *([] if notification is None else [notification])], [], [], min(left, .05))
            if notification is not None and select.select([notification], [], [], 0)[0]:
                os.read(notification, 65536)
        if notification is not None:
            os.close(notification); notification = None
        env.update(endpoint)
        if stack == 'xlibre':
            text = command([pins['output']['executable'], '--query'], pins['output'], env, root/'outputs-initial', 10,
                           bound_server.check).decode()
            names = re.findall(r'^(\S+) (?:connected|disconnected)\b', text, re.M)
            if names.count('DP-1') != 1:
                raise ValueError('X did not list DP-1 exactly once')
            off = [arg for name in names if name != 'DP-1' for arg in ('--output', name, '--off')]
            command([pins['output']['executable'], '--output', 'DP-1', '--mode', '1920x1080', '--rate', '119.880', '--primary', *off],
                    pins['output'], env, root/'outputs-set', 10, bound_server.check)
        control, inherited = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
        control.setblocking(False)
        journal_path = root/'journal.tsv'
        client_log = (root/'client.log').open('xb'); handles.append(client_log)
        args = ([str(journal_path)] if stack == 'niri' else ['--journal', str(journal_path)])
        args += ['--frames', '3', '--warmup', '0', '--buffers', '2', '--lifecycle', 'fresh', '--workload', 'fresh', '--hold-fd', str(inherited.fileno())]
        if stack == 'niri':
            args += ['--output', 'DP-1']
        try:
            client = spawn(pins['client'], [pins['client']['executable'], *args], stdin=subprocess.DEVNULL, stdout=client_log, stderr=client_log,
                           env=env, pass_fds=(inherited.fileno(),), start_new_session=True)
        finally:
            inherited.close()
        children.append(client)
        client_pin = dict(pins['client'], pid=client.pid, starttime=h.c.snapshot(client.pid))
        bound_client = h.c.Session(client_pin); identities.append(bound_client)

        def identity():
            if time.monotonic() >= deadline:
                raise ValueError('outer colour deadline')
            bound_server.check(); bound_client.check()
            drm_layout()
            if endpoints(bound_server, stack, runtime, recipe['display']) != endpoint:
                raise ValueError('compositor endpoint changed')
            device_check(bound_server)

        def quiet():
            identity()
            if select.select([control], [], [], 0)[0]:
                raise ValueError('client advanced/exited during capture')

        def output(label):
            args = ['msg', '--json', 'outputs'] if stack == 'niri' else ['--current', '--verbose']
            text = command([pins['output']['executable'], *args], pins['output'], env, root/label, 10, quiet).decode()
            facts = (outputs.niri if stack == 'niri' else outputs.xlibre)(text)
            write(root/(label+'-qualified.json'), facts)
            return facts

        def capture(generation, pattern):
            before = output(f'output-{generation}-before')
            until = time.monotonic()+35
            if stack == 'niri':
                png = root/f'capture-{generation}.png'
                if png.exists() or png.is_symlink():
                    raise ValueError('screenshot destination already exists')
                directory = h.c.private_directory(str(root))
                event = h.c.watch(directory)
                try:
                    command([pins['server']['executable'], 'msg', 'action', 'screenshot-screen', '--path', str(png),
                             '--show-pointer', 'false', '--write-to-disk', 'true'], pins['server'], env,
                            root/f'screenshot-{generation}', 10, quiet)
                    completed_file(event, png.name, until, quiet)
                    raw = read(png, 20*1024*1024)
                    write(root/f'png-{generation}.json', dict(sha256=hashlib.sha256(raw).hexdigest(), bytes=len(raw)))
                finally:
                    os.close(event); os.close(directory)
                args = [str(png)]
            else:
                args = []
            image = command([pins['capture']['executable'], *args], pins['capture'], env, root/f'pixels-{generation}',
                            max(.001, until-time.monotonic()), quiet, PAM_LIMIT)
            if stack == 'niri' and raw != read(png, 20*1024*1024):
                raise ValueError('screenshot changed during normalization')
            if before != output(f'output-{generation}-after'):
                raise ValueError('output changed across capture')
            return image

        sequence = colour.run(control, bound_client.fd, capture, identity)
        if client.wait(timeout=5) != 0:
            raise ValueError('colour client did not exit successfully')
        text = read(journal_path, 1024*1024).decode()
        receipt = journal.qualify_colour(text, 'wayland' if stack == 'niri' else 'x11')
        times = [int(line.split('\t')[3]) for line in text.splitlines() if line and not line.startswith('#')]
        if receipt['client_pid'] != client.pid or times != [row['ready']['presented_ns'] for row in sequence['records']]:
            raise ValueError('client journal identity/presentation differs from held generations')
        bound_server.check()
        result = dict(schema=1, status='colour_source_path_completed', stack=stack, server=server_pin, client=client_pin,
                      server_argv=argv, client_argv=[pins['client']['executable'], *args], endpoint=endpoint,
                      sequence=sequence, journal=receipt,
                      expected_inherited_fd='Each spawned executable retains its opened regular-file exec FD; descendants may inherit it.',
                      host_unpinned_inputs=(['Xorg xkbcomp, xkeyboard-config data and libinput input module; package closure must bind these or retain this limit.'] if stack == 'xlibre' else []),
                      limits='Untimed local process/capture evidence; requires package build closure, outer PAM/VT cleanup and separate real import-refusal/delayed-release controls. X manager selection check is not a proof against an unregistered redirector.')
    finally:
        cleanup = []
        for child in reversed(children):
            try:
                unexpected_exit = child is server and child.poll() is not None
                forced = stop(child)
                cleanup.append(dict(pid=child.pid, rc=child.returncode, forced=forced,
                                    unexpected_server_exit=unexpected_exit))
            except (OSError, subprocess.SubprocessError) as error:
                cleanup.append(dict(pid=child.pid, rc=child.returncode, forced=True, error=str(error)))
        for bound in reversed(identities):
            bound.close()
        if control is not None:
            control.close()
        if notification is not None:
            os.close(notification)
        for file in handles:
            file.close()
        write(root/'cleanup.json', dict(children=cleanup, scope='Owned children and process-group escalation only; outer descendant/PAM/cgroup/VT audit required.'))
    if any(row['forced'] or row.get('unexpected_server_exit') for row in cleanup) or server.returncode not in (0, -signal.SIGTERM):
        raise ValueError('unexpected server exit or forced termination during colour cleanup')
    write(root/'result.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--recipe', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    def interrupted(signum, _frame):
        raise InterruptedError(f'wrapper signal {signum}')
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    root = Path(h.r.absolute(str(args.evidence)))
    if root.exists() or root.is_symlink():
        return 1
    try:
        return 0 if run(h.r.record(read(args.recipe, 32768)), root) else 1
    except (OSError, ValueError, TypeError, KeyError, IndexError, subprocess.SubprocessError) as error:
        if root.is_dir() and not (root/'refused.json').exists():
            write(root/'refused.json', dict(schema=1, status='refused', error=str(error)))
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
