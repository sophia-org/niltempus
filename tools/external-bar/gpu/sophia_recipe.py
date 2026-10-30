"""Fixed untimed Sophia colour/capture configuration; no native qualification.

Preparation and preflight never launch Session. Run is only for the reviewed
unprivileged PAM/VT handoff; release/feature provenance and sealing are external.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import signal

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('sophia_hold_wrapper', HERE/'run_hold_sophia.py')
wrapper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wrapper)
spec = importlib.util.spec_from_file_location('native_colour', HERE/'run_colour_external.py')
external = importlib.util.module_from_spec(spec)
spec.loader.exec_module(external)
ARTIFACTS = {'host', 'peer', 'supervisor', 'wm', 'factotum', 'pam'}
CORE = '''/- kdl-version 2
schema 2
session {
    application-catalog "shell-timing" launch-policy="trusted-host" {}
    namespace-policy {
        recipe "capture" profile="observer" developer-launch=#true {
            capture-output "pci-0000:03:00.0/DP-1"
        }
    }
}
'''
PEER = dict(wire='files', output=1, frames=3, height=24, hold_msec=0,
            source='dma_buf', producer='gpu', capture_profile=False)


def load(path):
    return wrapper.h.r.record(external.read(path, 1024*1024))


def validate(inputs):
    if (type(inputs) is not dict or inputs.keys() != {'schema', 'artifacts', 'output_generation'}
        or type(inputs['schema']) is not int or inputs['schema'] != 1
        or type(inputs['artifacts']) is not dict or inputs['artifacts'].keys() != ARTIFACTS):
        raise ValueError('exact Sophia diagnostic input set required')
    wrapper.h.r.uint(inputs['output_generation'], True)
    for pin in inputs['artifacts'].values():
        wrapper.h.pin_artifact(pin)


def profile(peer, config):
    # JSON quoting is valid for these bounded, control-free KDL path strings.
    return '''schema 1
shell { enabled #true; content #true; content-input #true; panel 24; }
session {
    control "host-admin"
    exclude-gpu "pci-0000:16:00.0"
    shell-component "panel" "bar" {
        executable %s
        config %s
        gpu "direct"
        transport "9p2000.L"
        reservation "top" 24
    }
    application-catalog "shell-timing"
    startup
}
input { inherit-sophia #true; }
output {
    availability "adaptive"
    inherit-sophia #false
    named "DP-1" {
        enabled #true
        focus-at-startup #true
        gpu "pci-0000:03:00.0"
        mode "1920x1080@120"
        vrr 0
    }
    named "HDMI-A-1" { enabled #false; }
}
broker { enabled #false; }
''' % (json.dumps(peer, ensure_ascii=False), json.dumps(str(config), ensure_ascii=False))


def materialize(inputs, directory):
    validate(inputs)
    a = inputs['artifacts']
    argv = ['session', 'run', '--session-mode=normal', '--native-scanout', '--display=:94',
            '--input-seat=seat0', '--max-runtime-ms=280000', f'--config={directory/"core.kdl"}',
            f'--desktop-profile={directory/"desktop.kdl"}', '--wm-process='+a['wm']['executable'],
            '--factotum-agent='+a['factotum']['executable'], '--factotum-pam-helper='+a['pam']['executable']]
    recipe = dict(schema=1, host=a['host'], peer=a['peer'], supervisor=a['supervisor'],
                  output_generation=inputs['output_generation'], argv=argv)
    wrapper.validate(recipe)
    return {'core.kdl': CORE, 'desktop.kdl': profile(a['peer']['executable'], directory/'peer.json'),
            'peer.json': json.dumps(PEER, indent=2)+'\n',
            'recipe.json': json.dumps(recipe, indent=2)+'\n'}


def prepare(inputs, directory):
    directory = Path(wrapper.h.r.absolute(str(directory)))
    files = materialize(inputs, directory)
    directory.mkdir(mode=0o700)
    files['inputs.json'] = json.dumps(inputs, indent=2)+'\n'
    for name, text in files.items():
        with (directory/name).open('x') as stream:
            stream.write(text)
        (directory/name).chmod(0o600)
    return verify(directory)


def verify(directory):
    directory = Path(wrapper.h.r.absolute(str(directory)))
    fd = wrapper.h.c.private_directory(str(directory))
    os.close(fd)
    inputs = load(directory/'inputs.json')
    for name, expected in materialize(inputs, directory).items():
        if external.read(directory/name, 65536) != expected.encode():
            raise ValueError('fixed Sophia diagnostic input changed: '+name)
    return load(directory/'recipe.json')


def environment():
    keys = ('HOME', 'USER', 'LOGNAME', 'LANG', 'XDG_RUNTIME_DIR',
            'XDG_SESSION_ID', 'XDG_SEAT', 'XDG_VTNR')
    return dict({k:os.environ[k] for k in keys if k in os.environ},
                PATH='/usr/bin:/bin', LC_ALL='C', SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE='1')


def preflight(directory, evidence):
    recipe = verify(directory)
    evidence = Path(wrapper.h.r.absolute(str(evidence)))
    evidence.mkdir(mode=0o700)
    # Exercise the actual private-delivery flag too; parsing must not create it.
    capture = evidence/'uncreated-captures'
    argv = [recipe['host']['executable'], *recipe['argv'],
            f'--diagnostic-capture-directory={capture}', '--validate-session-args']
    output = external.command(argv, recipe['host'], environment(), evidence/'arguments', 15,
                              limit=4096)
    accepted = f'sophia_live_session_args schema=1 status=accepted arguments={len(recipe["argv"])+1}\n'.encode()
    if capture.exists() or output != accepted:
        raise ValueError('argument preflight did not return exact acceptance without capture creation')
    external.write(evidence/'result.json', dict(schema=1, status='arguments_accepted', argv=argv,
        host=recipe['host'], input_sha256={n:hashlib.sha256((directory/n).read_bytes()).hexdigest()
            for n in ('inputs.json','core.kdl','desktop.kdl','peer.json','recipe.json')},
        limits='Argument/config parser only. Does not prove build closure, peer behavior, live capture, actual 119.880 Hz, GPU import or PAM cleanup.'))


def run(directory, evidence):
    recipe = verify(directory)
    if os.getuid() != os.geteuid() or os.getuid() == 0 or re.fullmatch(r'/dev/tty[1-9][0-9]*', os.ttyname(0)) is None:
        raise ValueError('Sophia diagnostic requires unprivileged attended PAM/text VT')
    # Session and its driver are siblings; never enter another process namespace.
    clean, saved = environment(), dict(os.environ)
    def interrupted(_signal, _frame):
        raise InterruptedError('Sophia diagnostic interrupted')
    handlers = {sig:signal.signal(sig, interrupted) for sig in (signal.SIGTERM, signal.SIGINT)}
    try:
        os.environ.clear()
        os.environ.update(clean)
        return wrapper.run(recipe, Path(wrapper.h.r.absolute(str(evidence))))
    finally:
        os.environ.clear()
        os.environ.update(saved)
        for sig, handler in handlers.items():
            signal.signal(sig, handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='action', required=True)
    p = commands.add_parser('prepare')
    p.add_argument('--inputs', type=Path, required=True)
    p.add_argument('--directory', type=Path, required=True)
    for name in ('preflight', 'run'):
        p = commands.add_parser(name)
        p.add_argument('--directory', type=Path, required=True)
        p.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    if args.action == 'prepare':
        prepare(load(args.inputs), args.directory)
    else:
        globals()[args.action](args.directory, args.evidence)


if __name__ == '__main__':
    main()
