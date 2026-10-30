"""Stage an unsealed colour arm using the exercised CPU02 PAM handoff.

Staging never invokes sudo, a display server or the handoff. A reviewed package
must supply MANIFEST.sha256 and release/link provenance before native execution.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import signal

import importlib.util

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('native_colour', HERE/'run_colour_external.py')
w = importlib.util.module_from_spec(spec)
spec.loader.exec_module(w)
SEED = Path('/home/niltempus/.local/state/t331/desktop-matched-cpu-02/ssh/niri1')
CONTROL = Path('/home/niltempus/.local/state/t331/t331-crunch-report-02/remote/remote.py')
TTY = Path('/home/niltempus/dev/sophia/tools/sophia_tty_mode.py')
PYTHON = '/usr/bin/python3.14'
SEED_HASHES = {
    'ssh-run.py': 'ddad631207c4310b76154238998d670fe3056d54ecca74ef30638c11d1979aa7',
    'vt_session.py': '734d9565d5de962b9fadae464a3f10b4b009023039e8fa86f049966019b7877c',
    'remote_user.py': '623fb55e7f53e3987082af4e8228e11025625b3abf29deceef3f08bdbee0ef1d',
    'ssh-pins.json': '0ed7cea60af751d5970a7b52958b8dfad4335eaeb800716045398434ab055f1f',
}
SEALED_FILES = {'ssh-run.py', 'vt_session.py', 'remote_user.py', 'run.sh',
                'run-path.txt', 'native-command.json', 'recipe.json',
                'ssh-pins.json', 'preparation.json'}


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def load(path):
    return w.h.r.record(w.read(path, 1024*1024))


def replace_once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('handoff template changed: '+old)
    return text.replace(old, new)


def seed_files():
    result = {}
    for name, expected in SEED_HASHES.items():
        data = w.read(SEED/name, 1024*1024)
        if hashlib.sha256(data).hexdigest() != expected:
            raise ValueError('handoff seed hash changed: '+name)
        result[name] = data.decode()
    return result


def stage(recipe, destination, run):
    return _stage(recipe, destination, run)


def stage_sophia(directory, destination, run):
    sophia = w.colour.load('sophia_recipe')
    directory = Path(w.h.r.absolute(str(directory)))
    recipe = sophia.verify(directory)
    return _stage(recipe, destination, run, directory)


def _stage(recipe, destination, run, sophia_directory=None):
    """Create only new files. Failed preparation is retained, never repaired."""
    if sophia_directory is None:
        w.validate(recipe)
        artifacts = recipe['pins']
        stack = recipe['stack']
    else:
        artifacts = load(sophia_directory/'inputs.json')['artifacts']
        stack = 'sophia'
    destination = Path(w.h.r.absolute(str(destination)))
    run = Path(w.h.r.absolute(str(run)))
    if destination == run or destination in run.parents or run in destination.parents:
        raise ValueError('handoff and run directories must be separate')
    if destination.exists() or run.exists() or not run.parent.is_dir():
        raise ValueError('handoff/run must be fresh with an existing run parent')
    if stack == 'niri' and len(os.fsencode(run/'runtime'))+40 > 107:
        raise ValueError('short run path required for niri sockets')
    seed = seed_files()
    inherited = w.h.r.record(seed['ssh-pins.json'].encode())
    # Preserve the exercised host-helper closure, not old benchmark executables.
    pins = {path: digest for path, digest in inherited.items()
            if path.startswith(('/usr/', '/etc/', '/lib', '/nix/', '/opt/'))
            or path in (str(CONTROL), str(TTY), PYTHON)}
    if str(CONTROL) not in pins or PYTHON not in pins or str(TTY) not in pins:
        raise ValueError('inherited handoff helper closure is incomplete')
    for path, digest in pins.items():
        if sha(path) != w.h.r.digest(digest):
            raise ValueError('inherited helper drift: '+path)
    for pin in artifacts.values():
        path, digest = pin['executable'], pin['sha256']
        if path in pins and pins[path] != digest:
            raise ValueError('artifact conflicts with inherited helper pin')
        pins[path] = digest
    if sophia_directory is not None:
        for name in ('inputs.json', 'core.kdl', 'desktop.kdl', 'peer.json', 'recipe.json'):
            pins[str(sophia_directory/name)] = sha(sophia_directory/name)
    for path in HERE.glob('*.py'):
        if path.is_symlink():
            raise ValueError('source helper must not be a symlink')
        pins[str(path)] = sha(path)
    for name in ('niri.kdl', 'xorg.conf'):
        pins[str(HERE/name)] = sha(HERE/name)

    gate = (
        "# Complete seal and external pins precede privileged helper import.\n"
        "seal_rows = (BASE / 'MANIFEST.sha256').read_text().splitlines()\n"
        "seal_names = [row[66:] for row in seal_rows]\n"
        f"if len(seal_rows) != {len(SEALED_FILES)} or set(seal_names) != set({sorted(SEALED_FILES)!r}):\n"
        "    raise RuntimeError('Incomplete handoff seal')\n"
        "subprocess.run(['/usr/bin/sha256sum', '--strict', '--status', '-c', 'MANIFEST.sha256'],\n"
        "               cwd=BASE, check=True)\n"
        "for pinned_path, pinned_sha in json.loads((BASE / 'ssh-pins.json').read_text()).items():\n"
        "    if hashlib.sha256(Path(pinned_path).read_bytes()).hexdigest() != pinned_sha:\n"
        "        raise RuntimeError('Pinned handoff input changed: ' + pinned_path)\n"
    )
    ssh = replace_once(seed['ssh-run.py'], 'CONTROL = Path(', gate+'CONTROL = Path(')
    ssh = replace_once(ssh, "    run_path = Path(",
        "    if (BASE / 'attempt.json').exists():\n"
        "        raise RuntimeError('Handoff attempt already consumed')\n    run_path = Path(")
    ssh = replace_once(ssh, 'VT bound 180s, outer bound 240s', 'VT bound 360s, outer bound 420s')
    ssh = replace_once(ssh, 'SOPHIA_SCREEN_BOUND=180', 'SOPHIA_SCREEN_BOUND=360')
    ssh = replace_once(ssh, 'time.monotonic()+240', 'time.monotonic()+420')
    ssh = replace_once(ssh, '    service = ',
        "    with (BASE / 'attempt.json').open('x') as attempt:\n"
        f"        json.dump({{'schema': 1, 'kind': 'untimed_colour', 'stack': {stack!r}}}, attempt)\n"
        "        os.fchown(attempt.fileno(), 1000, 1000)\n    service = ")
    user = replace_once(seed['remote_user.py'], 'Starting pinned t339 matched GPU comparison',
                        'Starting pinned untimed GPU colour diagnostic')
    argv = [PYTHON, '-I', '-B', str(HERE/'run_colour_external.py'),
            '--recipe', str(destination/'recipe.json'), '--evidence', str(run)]
    if sophia_directory is not None:
        argv = [PYTHON, '-I', '-B', str(HERE/'sophia_recipe.py'), 'run',
                '--directory', str(sophia_directory), '--evidence', str(run)]
        recipe = dict(schema=1, stack='sophia', directory=str(sophia_directory), wrapper=recipe)
    files = {'ssh-run.py': ssh, 'vt_session.py': seed['vt_session.py'], 'remote_user.py': user,
             'run.sh': '#!/usr/bin/env bash\nset -euo pipefail\numask 077\n'
                       'export PATH=/usr/bin:/bin\nexec '+shlex.join(argv)+'\n',
             'run-path.txt': str(run)+'\n', 'native-command.json': json.dumps(argv)+'\n',
             'recipe.json': json.dumps(recipe, indent=2)+'\n'}
    destination.mkdir(mode=0o700)
    for name, text in files.items():
        with (destination/name).open('x') as stream:
            stream.write(text)
        (destination/name).chmod(0o600)
        pins[str(destination/name)] = sha(destination/name)
    w.write(destination/'ssh-pins.json', pins)
    w.write(destination/'preparation.json', dict(schema=1, status='staged_unsealed',
        seed=str(SEED), seed_sha256=SEED_HASHES, command=argv, stack=stack,
        budgets_seconds=dict(wrapper=300, pam=360, outer=420),
        limits=['No launch or release/link qualification. MANIFEST.sha256 deliberately absent.',
                'PAM/VT code unchanged; no namespace entry or grant added.',
                'Inherited host hashes are checked, not repinned on drift; shared libraries/data remain a package closure responsibility.',
                'Sophia configuration is explicit and pinned; authenticated protected execution remains unqualified.',
                'Real import refusal and delayed release remain separate device controls.']))
    for path in destination.iterdir():
        path.chmod(0o600)
    return destination


def zero(value):
    return type(value) is int and value == 0


def qualify_cleanup(base):
    """Read terminal evidence only; never turn local pixel success into a native pass."""
    base = Path(base)
    vt, user, outer = [load(base/name) for name in
                       ('vt-session-result.json', 'remote-user-result.json', 'outer-cleanup.json')]
    cleanup = vt.get('cleanup', {})
    if (vt.get('stage') != 'pam_cleanup_done' or vt.get('timed_out') is not False
        or not zero(vt.get('child_status')) or vt.get('residual_pids') != []
        or not zero(user.get('exit_status')) or type(cleanup) is not dict
        or cleanup.keys() != {'close', 'credentials', 'pam_end', 'tty_restored'}
        or not all(zero(cleanup[k]) for k in ('close', 'credentials', 'pam_end'))
        or cleanup['tty_restored'] is not True
        or any(outer.get(k) != [] for k in ('final_occupants', 'final_sessions', 'killed', 'killed_sessions'))):
        raise ValueError('outer PAM/VT cleanup refused')
    run = Path(w.h.r.absolute((base/'run-path.txt').read_text().strip()))
    if (run/'refused.json').exists():
        raise ValueError('local colour wrapper refused')
    result, local = load(run/'result.json'), load(run/'cleanup.json')
    recipe = load(base/'recipe.json')
    extra = []
    if recipe.get('stack') == 'sophia':
        if (recipe.keys() != {'schema', 'stack', 'directory', 'wrapper'}
            or type(recipe['schema']) is not int or recipe['schema'] != 1
            or w.colour.load('sophia_recipe').verify(Path(recipe['directory'])) != recipe['wrapper']):
            raise ValueError('Sophia handoff configuration changed')
        if (result.get('status') != 'driver_completed'
            or not zero(result.get('driver_exit')) or type(result.get('host_exit')) is not int
            or result['host_exit'] not in (0, -signal.SIGTERM)
            or local.get('driver_forced') is not False or local.get('host_forced') is not False
            or type(local.get('driver_exit')) is not int or type(local.get('host_exit')) is not int
            or local['driver_exit'] != result['driver_exit'] or local['host_exit'] != result['host_exit']
            or result.get('host_argv') != [recipe['wrapper']['host']['executable'],
                *recipe['wrapper']['argv'], f'--diagnostic-capture-directory={run/"captures"}']
            or (run/'driver/refused.json').exists()):
            raise ValueError('Sophia local cleanup refused')
        extra = [run/'driver/result.json', run/'driver/captures_verified.json']
        terminal, captures = map(load, extra)
        if (terminal.get('status') != 'session_exited_after_captures'
            or captures.get('status') != 'captures_verified'
            or type(captures.get('records')) is not list or len(captures['records']) != 3):
            raise ValueError('Sophia driver did not finish three captures')
        environment_path = run/'environment.json'
        environment = load(environment_path)
        raw_path, stderr_path = run/'environment.ndjson', run/'environment.stderr'
        raw = w.read(raw_path, 3*65536)
        if (result.get('environment_sha256') != sha(environment_path)
                or environment.get('status') != 'startup_environment_verified'
                or not zero(environment.get('receiver_exit')) or w.read(stderr_path, 4096)
                or environment.get('input_sha256') != hashlib.sha256(raw).hexdigest()
                or any(environment['host'].get(k) != v for k, v in recipe['wrapper']['host'].items())
                or any(environment['receiver'].get(k) != v for k, v in recipe['wrapper']['environment_receiver'].items())):
            raise ValueError('Sophia environment evidence differs from the executed recipe')
        parsed = w.colour.load('sophia_environment').qualify(raw, environment['host'],
            environment['start_usec'], environment['end_usec'])
        if any(environment.get(k) != v for k, v in parsed.items()):
            raise ValueError('Sophia environment reduction changed')
        extra += [environment_path, raw_path, stderr_path]
    else:
        qualify_external_cleanup(recipe, result, local)
    return dict(schema=1, status='handoff_cleanup_verified', stack=recipe['stack'],
                input_sha256={str(p): sha(p) for p in
                    [base/n for n in ('vt-session-result.json', 'remote-user-result.json', 'outer-cleanup.json',
                                      'recipe.json', 'run-path.txt')] + [run/'result.json', run/'cleanup.json'] + extra},
                limits='Cleanup receipts only; no native device/pixel/import qualification or performance claim.')


def qualify_external_cleanup(recipe, result, local):
    if (result.get('status') != 'colour_source_path_completed'
        or result.get('stack') != recipe.get('stack')):
        raise ValueError('local colour wrapper did not complete')
    rows = local.get('children')
    if type(rows) is not list or len(rows) != 2:
        raise ValueError('local cleanup needs client and server')
    expected = [result['client']['pid'], result['server']['pid']]
    if any(type(pid) is not int or pid <= 0 for pid in expected) or expected[0] == expected[1]:
        raise ValueError('local client/server identities must be distinct')
    for i, row in enumerate(rows):
        if (type(row.get('pid')) is not int or row['pid'] <= 0 or row['pid'] != expected[i]
            or row.get('forced') is not False or row.get('unexpected_server_exit') is not False
            or 'error' in row or type(row.get('rc')) is not int
            or row['rc'] not in ((0,) if i == 0 else (0, -signal.SIGTERM))):
            raise ValueError('local child cleanup refused')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='action', required=True)
    prepare = commands.add_parser('stage')
    prepare.add_argument('--recipe', type=Path, required=True)
    prepare.add_argument('--destination', type=Path, required=True)
    prepare.add_argument('--run', type=Path, required=True)
    prepare = commands.add_parser('stage-sophia')
    prepare.add_argument('--directory', type=Path, required=True)
    prepare.add_argument('--destination', type=Path, required=True)
    prepare.add_argument('--run', type=Path, required=True)
    check = commands.add_parser('cleanup')
    check.add_argument('directory', type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    if args.action == 'stage':
        stage(load(args.recipe), args.destination, args.run)
    elif args.action == 'stage-sophia':
        stage_sophia(args.directory, args.destination, args.run)
    else:
        print(json.dumps(qualify_cleanup(args.directory), indent=2))


if __name__ == '__main__':
    main()
