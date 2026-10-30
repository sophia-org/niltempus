"""Strict private Sophia delivery receipt and held-candidate checks.

The capture frame counter and shell presentation epoch are different domains.
The hold is joined through the same owner candidate before and after capture,
not by equating those counters. Native controller/channel custody remains a
separate qualification gate.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import stat

CONNECTOR = 'pci-0000:03:00.0/DP-1'
PRESENTED = {'schema', 'status', 'output', 'candidate_generation',
             'presentation_epoch', 'staging_bytes', 'resident_bytes',
             'retiring_bytes', 'backing_bytes', 'connection_epoch', 'content_grant_epoch'}
PREPARED = PRESENTED - {'presentation_epoch', 'connection_epoch', 'content_grant_epoch'}
RECEIPT = {'schema', 'status', 'binding', 'launch_generation', 'transaction', 'connector',
           'namespace', 'peer_pid', 'peer_starttime', 'exe_dev', 'exe_ino', 'request',
           'output', 'output_generation', 'presentation_generation', 'width', 'height',
           'pam_sha256'}
HOLD = {'generation', 'pattern', 'connection_epoch', 'content_grant_epoch', 'presentation_epoch'}


def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate JSON key')
        result[key] = value
    return result


def record(data):
    return json.loads(data, object_pairs_hook=unique,
                      parse_constant=lambda _: (_ for _ in ()).throw(ValueError('nonfinite JSON')))


def uint(value, positive=False):
    if type(value) is not int or not int(positive) <= value <= 2**64 - 1:
        raise ValueError('expected bounded integer')
    return value


def digest(value):
    if type(value) is not str or re.fullmatch('[0-9a-f]{64}', value) is None:
        raise ValueError('expected canonical SHA256')
    return value


def absolute(value):
    if (type(value) is not str or not value.startswith('/') or str(Path(value)) != value
            or '..' in Path(value).parts or any(ord(c) < 32 or ord(c) == 127 for c in value)):
        raise ValueError('expected canonical absolute path')
    return value


def read_at(directory, name, limit):
    fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC,
                 dir_fd=directory)
    with os.fdopen(fd, 'rb') as file:
        before = os.fstat(file.fileno())
        if (not stat.S_ISREG(before.st_mode) or stat.S_IMODE(before.st_mode) != 0o600
                or before.st_uid != os.getuid() or before.st_nlink != 1 or before.st_size > limit):
            raise ValueError('unsafe or oversized private capture file')
        data = file.read(limit + 1)
        after = os.fstat(file.fileno())
        if (len(data) > limit or len(data) != before.st_size
                or any(getattr(before, key) != getattr(after, key) for key in
                       ('st_dev', 'st_ino', 'st_size', 'st_mtime_ns', 'st_ctime_ns'))):
            raise ValueError('capture file changed during read')
        return data


def inventory(directory):
    groups = {}
    for name in os.listdir(directory):
        match = re.fullmatch(r'capture-([1-9][0-9]*)\.(pam|json|stderr|refused|pam\.partial|json\.partial)', name)
        if not match:
            raise ValueError('unexpected delivery directory entry')
        generation = uint(int(match[1]), True)
        groups.setdefault(generation, set()).add(match[2])
    return groups


def held(log, expected, output):
    if type(expected) is not dict or expected.keys() != HOLD:
        raise ValueError('invalid declared hold')
    for value in expected.values():
        uint(value, True)
    if expected['generation'] not in (1, 2, 3) or expected['pattern'] != (1, 2, 1)[expected['generation'] - 1]:
        raise ValueError('hold must be the declared 1,2,1 pattern sequence')
    latest = None
    seen = set()
    for line in log.decode('utf-8').splitlines():
        if 'sophia_live_shell_content' not in line:
            continue
        if not line.startswith('sophia_live_shell_content '):
            raise ValueError('embedded content record')
        pairs = [part.split('=', 1) for part in line.split(' ')[1:]]
        if any(len(pair) != 2 or not all(pair) for pair in pairs):
            raise ValueError('malformed content record')
        row = unique(pairs)
        if row.get('status') not in ('prepared', 'presented'):
            if row.get('output', str(output)) == str(output):
                latest = None
            continue
        if row.keys() != (PRESENTED if row['status'] == 'presented' else PREPARED):
            raise ValueError('unknown content phase fields')
        for key, value in row.items():
            if key == 'status':
                continue
            if re.fullmatch(r'0|[1-9][0-9]*', value) is None:
                raise ValueError('noncanonical content integer')
            uint(int(value))
        if row['schema'] != '1':
            raise ValueError('unsupported content phase schema')
        if int(row['output']) == output:
            key = (row['status'], row['candidate_generation'])
            if key in seen:
                raise ValueError('duplicate owner phase')
            seen.add(key)
            latest = row
    if (latest is None or latest['status'] != 'presented'
            or any(int(latest[key]) != expected[other] for key, other in (
                ('candidate_generation', 'generation'), ('connection_epoch', 'connection_epoch'),
                ('content_grant_epoch', 'content_grant_epoch'), ('presentation_epoch', 'presentation_epoch')))):
        raise ValueError('owner does not show the declared held candidate')
    return latest


def verify(directory, generation, expected, executable, pixel_check):
    names = inventory(directory).get(generation)
    if names != {'pam', 'json', 'stderr'}:
        raise ValueError('delivery is incomplete or refused')
    prefix = f'capture-{generation}'
    data = read_at(directory, prefix + '.json', 4096)
    receipt = record(data)
    if type(receipt) is not dict or receipt.keys() != RECEIPT:
        raise ValueError('unexpected capture receipt fields')
    for key in RECEIPT - {'status', 'binding', 'connector', 'pam_sha256'}:
        uint(receipt[key], key != 'exe_dev')
    if (receipt['schema'] != 1 or receipt['status'] != 'delivered'
            or receipt['binding'] != 'launch_custody_not_per_message'
            or receipt['connector'] != CONNECTOR or receipt['launch_generation'] != generation
            or receipt['transaction'] != 1
            or receipt['output'] != expected['output']
            or receipt['output_generation'] != expected['output_generation']
            or (receipt['exe_dev'], receipt['exe_ino']) != executable
            or (receipt['width'], receipt['height']) != (1920, 1080)):
        raise ValueError('capture receipt identity mismatch')
    if read_at(directory, prefix + '.stderr', 0) != b'':
        raise ValueError('capture stderr must exist and be empty')
    pam = read_at(directory, prefix + '.pam', 1920 * 1080 * 4 + 256)
    if hashlib.sha256(pam).hexdigest() != digest(receipt['pam_sha256']):
        raise ValueError('PAM hash mismatch')
    pixels = pixel_check(pam, expected['hold']['pattern'])
    # Recheck terminal state after the larger PAM read and pixel oracle.
    if inventory(directory).get(generation) != names or read_at(directory, prefix + '.json', 4096) != data:
        raise ValueError('terminal receipt changed')
    return {'receipt': receipt, 'pixels': pixels, 'receipt_sha256': hashlib.sha256(data).hexdigest()}


def output_mapping(log):
    tag = 'sophia_live_native_head'
    lines = [line for line in log.decode('utf-8').splitlines() if tag in line]
    if len(lines) != 1 or not lines[0].startswith(tag + ' '):
        raise ValueError('expected one native output mapping')
    pairs = [word.split('=', 1) for word in lines[0].split(' ')[1:]]
    if any(len(pair) != 2 or not all(pair) for pair in pairs):
        raise ValueError('malformed output mapping')
    row = unique(pairs)
    if (row.keys() != {'schema', 'status', 'output', 'head', 'connector', 'connector_id',
                       'mode', 'refresh_millihz', 'mirrored'}
            or row['schema'] != '2' or row['status'] != 'ready' or row['connector'] != 'DP-1'
            or row['mode'] != '1920x1080' or row['refresh_millihz'] != '120000'
            or row['mirrored'] != 'false'):
        raise ValueError('output mapping differs from pinned DP-1 workload')
    for key in ('output', 'head', 'connector_id'):
        if re.fullmatch(r'[1-9][0-9]*', row[key]) is None:
            raise ValueError('noncanonical output identity')
        uint(int(row[key]), True)
    return row


def advance(previous, current):
    if previous is not None and (
            any(current[key] != previous[key] for key in
                ('namespace', 'output', 'output_generation', 'connector', 'exe_dev', 'exe_ino'))
            or any(current[key] <= previous[key] for key in
                   ('launch_generation', 'request', 'presentation_generation'))):
        raise ValueError('capture identity changed or sequence did not advance')
