"""One untimed static Lom capture, run beside Session in its namespaces."""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import select
import struct
import time

HERE = Path(__file__).resolve().parent


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


c = load('private_capture', HERE.parent / 'gpu/capture_sophia.py')
r = c.r
pixels = load('opaque_pixels', HERE / 'pixels.py')
static = load('static_lom', HERE / 'static.py')


def capture(expected, session, directory, deadline, evidence):
    result = capture_held(expected, session, directory, deadline, evidence,
                          lambda log: static.snapshot(log, expected['arm']), [])
    result['status'] = 'static_capture_verified'
    return result


def capture_held(expected, session, directory, deadline, evidence, snapshot, previous):
    before, log_identity = c.log_bytes(expected['session_log'])
    held = snapshot(before)
    output = static.positive(held['head']['output'])
    endpoint, endpoint_identity = c.launch_socket(before, session)
    prior = {item['receipt']['launch_generation']: {'pam', 'json', 'stderr'} for item in previous}
    if r.inventory(directory) != prior:
        raise ValueError('capture directory differs from prior deliveries' if previous
                         else 'capture directory must be unused')
    def check_previous():
        for item in previous:
            generation = item['receipt']['launch_generation']
            if (hashlib.sha256(r.read_at(directory, f'capture-{generation}.json', 4096)).hexdigest()
                    != item['receipt_sha256']
                    or hashlib.sha256(r.read_at(directory, f'capture-{generation}.pam', 1920*1080*4+256)).hexdigest()
                    != item['receipt']['pam_sha256']
                    or r.read_at(directory, f'capture-{generation}.stderr', 0) != b''):
                raise ValueError('prior delivery changed')
    check_previous()
    notification = c.watch(directory)
    try:
        argv = [session.executable, 'launch', '--socket', endpoint, '--recipe', 'capture',
                '--', session.executable, 'capture', str(output), '--diagnostic-stream']
        c.launch(argv, session, min(deadline, time.monotonic()+c.LAUNCH_TIMEOUT), evidence)
        deadline = min(deadline, time.monotonic()+c.RECEIPT_TIMEOUT)
        generation = None
        while True:
            session.check()
            groups = r.inventory(directory)
            # Filenames use Session's global launch generation, not the capture count.
            if any(groups.get(key) != value for key, value in prior.items()):
                raise ValueError('prior delivery changed')
            fresh = groups.keys() - prior.keys()
            if generation is None and len(fresh) == 1:
                generation = next(iter(fresh))
                if generation <= max(prior, default=0):
                    raise ValueError('stale capture attempt')
            if (fresh and fresh != {generation}) or (generation is not None and not fresh):
                raise ValueError('unexpected capture attempt')
            if 'refused' in groups.get(generation, set()):
                raise ValueError('private capture refused')
            if 'json' in groups.get(generation, set()):
                checksum = held['candidate']['checksum']
                height = int(held['candidate']['height'])
                hashed = held['candidate']['schema'] == '1'
                result = r.verify_delivery(directory, generation,
                    dict(output=output, output_generation=expected['output_generation']),
                    session.identity, lambda pam: pixels.check(pam, checksum, height) if hashed
                    else pixels.capture_strip(pam, height))
                after, after_identity = c.log_bytes(expected['session_log'])
                if after_identity != log_identity or not after.startswith(before):
                    raise ValueError('Session log replaced or truncated')
                if snapshot(after) != held:
                    raise ValueError('static scene changed during capture')
                if r.inventory(directory) != {**prior, generation: {'pam', 'json', 'stderr'}}:
                    raise ValueError('concurrent capture attempt')
                check_previous()
                r.advance(previous[-1]['receipt'] if previous else None, result['receipt'])
                current = os.stat(endpoint, follow_symlinks=False)
                if (current.st_dev, current.st_ino) != endpoint_identity:
                    raise ValueError('launch endpoint changed')
                session.check()
                for name, data in [('before.log', before), ('after.log', after)]:
                    with (evidence / name).open('xb') as stream:
                        stream.write(data)
                return dict(schema=1, status='held_capture_verified', arm=expected['arm'],
                    capture=result, scene=held,
                    before_sha256=hashlib.sha256(before).hexdigest(),
                    after_sha256=hashlib.sha256(after).hexdigest(),
                    session=dict(pid=session.pid, starttime=session.starttime,
                                 namespaces=c.namespace_ids(session.pid)),
                    join='One serialized new private delivery; transaction 1 is a format check. '
                         'Same held shell candidate brackets capture; capture frame and '
                         'shell presentation epoch are separate domains.',
                    limits='Launch custody only, not per-writer authentication; log joins '
                           'are consistency evidence. No PNG normalization or vertical flip. '
                           'Full-run postflight must validate the declared scene sequence.')
            remaining = deadline-time.monotonic()
            if remaining <= 0 or not select.select([notification, session.fd], [], [], max(0, remaining))[0]:
                raise ValueError('capture terminal receipt deadline')
            try:
                events = os.read(notification, 65536)
            except BlockingIOError:
                continue
            offset = 0
            while offset < len(events):
                if len(events)-offset < 16:
                    raise ValueError('truncated capture watch event')
                _, mask, _, length = struct.unpack_from('iIII', events, offset)
                offset += 16 + length
                if offset > len(events) or mask & (0x4000 | 0x400 | 0x800 | 0x8000):
                    raise ValueError('capture watch invalidated or overflowed')
    finally:
        os.close(notification)


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
    if (type(expected) is not dict or expected.keys() != {'schema', 'session', 'session_log',
            'delivery_directory', 'output_generation', 'arm'}
            or type(expected['schema']) is not int or expected['schema'] != 1
            or expected['arm'] not in static.STARTUP):
        raise ValueError('invalid static capture declaration')
    r.uint(expected['output_generation'], True)
    r.absolute(expected['session_log'])
    args.evidence.mkdir(mode=0o700)
    session = c.Session(expected['session'])
    directory = None
    try:
        directory = c.private_directory(expected['delivery_directory'])
        result = capture(expected, session, directory, time.monotonic()+c.DEADLINE, args.evidence)
        with (args.evidence/'result.json').open('x') as stream:
            json.dump(result, stream, indent=2)
            stream.write('\n')
        print('lom_pixels_verified arm=' + expected['arm'], flush=True)
    finally:
        if directory is not None:
            os.close(directory)
        session.close()


if __name__ == '__main__':
    main()
