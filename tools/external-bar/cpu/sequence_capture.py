"""Three designated phase captures beside Session; the held host permits only three attempts."""
import argparse
import json
import os
from pathlib import Path
import time

import importlib.util


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


capture, sequence = load('capture'), load('sequence')
c, r = capture.c, capture.r
HOLD_SECONDS = 10
CONTROLLER_SECONDS = len(sequence.PHASES) * HOLD_SECONDS + 30


def declared(expected):
    if (type(expected) is not dict or expected.keys() != {'schema', 'session', 'session_log',
            'delivery_directory', 'output_generation', 'arm', 'group'}
            or type(expected['schema']) is not int or expected['schema'] != 1
            or expected['arm'] not in sequence.ARMS
            or type(expected['group']) is not int or expected['group'] not in range(3)):
        raise ValueError('invalid sequence capture declaration')
    r.uint(expected['output_generation'], True)
    r.absolute(expected['session_log'])
    return sequence.GROUPS[expected['group']]


def run(expected, session, directory, deadline, evidence):
    started_ns = time.monotonic_ns()
    phases = declared(expected)
    if r.inventory(directory):
        raise ValueError('sequence capture directory must be unused')
    results = []
    for phase in phases:
        while True:
            session.check()
            log, _ = c.log_bytes(expected['session_log'])
            state = sequence.state(log, expected['arm'])
            if len(state['installed']) > phase + 1:
                raise ValueError('missed designated phase; no retry')
            if len(state['completed']) == phase + 1:
                break
            if time.monotonic() >= deadline:
                raise ValueError('phase readiness deadline')
            time.sleep(min(0.05, max(0, deadline-time.monotonic())))
        phase_dir = evidence / f'phase-{phase}'
        phase_dir.mkdir(mode=0o700)
        started = time.time_ns()
        result = capture.capture_held(expected, session, directory, deadline, phase_dir,
            lambda log: sequence.snapshot(log, expected['arm'], phase),
            [item['capture'] for item in results])
        result.update(phase=phase, name=sequence.PHASES[phase],
                      capture_started_ns=started, capture_verified_ns=time.time_ns())
        with (phase_dir/'result.json').open('x') as stream:
            json.dump(result, stream, indent=2); stream.write('\n')
        results.append(result)
    return dict(schema=1, status='designated_captures_verified', arm=expected['arm'],
                group=expected['group'], phases=list(phases), captures=results,
                controller_started_monotonic_ns=started_ns,
                controller_finished_monotonic_ns=time.monotonic_ns(),
                limits='Captured strips only; no producer checksum. Final postflight must require '
                       'all nine phases complete, even phases not captured by this group. '
                       'Capture presentation generation and shell presentation epoch are separate domains.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--expected', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    with args.expected.open('rb') as stream:
        data = stream.read(16385)
    if len(data) > 16384:
        raise ValueError('oversized capture declaration')
    expected = r.record(data)
    declared(expected)
    args.evidence.mkdir(mode=0o700)
    with (args.evidence/'expected.json').open('xb') as stream: stream.write(data)
    session = c.Session(expected['session'])
    directory = None
    try:
        directory = c.private_directory(expected['delivery_directory'])
        result = run(expected, session, directory, time.monotonic()+CONTROLLER_SECONDS, args.evidence)
        with (args.evidence/'result.json').open('x') as stream:
            json.dump(result, stream, indent=2); stream.write('\n')
        print('lom_region_captures_verified arm=' + expected['arm'], flush=True)
    finally:
        if directory is not None: os.close(directory)
        session.close()


if __name__ == '__main__':
    main()
