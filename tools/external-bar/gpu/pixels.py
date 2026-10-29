"""Exact non-grey top-strip check for an untimed consumer RGBA capture.

Input is a tightly packed 1920x1080 RGB_ALPHA PAM. This validates pixels only;
the caller must independently bind capture provenance and the held generation.
"""
import argparse
import hashlib
import json
from pathlib import Path

WIDTH, HEIGHT, BAR_HEIGHT = 1920, 1080, 24
HEADER = (b"P7\nWIDTH 1920\nHEIGHT 1080\nDEPTH 4\nMAXVAL 255\n"
          b"TUPLTYPE RGB_ALPHA\nENDHDR\n")
PATTERNS = {
    1: ((255, 0, 0), (0, 0, 255)),
    2: ((0, 255, 0), (255, 255, 255)),
}


def check(data, pattern):
    if type(pattern) is not int or pattern not in PATTERNS:
        raise ValueError("declared pattern must be 1 or 2")
    if not data.startswith(HEADER) or len(data) != len(HEADER) + WIDTH * HEIGHT * 4:
        raise ValueError("expected exact full-output 1920x1080 RGBA8 PAM")
    pixels = memoryview(data)[len(HEADER):]
    left, right = PATTERNS[pattern]
    checked = 0
    for y in range(BAR_HEIGHT):
        for x in range(WIDTH):
            # Exclude the same two split-edge columns as the existing EGL oracle.
            if x in (WIDTH // 2 - 1, WIDTH // 2):
                continue
            expected = (*left, 255) if x < WIDTH // 2 else (*right, 255)
            offset = (y * WIDTH + x) * 4
            if tuple(pixels[offset:offset + 4]) != expected:
                raise ValueError(f"pattern {pattern} mismatch at x={x} y={y}")
            checked += 1
    return {"schema": 1, "status": "pixels_match", "pattern": pattern,
            "size": [WIDTH, HEIGHT], "region": [0, 0, WIDTH, BAR_HEIGHT],
            "excluded_columns": [WIDTH // 2 - 1, WIDTH // 2], "checked_pixels": checked,
            "capture_sha256": hashlib.sha256(data).hexdigest(),
            "bar_rgba_sha256": hashlib.sha256(pixels[:WIDTH * BAR_HEIGHT * 4]).hexdigest(),
            "limits": "Pixels only. No capture provenance, held-generation, device, "
                      "import/synchronization/release, presentation or performance proof. "
                      "Capture alpha is checked; DMA-BUF XR24 padding is not alpha."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    parser.add_argument("--pattern", type=int, choices=PATTERNS, required=True)
    args = parser.parse_args()
    try:
        # A malformed input cannot force an unbounded allocation.
        with args.capture.open("rb") as stream:
            data = stream.read(len(HEADER) + WIDTH * HEIGHT * 4 + 1)
        result = check(data, args.pattern)
    except (OSError, ValueError) as error:
        parser.exit(1, f"GPU pixel refusal: {error}\n")
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
