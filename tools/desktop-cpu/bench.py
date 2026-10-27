#!/usr/bin/env python3
"""Same-workload CPU-per-frame harness for XLibre, niri and Sophia.

Workload: the external-bar 1920x24 strip, a new full panel every presented
frame (workload fresh), the same fill as the Sophia timing peer. Each stack
runs WARMUP + FRAMES generations; the counted window runs from the
presentation of generation WARMUP+1 to generation WARMUP+FRAMES, which is
the 120..2120 window (2000 intervals) Sophia's shell-timing runs use.

  run --stack niri|xlibre --mode native|smoke --out DIR
      native: run on a free text VT as the seat user; real KMS output.
      smoke: GPU- and device-free inside bwrap (dummy X; a wlroots headless
             stand-in for niri, whose config niri still validates);
             protocol and harness check only, never CPU evidence.
  import-sophia --out DIR SOPHIA_RUN   an existing shell-timing run directory
  report --out DIR RUN...              one cpuwin plan over all runs

CPU is cpuwin.py's tick-bounded attribution over every bound process of a
stack (Sophia: session+peer; niri: compositor+client; XLibre: server+client,
plus window manager and compositor when started). Anchors are presentation
times in CLOCK_MONOTONIC: kernel completion for Sophia, wp_presentation for
niri, validated Present COMPLETE_NOTIFY ust for X.
Run with python3 -I -B.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import signal
import statistics
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
CPUWIN = REPO / 'tools/cpu-window/cpuwin.py'
CLIENTS = REPO / 'tools/external-bar/build'
SCHEMA = 1
RIG_OTHER_OUTPUTS = ['HDMI-A-1', 'HDMI-A-2', 'DP-2', 'DP-3', 'DP-4', 'DP-5', 'DP-6',
                     'Writeback-1', 'Writeback-2']
NATIVE_SOURCES = {'niri': {'wp_presentation'}, 'xlibre': {'xpresent'}}
CLIENT_ARM = {'niri': 'wl-panel', 'xlibre': 'x-panel'}
# The one compared recipe, as both clients write it in their journal header.
RECIPE = dict(width='1920', height='24', buffers='2', fill='shade_mod128_a255', frames='2001', warmup='119',
              bulk='0', bulk_width='0', bulk_height='0', lifecycle='reuse', workload='fresh', tiles='0',
              interval_ms='0', damage='full', idle_sec='0', clock='CLOCK_MONOTONIC_ns')
SOPHIA_FILES = ('cpu-window.jsonl', 'cpu-binding.json', 'native-samples.json', 'postflight.json', 'result.json',
                'plan.json', 'panel-diagnostic.json')
DUMMY_XORG = '''Section "ServerFlags"
 Option "AutoAddDevices" "false"
 Option "AutoAddGPU" "false"
 Option "AutoEnableDevices" "false"
 Option "DontVTSwitch" "true"
EndSection
Section "Device"
 Identifier "dummy"
 Driver "dummy"
 VideoRam 65536
EndSection
Section "Monitor"
 Identifier "screen"
 HorizSync 20-200
 VertRefresh 20-200
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


class Refused(Exception):
    pass


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for block in iter(lambda: f.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


def write_json(path, value):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'w') as f:
        json.dump(value, f, indent=1, sort_keys=True)
        f.write('\n')


def stat_fields(pid):
    """(starttime, session id, comm) from /proc/PID/stat, or None if gone."""
    try:
        raw = Path(f'/proc/{pid}/stat').read_text()
    except (FileNotFoundError, ProcessLookupError):
        return None
    comm = raw[raw.index('(') + 1:raw.rindex(')')]
    rest = raw[raw.rindex(')') + 2:].split()
    return int(rest[19]), int(rest[3]), comm  # fields 22, 6, 2


def stack_processes(sessions):
    """Every live process in the given session ids (each child gets its own)."""
    found = {}
    for entry in Path('/proc').iterdir():
        if entry.name.isdigit():
            fields = stat_fields(int(entry.name))
            if fields and fields[1] in sessions:
                found[int(entry.name)] = dict(starttime=fields[0], comm=fields[2])
    return found


def stop(process, grace=3):
    if process is None or process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout=grace)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()
    except ProcessLookupError:
        pass


def stop_traced(process, grace=3):
    """A traced child is never waited through Popen: a signal to it is a trace
    stop, which Popen would record as its exit. Kill it and drain its stops
    until the kernel reports its death."""
    if process.returncode is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    deadline = time.monotonic() + grace
    while time.monotonic() < deadline:
        try:
            pid, status = os.waitpid(process.pid, os.WNOHANG)
        except ChildProcessError:
            return
        if pid == 0:
            time.sleep(0.005)
        elif os.WIFEXITED(status) or os.WIFSIGNALED(status):
            process.returncode = os.waitstatus_to_exitcode(status)
            return
        else:  # the exit-event stop; death follows the continue
            LIBC.ptrace(PTRACE_CONT, process.pid, None, None)
    raise Refused(f'traced client {process.pid} not reaped {grace} s after SIGKILL')


def wait_for(predicate, process, what, seconds=10):
    deadline = time.monotonic() + seconds
    while not predicate():
        if process.poll() is not None:
            raise Refused(f'{what}: process exited with {process.returncode} during startup')
        if time.monotonic() > deadline:
            raise Refused(f'{what}: not ready after {seconds} s')
        time.sleep(0.02)


def native_xorg(args):
    # Codex recipe: one GPU and its card node, modesetting/glamor, explicit
    # TearFree off (installed default on), VRR off, page flips on. Outputs
    # are chosen at runtime with xrandr among the server's own modes.
    return ('Section "ServerFlags"\n Option "AutoAddGPU" "false"\nEndSection\n'
            f'Section "Device"\n Identifier "gpu"\n Driver "modesetting"\n BusID "{args.bus_id}"\n'
            f' Option "kmsdev" "{args.kmsdev}"\n Option "AccelMethod" "glamor"\n'
            ' Option "TearFree" "false"\n Option "VariableRefresh" "false"\n'
            ' Option "PageFlip" "true"\nEndSection\n'
            'Section "Screen"\n Identifier "screen"\n Device "gpu"\n DefaultDepth 24\nEndSection\n')


def free_display():
    for n in range(20, 100):
        if not Path(f'/tmp/.X{n}-lock').exists() and not Path(f'/tmp/.X11-unix/X{n}').exists():
            return n
    raise Refused('no free X display number')


LIBC = ctypes.CDLL(None, use_errno=True)
PTRACE_TRACEME, PTRACE_CONT, PTRACE_SETOPTIONS = 0, 7, 0x4200
PTRACE_O_TRACEEXIT, PTRACE_O_EXITKILL, PTRACE_EVENT_EXIT = 0x40, 0x100000, 6


def ptrace(request, pid, data=0):
    if LIBC.ptrace(request, pid, None, ctypes.c_void_p(data)) != 0:
        raise Refused(f'ptrace {request:#x} on {pid}: errno {ctypes.get_errno()}')


def trace_me():
    # Runs in the child before exec: the kernel then stops it with SIGTRAP
    # just after execve, so it is bound under its final comm before it runs.
    if LIBC.ptrace(PTRACE_TRACEME, 0, None, None) != 0:
        os._exit(127)


def held_at_exec(process):
    """The client is stopped after execve. It will also stop at its exit, still
    the same readable process with final CPU counters, until released."""
    _, status = os.waitpid(process.pid, 0)
    if os.WIFEXITED(status) or os.WIFSIGNALED(status):
        process.returncode = os.waitstatus_to_exitcode(status)
    if not (os.WIFSTOPPED(status) and os.WSTOPSIG(status) == signal.SIGTRAP):
        raise Refused(f'client was not held at exec (status {status:#x})')
    ptrace(PTRACE_SETOPTIONS, process.pid, PTRACE_O_TRACEEXIT | PTRACE_O_EXITKILL)


def run_held(process, timeout, hold):
    """Resume the held client, forward its signals, and keep it alive `hold`
    seconds at its exit stop. Returns (exit status or None, exit-stop time)."""
    ptrace(PTRACE_CONT, process.pid)
    deadline, exit_stop = time.monotonic() + timeout, None
    while time.monotonic() < deadline:
        pid, status = os.waitpid(process.pid, os.WNOHANG)
        if pid == 0:
            time.sleep(0.005)
            continue
        if os.WIFEXITED(status) or os.WIFSIGNALED(status):
            process.returncode = os.waitstatus_to_exitcode(status)
            return process.returncode, exit_stop
        if status >> 8 == signal.SIGTRAP | (PTRACE_EVENT_EXIT << 8):
            exit_stop = time.monotonic_ns()
            time.sleep(hold)  # samples past the final presentation still read this process
            ptrace(PTRACE_CONT, process.pid)
        else:
            ptrace(PTRACE_CONT, process.pid, os.WSTOPSIG(status))  # deliver the signal
    return None, exit_stop


class Stack:
    """Started processes of one run, each in its own session."""

    def __init__(self, out, env):
        self.out, self.env, self.started, self.labels, self.traced = out, env, [], {}, set()

    def spawn(self, label, argv, env=None, bind=True, hold=False):
        with (self.out / f'{label}.log').open('w') as log:  # the child keeps its own copy
            process = subprocess.Popen(argv, env=env or self.env, stdout=log, stderr=subprocess.STDOUT,
                                       stdin=subprocess.DEVNULL, start_new_session=True,
                                       preexec_fn=trace_me if hold else None)
        self.started.append((label, process, argv))  # before anything can raise
        if hold:
            self.traced.add(process.pid)
            held_at_exec(process)
        if bind:
            self.labels[label] = process
        return process

    def teardown(self):
        """Stop everything, newest first; returns what could not be stopped."""
        errors = []
        for label, process, _ in reversed(self.started):
            try:
                (stop_traced if process.pid in self.traced else stop)(process)
            except Exception as e:
                errors.append(f'{label}: {e!r}')
        return errors


def x_server(stack, args, env, native, label='server'):
    display = free_display()
    socket = Path(f'/tmp/.X11-unix/X{display}')
    if native:
        tty = os.ttyname(0)
        if not tty.startswith('/dev/tty') or not tty[8:].isdigit():
            raise Refused(f'native X needs a text VT on stdin, not {tty}')
        prefix = Path(args.xlibre_prefix)
        (stack.out / 'xorg.conf').write_text(native_xorg(args))
        (stack.out / 'xorg.conf.d').mkdir(mode=0o700)
        cookie = stack.out / 'Xauthority'
        cookie.touch(mode=0o600)
        subprocess.run(['xauth', '-f', str(cookie), 'add', f':{display}', '.',
                        subprocess.run(['mcookie'], capture_output=True, text=True, check=True).stdout.strip()],
                       check=True, timeout=10)
        env = dict(env, XAUTHORITY=str(cookie))
        # The unprivileged prefix server, never the setuid /usr/bin/Xorg wrapper;
        # absolute private -config/-configdir are honoured without privileges.
        argv = [str(prefix / 'bin/Xorg'), f':{display}', f'vt{tty[8:]}', '-keeptty', '-seat', args.seat,
                '-nolisten', 'tcp', '-noreset', '-auth', str(cookie), '-modulepath', str(prefix / 'lib/xorg/modules/xlibre-25'),
                '-config', str(stack.out / 'xorg.conf'), '-configdir', str(stack.out / 'xorg.conf.d'),
                '-logfile', str(stack.out / 'Xorg.log')]
    else:
        (stack.out / 'xorg.conf').write_text(DUMMY_XORG)
        argv = [args.smoke_xserver, f':{display}', '-config', str(stack.out / 'xorg.conf'),
                '-logfile', str(stack.out / 'Xorg.log'), '-nolisten', 'tcp', '-noreset',
                '-novtswitch', '-ac']
    server = stack.spawn(label, argv, bind=label == 'server')
    wait_for(socket.is_socket, server, 'X server')
    return dict(env, DISPLAY=f':{display}'), argv[0]


def run_stack(args, out):
    native = args.mode == 'native'
    runtime = out / 'runtime'
    runtime.mkdir(mode=0o700)
    env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime))
    for key in ('DISPLAY', 'WAYLAND_DISPLAY', 'WAYLAND_SOCKET', 'NIRI_SOCKET', 'XAUTHORITY',
                'DBUS_SESSION_BUS_ADDRESS'):
        env.pop(key, None)
    journal = out / 'journal.tsv'
    workload = [arg for k in ('frames', 'warmup', 'buffers', 'lifecycle', 'workload')
                for arg in (f'--{k}', RECIPE[k])]
    stack, binaries = Stack(out, env), {}
    record = dict(schema=SCHEMA, stack=args.stack, mode=args.mode, recipe=RECIPE,
                  output=args.output, mode_size=args.mode_size, rate=args.rate,
                  disabled_outputs=args.disable_output,
                  evidence='native' if native else 'diagnostic')
    try:
        if args.stack == 'xlibre':
            xenv, server_bin = x_server(stack, args, env, native)
            binaries['server'] = server_bin
            if native and args.output:
                # X names connectors itself (modesetting: HDMI-1 for kernel HDMI-A-1),
                # so every output the server lists, other than ours, is turned off.
                listed = x_outputs(subprocess.run(['xrandr', '--query'], env=xenv, capture_output=True,
                                                  text=True, check=True, timeout=10).stdout)
                if args.output not in listed:
                    raise Refused(f'X lists no output {args.output}: {listed}')
                record['disabled_outputs'] = [name for name in listed if name != args.output]
                off = [arg for name in record['disabled_outputs'] for arg in ('--output', name, '--off')]
                subprocess.run(['xrandr', '--output', args.output, '--mode', args.mode_size, '--rate',
                                args.rate, '--primary', *off], env=xenv, check=True, timeout=10)
                # Readback of the chosen mode timings; the report checks frame gaps.
                with (out / 'xrandr.txt').open('w') as readback:
                    subprocess.run(['xrandr', '--verbose'], env=xenv, stdout=readback, check=True, timeout=10)
            if args.wm:
                binaries['wm'] = args.wm
                stack.spawn('wm', [args.wm], env=xenv)
                time.sleep(0.5)
            if args.compositor:
                binaries['compositor'] = args.compositor
                stack.spawn('compositor', [args.compositor, '-n'], env=xenv)
                time.sleep(0.5)
            client_argv = [str(CLIENTS / 'x-panel'), '--journal', str(journal), *workload]
            client_env = xenv
        else:
            config = out / 'niri.kdl'
            config.write_text(niri_config(args, native))
            subprocess.run([args.niri, 'validate', '--config', str(config)], env=env, check=True,
                           timeout=10, stdout=subprocess.DEVNULL)
            if native:
                binaries['server'] = args.niri
                # Codex recipe: no --session, no startup commands, explicit logging.
                nenv = dict(env, XDG_SESSION_TYPE='wayland', RUST_LOG=args.niri_log)
                nenv.pop('NIRI_CONFIG', None)
                record['niri_env'] = dict(XDG_SESSION_TYPE='wayland', RUST_LOG=args.niri_log)
                server = stack.spawn('server', [args.niri, '--config', str(config)], env=nenv)
            else:
                # niri's headless backend is test-only and its nested winit output
                # is always transformed, which the client refuses. A headless
                # wlroots compositor stands in so the harness path is exercised;
                # record['smoke_server'] says so and the row is never evidence.
                binaries['server'] = args.smoke_wayland
                record['smoke_server'] = 'wlroots-headless-pixman stand-in, not niri'
                wenv = dict(env, WLR_BACKENDS='headless', WLR_HEADLESS_OUTPUTS='1',
                            WLR_RENDERER='pixman', WLR_LIBINPUT_NO_DEVICES='1')
                server = stack.spawn('server', [args.smoke_wayland, '-no-xwayland', '-c', 'true'], env=wenv)
            wait_for(lambda: any(p.is_socket() for p in runtime.glob('wayland-*')), server, 'compositor')
            sockets = [p.name for p in runtime.glob('wayland-*') if p.is_socket()]
            if len(sockets) != 1:
                raise Refused(f'expected one Wayland socket, found {sockets}')
            client_env = dict(env, WAYLAND_DISPLAY=sockets[0])
            if native:
                ipc = [p for p in runtime.glob('niri.*.sock') if p.is_socket()]
                if len(ipc) != 1:
                    raise Refused(f'expected one niri IPC socket, found {ipc}')
                with (out / 'outputs.json').open('w') as receipt:  # actual mode, not the requested one
                    subprocess.run([args.niri, 'msg', '--json', 'outputs'], env=dict(env, NIRI_SOCKET=str(ipc[0])),
                                   stdout=receipt, check=True, timeout=10)
            if not native:
                binaries['mode-helper'] = str(CLIENTS / 'headless-output')
                subprocess.run([str(CLIENTS / 'headless-output')], env=dict(client_env, EXTERNAL_BAR_HEADLESS_SMOKE='1'), check=True, timeout=10,
                               stdout=(out / 'mode-helper.log').open('w'), stderr=subprocess.STDOUT)
            client_argv = [str(CLIENTS / 'wl-panel'), str(journal), *workload]
            if args.output and native:
                client_argv += ['--output', args.output]
        binaries['client'] = client_argv[0]
        time.sleep(args.settle)
        client = stack.spawn('client', client_argv, env=client_env, hold=True)
        expect = {}
        for label, process in stack.labels.items():
            fields = stat_fields(process.pid)
            if fields is None:
                raise Refused(f'{label} exited before binding')
            expect[label] = dict(pid=process.pid, starttime=fields[0])
        sessions = {p.pid for _, p, _ in stack.started}
        bound = {v['pid'] for v in expect.values()}
        scaffolding = {p.pid for label, p, _ in stack.started if label not in stack.labels}
        sampler_argv = [sys.executable, '-I', '-B', str(CPUWIN), 'sample', '--out',
                        str(out / 'cpu-window.jsonl'), '--seconds', str(args.timeout),
                        '--interval-ms', '5'] + [f'--pid={k}={v["pid"]}:{v["starttime"]}'
                                                 for k, v in expect.items()]
        sampler = stack.spawn('sampler', sampler_argv, bind=False)  # teardown stops it on any error

        def unbound():
            return {str(pid): v for pid, v in stack_processes(sessions).items()
                    if pid not in bound and pid not in scaffolding}

        samples = out / 'cpu-window.jsonl'
        wait_for(lambda: samples.exists() and samples.read_text().count('\n') >= 3, sampler, 'sampler')
        early = unbound()
        # The client submits nothing before the sampler holds samples, and is
        # held at exit until samples past its final presentation exist.
        client_status, exit_stop_ns = run_held(client, args.timeout, hold=0.3)
        late = unbound()
        stop(sampler)
        record.update(expect=expect, client_status=client_status, client_exit_stop_ns=exit_stop_ns,
                      unbound_start=early,
                      unbound_end=late, sampler_status=sampler.returncode,
                      argv={label: argv for label, _, argv in stack.started},
                      sampler_argv=sampler_argv,
                      binaries={k: dict(path=str(Path(v).resolve()), sha256=sha256(v))
                                for k, v in binaries.items()})
        if client_status != 0:
            raise Refused(f'client exited with {client_status}; see client.log')
        record['finished'] = True
    except BaseException as e:
        record['error'] = repr(e)
        raise
    finally:
        record['teardown_errors'] = stack.teardown()
        write_json(out / 'run.json', record)


def x_outputs(query):
    """Output names in xrandr --query order."""
    return [line.split()[0] for line in query.splitlines()
            if line and not line[0].isspace() and line.split()[1:2] in (['connected'], ['disconnected'])]


def niri_config(args, native):
    lines = ['hotkey-overlay {\n    skip-at-startup\n}', 'animations {\n    off\n}',
             'xwayland-satellite {\n    off\n}', 'prefer-no-csd']
    if native and args.output:
        lines.append(f'output "{args.output}" {{\n    mode "{args.mode_size}@{args.rate}"\n'
                     '    scale 1\n    transform "normal"\n    position x=0 y=0\n}')
    for name in args.disable_output:
        lines.append(f'output "{name}" {{\n    off\n}}')
    return '\n'.join(lines) + '\n'


def cmd_run(args):
    out = args.out.resolve()
    if args.inside:
        out = Path('/tmp/bench')
    else:
        out.mkdir(parents=True, exist_ok=False, mode=0o700)
    if args.mode == 'smoke' and not args.inside:
        # Private /dev without DRM or input nodes, private /tmp and /run, all
        # namespaces unshared; only the fresh run directory is writable.
        command = ['bwrap', '--unshare-all', '--die-with-parent', '--ro-bind', '/', '/',
                   '--dev', '/dev', '--proc', '/proc', '--tmpfs', '/tmp', '--tmpfs', '/run',
                   '--bind', str(out), '/tmp/bench', '--chdir', '/tmp/bench',
                   sys.executable, '-I', '-B', str(Path(__file__).resolve()), *sys.argv[1:], '--inside']
        status = subprocess.run(command, timeout=args.timeout + 60).returncode
        return relocate(out) if status == 0 else status
    try:
        run_stack(args, out)
    except Refused as e:
        print(f'REFUSED: {e}', file=sys.stderr)
        return 2
    return 0


def relocate(out):
    """A smoke run records paths inside the sandbox; point them at the host copy."""
    record = json.loads((out / 'run.json').read_text())
    record['sandbox_root'] = '/tmp/bench'
    (out / 'run.json').unlink()
    write_json(out / 'run.json', record)
    return 0


def sophia_window(src):
    """A shell-timing run's counted window, validated: run outcome, output and
    shape, exact kernel-completion joins and sampled identity. Returns
    (instrumentation, expect, presented ns 120..2120, latency ns 121..2120)."""
    load = lambda name: json.loads((src / name).read_text())
    native, binding, request = load('native-samples.json'), load('cpu-binding.json'), load('plan.json')['request']
    result, postflight, panel = load('result.json'), load('postflight.json'), load('panel-diagnostic.json')
    sampled = json.loads((src / 'cpu-window.jsonl').read_text().split('\n', 1)[0])['procs']
    problems = []
    if (result.get('status'), result.get('error')) != ('captured_only', None):
        problems.append(f'result {result.get("status")}: {result.get("error")}')
    if postflight.get('status') != 'passed' or postflight.get('failures'):
        problems.append(f'postflight {postflight.get("status")}')
    observed, target = postflight.get('observed', {}), panel.get('target', {})
    if (observed.get('connector'), observed.get('actual_refresh_millihz')) != ('DP-1', 119880):
        problems.append(f'postflight saw {observed.get("connector")} at {observed.get("actual_refresh_millihz")} mHz')
    if ((target.get('connector'), target.get('width'), target.get('height'), target.get('vrr'))
            != ('DP-1', 1920, 1080, False)
            or (panel.get('bar_height'), panel.get('warmup'), panel.get('counted')) != (24, 120, 2000)):
        problems.append('panel is not the 1920x24 strip on DP-1 1920x1080, VRR off, 120+2000 frames')
    if (request.get('source'), request.get('frames')) != ('owned_ring', 2120) \
            or request.get('instrumentation') != native.get('instrumentation'):
        problems.append('request is not owned_ring, 2120 frames, at the samples\' instrumentation')
    samples = native.get('samples', [])
    if native.get('verdict') != 'joined_only' or native.get('offered') != 2120 \
            or [s['candidate_generation'] for s in samples] != list(range(1, 2121)):
        problems.append('samples are not generations 1..2120, each once, in order')
    else:
        done = [s['completed_monotonic_usec'] for s in samples]
        if any(b <= a for a, b in zip(done, done[1:])):
            problems.append('kernel completions are not strictly increasing')
        if any(not s['submitted_monotonic_usec'] <= s['completed_monotonic_usec']
               or s['latency_usec'] != s['completed_monotonic_usec'] - s['submitted_monotonic_usec']
               or (s['output'], s['heads']) != (1, 1) for s in samples):
            problems.append('a sample does not join one submission to one completion on output 1')
    offers = native.get('journal', {}).get('offers', [])
    if len(offers) != 2120 or any(o.get('outcome') != 'presented' for o in offers):
        problems.append('not every offer was presented')
    expect = {label: dict(pid=binding[key]['pid'], starttime=binding[key]['start_time_ticks'])
              for label, key in (('session', 'host'), ('peer', 'peer'))}
    for label, want in expect.items():
        got = sampled.get(label, {})
        if (got.get('pid'), got.get('starttime')) != (want['pid'], want['starttime']):
            problems.append(f'sampled {label} is not the bound {label}')
    if problems:
        raise Refused(f'{src}: ' + '; '.join(problems))
    presented = [s['completed_monotonic_usec'] * 1000 for s in samples[119:]]
    latency = [s['latency_usec'] * 1000 for s in samples[120:]]
    return native['instrumentation'], expect, presented, latency


def cmd_import_sophia(args):
    src = args.run.resolve()
    try:
        instrumentation, expect, _, _ = sophia_window(src)
    except (Refused, KeyError, TypeError, ValueError, OSError) as e:
        print(f'REFUSED: {e!r}', file=sys.stderr)
        return 2
    # cpuplan.py refuses capture-profile runs for comparisons; so does this,
    # unless asked, and then the row is diagnostic.
    if instrumentation != 'none' and not args.allow_instrumented:
        print(f'REFUSED: Sophia run instrumentation is {instrumentation!r}; pass --allow-instrumented '
              'for a diagnostic row', file=sys.stderr)
        return 2
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False, mode=0o700)
    files = {name: dict(path=str(src / name), sha256=sha256(src / name)) for name in SOPHIA_FILES}
    write_json(out / 'run.json', dict(
        schema=SCHEMA, stack='sophia', mode='imported', instrumentation=instrumentation,
        evidence='native' if instrumentation == 'none' else 'diagnostic',
        source=str(src), files=files, expect=expect,
        artifacts=json.loads((src / 'plan.json').read_text())['request'].get('artifacts')))
    return 0


def parse_journal(path):
    header, rows = None, []
    for line in Path(path).read_text().splitlines():
        if line.startswith('# header '):
            fields, _, note = line[9:].partition(' presented_clock=')
            header = dict(kv.split('=', 1) for kv in fields.split())
            header['presented_clock'] = note
        elif line and not line.startswith('#'):
            frame, fill, submit, presented, pace, source = line.split('\t')
            rows.append(dict(frame=int(frame), fill=int(fill), submit=int(submit),
                             presented=int(presented), source=source))
    if header is None:
        raise Refused(f'{path}: no journal header')
    return header, rows


def quantiles(values):
    values = sorted(values)
    pick = lambda q: values[min(len(values) - 1, int(q * len(values)))]
    return dict(min=values[0], p50=statistics.median(values), p99=pick(0.99), max=values[-1],
                count=len(values))


def frame_stats(presented_ns, latency_ns, nominal_hz=None):
    """Missed refreshes are gaps over 1.5 periods of the observed mode when
    known (else of the median gap, descriptive only)."""
    gaps = [b - a for a, b in zip(presented_ns, presented_ns[1:])]
    period = 1e9 / nominal_hz if nominal_hz else statistics.median(gaps)
    return dict(gap_us=quantiles([g / 1e3 for g in gaps]),
                missed_vblank_gaps=sum(g > 1.5 * period for g in gaps),
                refresh_hz=1e9 / statistics.median(gaps), nominal_hz=nominal_hz,
                latency_us=quantiles([l / 1e3 for l in latency_ns]))


def close(actual, nominal, tolerance=0.001):
    return abs(actual - nominal) <= tolerance * nominal


def niri_output_problems(record, receipt):
    """The mode niri actually set: a rejected request may fall back silently."""
    outputs, problems = json.loads(receipt.read_text()), []
    want = outputs.get(record['output'])
    if want is None or want.get('current_mode') is None:
        return [f'{record["output"]} is not enabled']
    mode = want['modes'][want['current_mode']]
    size = f'{mode["width"]}x{mode["height"]}'
    if size != record['mode_size'] or not close(mode['refresh_rate'] / 1000, float(record['rate'])):
        problems.append(f'{record["output"]} runs {size}@{mode["refresh_rate"] / 1000}')
    if want.get('vrr_enabled'):
        problems.append('VRR is enabled')
    problems += [f'{name} is enabled' for name, o in outputs.items()
                 if name != record['output'] and o.get('current_mode') is not None]
    return problems


def x_output_problems(record, receipt):
    """xrandr --verbose readback: the requested output's current mode and
    vertical clock, and no other active output."""
    problems, output, current, vclock, active = [], None, False, None, []
    for line in receipt.read_text().splitlines():
        if line and not line[0].isspace():
            fields = line.split()
            output = fields[0] if len(fields) > 1 and fields[1] in ('connected', 'disconnected') else None
            if output and any('x' in f and '+' in f for f in fields[2:4]):
                active.append(output)
            current = False
        elif output == record['output'] and line.startswith('  ') and not line.startswith('    '):
            fields = line.split()
            current = '*current' in fields and fields[0] == record['mode_size']
        elif output == record['output'] and current and line.strip().startswith('v:') and 'clock' in line:
            vclock = float(line.split('clock')[1].strip().removesuffix('Hz'))
    if vclock is None:
        problems.append(f'{record["output"]} has no current {record["mode_size"]} mode')
    elif not close(vclock, float(record['rate']), 0.0005):
        problems.append(f'{record["output"]} vertical clock {vclock} Hz')
    problems += [f'{name} is active' for name in active if name != record['output']]
    return problems


def plan_run(name, run_dir):
    """One cpuwin plan entry plus frame observations for a recorded run."""
    record = json.loads((run_dir / 'run.json').read_text())
    if record.get('schema') != SCHEMA:
        raise Refused(f'{run_dir}: run.json schema {record.get("schema")}')
    notes, evidence = [], record['evidence']
    if record['stack'] == 'sophia':
        src = Path(record['source'])
        if sorted(record['files']) != sorted(SOPHIA_FILES):
            raise Refused(f'{run_dir}: import does not pin {SOPHIA_FILES}')
        for label, f in record['files'].items():
            if f['path'] != str(src / label) or sha256(f['path']) != f['sha256']:
                raise Refused(f'{run_dir}: {label} changed since import')
        instrumentation, expect, presented, latency = sophia_window(src)
        if expect != record['expect']:
            raise Refused(f'{run_dir}: bound processes differ from the import')
        evidence = 'native' if instrumentation == 'none' else 'diagnostic'
        nominal = 119.880  # postflight's observed actual_refresh_millihz, pinned in sophia_window
        start, end = 120, 2120
        jsonl = record['files']['cpu-window.jsonl']['path']
    else:
        if not (record.get('finished') is True and record.get('client_status') == 0
                and record.get('sampler_status') == 0 and record.get('client_exit_stop_ns')
                and not record.get('teardown_errors')):
            raise Refused(f'{run_dir}: run did not complete cleanly: finished={record.get("finished")} '
                          f'client={record.get("client_status")} sampler={record.get("sampler_status")} '
                          f'exit_stop={record.get("client_exit_stop_ns")} '
                          f'teardown={record.get("teardown_errors")} error={record.get("error")}')
        header, rows = parse_journal(run_dir / 'journal.tsv')
        want = dict(RECIPE, arm=CLIENT_ARM[record['stack']])
        drift = [f'{k}={header.get(k)} (want {v})' for k, v in want.items() if header.get(k) != v]
        if drift or record.get('recipe') != RECIPE:
            raise Refused(f'{run_dir}: not the pinned recipe: {", ".join(drift) or "run.json recipe differs"}')
        warmup, frames = int(RECIPE['warmup']), int(RECIPE['frames'])
        if [r['frame'] for r in rows] != list(range(1, frames + 1)):
            raise Refused(f'{run_dir}: journal rows are not 1..{frames}')
        if any(r['presented'] < 0 for r in rows):
            raise Refused(f'{run_dir}: {sum(r["presented"] < 0 for r in rows)} frames never presented')
        sources = {r['source'] for r in rows}
        nominal = float(record['rate']) if evidence == 'native' else None  # the receipt checks the actual mode
        if evidence == 'native':
            if not sources <= NATIVE_SOURCES[record['stack']]:
                raise Refused(f'{run_dir}: presentation sources {sorted(sources)} are not scanout times')
            receipt = run_dir / ('outputs.json' if record['stack'] == 'niri' else 'xrandr.txt')
            if not receipt.exists():
                raise Refused(f'{run_dir}: no output receipt {receipt.name}')
            problems = (niri_output_problems if record['stack'] == 'niri' else x_output_problems)(record, receipt)
            if problems:
                raise Refused(f'{run_dir}: output receipt: {"; ".join(problems)}')
            if record['unbound_start'] or record['unbound_end']:
                raise Refused(f'{run_dir}: unsampled stack processes {record["unbound_start"] | record["unbound_end"]}')
        else:
            notes.append(f'presentation sources {sorted(sources)}')
        start, end = warmup + 1, warmup + frames
        presented = [r['presented'] for r in rows]
        latency = [r['presented'] - r['submit'] for r in rows[1:]]
        jsonl = str(run_dir / 'cpu-window.jsonl')
    if any(b <= a for a, b in zip(presented, presented[1:])):
        raise Refused(f'{run_dir}: presentation times are not strictly increasing')
    stats = frame_stats(presented, latency, nominal)
    # Native acceptance: every counted interval is one refresh of the observed mode.
    if evidence == 'native' and (stats['missed_vblank_gaps'] or not close(stats['refresh_hz'], nominal, 0.005)):
        raise Refused(f'{run_dir}: refresh {stats["refresh_hz"]:.3f} Hz with {stats["missed_vblank_gaps"]} missed '
                      f'refreshes against the observed {nominal} Hz mode')
    entry = dict(name=name, arm=record['stack'], samples=jsonl, start_frame=start, end_frame=end,
                 anchor_start_ns=presented[0], anchor_end_ns=presented[-1],
                 expect={k: dict(pid=v['pid'], starttime=v['starttime']) for k, v in record['expect'].items()})
    return entry, dict(name=name, stack=record['stack'], mode=record['mode'], evidence=evidence,
                       run=str(run_dir), notes=notes, **stats)


def cmd_report(args):
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False, mode=0o700)
    entries, frames, refused = [], [], []
    for index, run in enumerate(args.runs):
        name = f'{index:02d}-{run.name}'
        try:
            entry, stats = plan_run(name, run.resolve())
        except (Refused, KeyError, ValueError) as e:
            refused.append(dict(name=name, run=str(run), reason=str(e)))
            continue
        native = [f for f in frames if f['evidence'] == 'native']
        if stats['evidence'] == 'native' and native and not close(stats['refresh_hz'], native[0]['refresh_hz'], 0.005):
            refused.append(dict(name=name, run=str(run), reason=f'refresh {stats["refresh_hz"]:.3f} Hz differs '
                                f'from {native[0]["name"]} at {native[0]["refresh_hz"]:.3f} Hz'))
            continue
        entries.append(entry)
        frames.append(stats)
    stacks = {e['arm'] for e in entries}
    plan = dict(max_gap_ms=20, runs=entries)
    if entries:  # differences are against Sophia when present
        plan['baseline'] = 'sophia' if 'sophia' in stacks else entries[0]['arm']
    write_json(out / 'plan.json', plan)
    analysis = out / 'cpuwin.json'
    with (out / 'cpuwin.txt').open('w') as text:
        status = subprocess.run([sys.executable, '-I', '-B', str(CPUWIN), 'analyze', str(out / 'plan.json'),
                                 '--json', str(analysis)], stdout=text, stderr=subprocess.STDOUT).returncode
    cpu = json.loads(analysis.read_text()) if analysis.exists() else dict(runs=[], refused=[], arms={})
    evidence = {f['evidence'] for f in frames}
    by_name = {r['name']: r for r in cpu['runs']}
    rows = []
    for f in frames:
        r = by_name.get(f['name'])
        rows.append(dict(f, cpu_us_per_frame=r and r['per_frame_us'],
                         per_process_us_per_frame=r and r['per_proc_per_frame_us']))
    refused += cpu.get('refused', [])
    # A difference is only reported when every requested row is native and accepted.
    native = evidence == {'native'} and not refused
    report = dict(schema=SCHEMA, evidence='native' if native else 'diagnostic',
                  rows=rows, stacks=cpu.get('arms', {}), differences=cpu.get('differences', {}) if native else {},
                  refused=refused, cpuwin_status=status)
    write_json(out / 'report.json', report)
    (out / 'report.md').write_text(render(report))
    if not getattr(args, 'quiet', False):
        print(render(report), end='')
    return 0 if status == 0 and not report['refused'] else 1


def render(report):
    fmt = lambda b: f'[{b[0]:.0f}, {b[1]:.0f}]' if b else 'refused'
    lines = [f'# Desktop CPU per frame ({report["evidence"]})', '',
             '| run | stack | mode | CPU us/frame | per process | refresh Hz | missed gaps | latency p50 us |',
             '|---|---|---|---|---|---|---|---|']
    for r in report['rows']:
        procs = ', '.join(f'{k} {fmt(v)}' for k, v in (r['per_process_us_per_frame'] or {}).items())
        lines.append(f'| {r["name"]} | {r["stack"]} | {r["mode"]} | {fmt(r["cpu_us_per_frame"])} | {procs} | '
                     f'{r["refresh_hz"]:.2f} | {r["missed_vblank_gaps"]} | {r["latency_us"]["p50"]:.0f} |')
    lines.append('')
    for stack, bounds in report['stacks'].items():
        lines.append(f'- {stack}: mean {fmt(bounds)} us/frame')
    for k, bounds in report['differences'].items():
        lines.append(f'- {k}: {fmt(bounds)} us/frame, interval of the difference of means')
    for r in report['refused']:
        lines.append(f'- REFUSED {r["name"]}: {r["reason"]}')
    if report['evidence'] != 'native':
        lines.append('- Diagnostic only: a row is a smoke, instrumented or refused, so no stack difference is reported.')
    lines.append('- CPU is the bound processes only (each thread group, not its children). The start and end '
                 'process snapshots are a boundary inventory, not exhaustive: transient children and setsid '
                 'helpers between them are missed. This is not whole-desktop CPU.')
    lines.append('- Pixel producers differ: the external clients fill shared memory on the CPU, while the '
                 'Sophia peer renders on the GPU and copies a readback into its owned ring. Server and client '
                 'rows are reported separately; a combined figure describes these concrete paths, not '
                 'intrinsic compositor efficiency.')
    lines.append('- Bounds are tick attribution intervals, not confidence intervals. Latency is each '
                 "stack's own submit-to-presentation span, not a common end-to-end measure.")
    return '\n'.join(lines) + '\n'


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest='cmd', required=True)
    r = sub.add_parser('run')
    r.add_argument('--stack', choices=('niri', 'xlibre'), required=True)
    r.add_argument('--mode', choices=('native', 'smoke'), required=True)
    r.add_argument('--out', type=Path, required=True)
    r.add_argument('--output', help='connector, e.g. DP-1 (native)')
    r.add_argument('--mode-size', default='1920x1080')
    r.add_argument('--rate', default='119.880')
    r.add_argument('--disable-output', action='append', default=None,
                   help=f'repeatable, niri only; native default: the rig\'s other connectors '
                        f'{RIG_OTHER_OUTPUTS}. Native X turns off every other output the server lists.')
    r.add_argument('--niri-log', default='warn', help='RUST_LOG for niri (its source default is debug)')
    r.add_argument('--niri', default='/usr/bin/niri')
    r.add_argument('--xlibre-prefix', default=str(Path.home() / '.local/opt/xlibre-56be9f4320ef'))
    r.add_argument('--smoke-xserver', default='/usr/libexec/Xorg')
    r.add_argument('--smoke-wayland', default='/usr/bin/river', help='wlroots 0.3 river for smoke')
    r.add_argument('--bus-id', default='PCI:3:0:0', help='native X GPU')
    r.add_argument('--kmsdev', default='/dev/dri/card0')
    r.add_argument('--seat', default='seat0')
    r.add_argument('--wm', help='X window manager binary, e.g. xmonad')
    r.add_argument('--compositor', help='X compositor binary, run with -n')
    r.add_argument('--settle', type=float, default=1.0)
    r.add_argument('--timeout', type=float, default=120)
    r.add_argument('--inside', action='store_true', help=argparse.SUPPRESS)
    r.set_defaults(fn=cmd_run)
    s = sub.add_parser('import-sophia')
    s.add_argument('--out', type=Path, required=True)
    s.add_argument('--allow-instrumented', action='store_true',
                   help='import a capture-profile run as a diagnostic row')
    s.add_argument('run', type=Path)
    s.set_defaults(fn=cmd_import_sophia)
    p = sub.add_parser('report')
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('runs', type=Path, nargs='+')
    p.set_defaults(fn=cmd_report)
    args = ap.parse_args()
    if args.cmd == 'run':
        if args.disable_output is None:
            args.disable_output = RIG_OTHER_OUTPUTS if args.mode == 'native' else []
        if args.mode == 'native' and not args.output:
            ap.error('native runs name their connector with --output')
    return args.fn(args)


if __name__ == '__main__':
    raise SystemExit(main())
