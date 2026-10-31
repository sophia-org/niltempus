"""Untimed opaque Lom strip check; the portal PAM is already top-down RGBA8."""

import hashlib
import re

WIDTH, HEIGHT, BAR_HEIGHT = 1920, 1080, 24
HEADER = (b'P7\nWIDTH 1920\nHEIGHT 1080\nDEPTH 4\nMAXVAL 255\n'
          b'TUPLTYPE RGB_ALPHA\nENDHDR\n')
LIMIT = len(HEADER) + WIDTH * HEIGHT * 4


def fnv64(data):
    value = 0xcbf29ce484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001b3) & 0xffffffffffffffff
    return f'{value:016x}'


def strip(data, height=BAR_HEIGHT):
    if type(height) is not int or height not in (24, 512):
        raise ValueError('expected declared strip height 24 or 512')
    if len(data) != LIMIT or not data.startswith(HEADER):
        raise ValueError('expected exact full-output 1920x1080 RGBA8 PAM')
    rgba = memoryview(data)[len(HEADER):len(HEADER) + WIDTH * height * 4]
    if any(alpha != 255 for alpha in rgba[3::4]):
        raise ValueError('Lom strip must be fully opaque')
    if not any(red != blue for red, blue in zip(rgba[0::4], rgba[2::4])):
        raise ValueError('strip cannot detect a red/blue swap')
    bgra = bytearray(rgba)
    bgra[0::4], bgra[2::4] = rgba[2::4], rgba[0::4]
    return bgra


def capture_strip(data, height=BAR_HEIGHT):
    bgra = strip(data, height)
    return dict(schema=1, status='opaque_strip_captured', size=[WIDTH, HEIGHT],
                region=[0, 0, WIDTH, height], checked_pixels=WIDTH * height,
                produced_fnv64=None, captured_bgra_fnv64=fnv64(bgra),
                capture_sha256=hashlib.sha256(data).hexdigest(),
                bar_bgra_sha256=hashlib.sha256(bgra).hexdigest(),
                limits='Captured opaque strip only; no producer checksum or pixel equality '
                       'claim. Capture hashes bind saved pixels only. Separate comparison '
                       'must bind the reference scene, inputs and capture provenance.')


def check(data, checksum, height=BAR_HEIGHT):
    if type(checksum) is not str or re.fullmatch('[0-9a-f]{16}', checksum) is None:
        raise ValueError('expected canonical Lom FNV64')
    result = capture_strip(data, height)
    if result['captured_bgra_fnv64'] != checksum:
        raise ValueError('captured strip differs from Lom produced-byte FNV64')
    result.update(status='opaque_strip_checksum_match', produced_fnv64=checksum,
                limits='FNV64 consistency for one opaque scale-1 scene, not a cryptographic '
                       'producer proof. Capture SHA256 binds saved pixels only. No capture '
                       'provenance, changing-content, alpha-blending or performance proof.')
    return result
