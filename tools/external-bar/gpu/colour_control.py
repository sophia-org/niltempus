"""Untimed external-client READY/capture/ACK sequence; capture provenance is caller-owned.

The native wrapper must pin the spawned client, inherited socket custody and
compositor/output, and bound the capture callback. This module never launches a
compositor or chooses a screenshot tool. Captures are consumer RGBA PAM bytes.
"""
import importlib.util
import json
from pathlib import Path
import re
import select
import time


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


pixels = load('pixels')
records = load('capture_receipt')
FIELDS = {'schema', 'status', 'generation', 'pattern', 'presented_ns', 'nonce'}


def receive(connection, pidfd, deadline):
    data = bytearray()
    while True:
        left = deadline - time.monotonic()
        if left <= 0:
            raise ValueError('external hold deadline')
        ready = select.select([connection, pidfd], [], [], left)[0]
        # A final DONE may already be buffered when the client exits. The caller
        # still requires a successful reap; a missing record always refuses.
        if connection not in ready:
            raise ValueError('external client exited or hold deadline')
        part = connection.recv(1025)
        if not part:
            raise ValueError('external hold EOF')
        data.extend(part)
        if len(data) > 1024:
            raise ValueError('external hold record bound')
        if b'\n' in data:
            if data.index(b'\n') != len(data)-1:
                raise ValueError('early or multiple external hold records')
            return records.record(data)


def ready(value, generation, previous, nonces):
    if type(value) is not dict or value.keys() != FIELDS:
        raise ValueError('external READY fields')
    for key in ('schema', 'generation', 'pattern', 'presented_ns'):
        records.uint(value[key], True)
    if (value['schema'] != 1 or value['status'] != 'ready' or value['generation'] != generation
            or value['pattern'] != (1, 2, 1)[generation-1]
            or type(value['nonce']) is not str or re.fullmatch('[0-9a-f]{32}', value['nonce']) is None
            or value['nonce'] in nonces
            or value['presented_ns'] <= previous):
        raise ValueError('stale or invalid external READY')


def run(connection, pidfd, capture, check_identity):
    """capture(generation, pattern) must return bounded PAM bytes within 60 seconds.

    check_identity verifies the live native client/compositor/output binding.
    Neither callback may restart a client, retry a capture, or infer the pattern.
    Caller must require DONE, clean client exit and exact consumer-release receipts.
    """
    result, nonces, previous = [], set(), 0
    for generation in (1, 2, 3):
        check_identity()
        value = receive(connection, pidfd, time.monotonic()+30)
        ready(value, generation, previous, nonces)
        if select.select([connection, pidfd], [], [], 0)[0]:
            raise ValueError('external client advanced/exited before capture')
        started = time.monotonic()
        image = capture(generation, value['pattern'])
        if time.monotonic()-started > 60:
            raise ValueError('external capture exceeded callback budget')
        oracle = pixels.check(image, value['pattern'])
        check_identity()
        if select.select([connection, pidfd], [], [], 0)[0]:
            raise ValueError('external client advanced/exited before ACK')
        ack = {'schema': 1, 'status': 'ack', 'generation': generation, 'nonce': value['nonce']}
        data = (json.dumps(ack, separators=(',', ':'))+'\n').encode()
        if connection.send(data) != len(data):
            raise ValueError('partial external ACK; no retry')
        result.append({'ready': value, 'pixels': oracle, 'ack': ack})
        nonces.add(value['nonce'])
        previous = value['presented_ns']
    done = receive(connection, pidfd, time.monotonic()+10)
    if (type(done) is not dict or done != {'schema': 1, 'status': 'done', 'generation': 3}
            or type(done['schema']) is not int or type(done['generation']) is not int):
        raise ValueError('external DONE required')
    return {'schema': 1, 'status': 'pixel_sequence_verified', 'records': result,
            'limits': 'Caller owns compositor/output/capture provenance and final reap/release proof. '
                      'Socketpair custody is not per-message writer identity; no performance claim.'}
