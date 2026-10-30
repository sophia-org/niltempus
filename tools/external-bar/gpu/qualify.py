"""Validate supporting GPU journal receipts, not native/device qualification.

The caller supplies the declared transport and workload. Artifact/process,
output, pixel, synchronization and cleanup proofs remain separate gates.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re


TRANSPORTS = {
    "wayland": (
        "linux_dmabuf_v4", "wl-panel", "wp_presentation",
        "wp_presentation CLOCK_MONOTONIC (id=1); DMA-BUF implicit sync",
        "schema=1 protocol=linux_dmabuf version=4 main_pci=0000:03:00.0 "
        "tranche_pci=0000:03:00.0 format=XR24 modifier=0 sync=implicit",
    ),
    "x11": (
        "dri3_1_2", "x-panel", "xpresent",
        "xpresent_ust_usec_assumed_CLOCK_MONOTONIC_checked_per_frame",
        "schema=1 protocol=dri3 version=1.2 server_pci=0000:03:00.0 "
        "format=XR24 modifier=0 present=plain release=IdleNotify sync=implicit",
    ),
}


def fields(text):
    pairs = [word.split("=", 1) for word in text.split(" ")]
    if any(len(pair) != 2 or not all(pair) for pair in pairs):
        raise ValueError("malformed receipt fields")
    if len({pair[0] for pair in pairs}) != len(pairs):
        raise ValueError("duplicate receipt field")
    return dict(pairs)


def uint(text):
    if not re.fullmatch(r"0|[1-9][0-9]*", text) or int(text) > 2**64 - 1:
        raise ValueError("noncanonical or overflowing integer")
    return int(text)


def qualify(text, transport, frames, warmup):
    return _qualify(text, transport, frames, warmup, 'vello_shade_mod128_opaque_rgb')


def qualify_colour(text, transport):
    return _qualify(text, transport, 3, 0, 'diagnostic_colour_hold_121')


def _qualify(text, transport, frames, warmup, fill):
    if (transport not in TRANSPORTS or type(frames) is not int or type(warmup) is not int
            or not 1 <= frames <= 1_000_000 or not 0 <= warmup <= 1_000_000):
        raise ValueError("invalid declared GPU workload")
    if (not text.isascii() or not text.endswith("\n") or
            any((ord(c) < 32 and c not in "\n\t") or ord(c) == 127 for c in text)):
        raise ValueError("truncated journal or control character")
    wire, arm, source, clock, imported = TRANSPORTS[transport]
    expected = {
        "gpu_producer": fields(
            f"schema=1 transport={wire} producer=vello_wgpu pci=0000:03:00.0 "
            "node=/dev/dri/renderD128 format=XR24 modifier=0 planes=1 buffers=2 lifecycle=fresh"),
        "gpu_import": fields(imported),
        "gpu_end": {"schema": "1", "rendered": str(frames + warmup),
                    "released": str(frames + warmup)},
    }
    header = fields(
        f"arm={arm} width=1920 height=24 buffers=2 fill={fill} "
        f"frames={frames} warmup={warmup} bulk=0 bulk_width=0 bulk_height=0 "
        "lifecycle=fresh workload=fresh tiles=0 interval_ms=0 damage=full idle_sec=0 "
        "clock=CLOCK_MONOTONIC_ns")
    order = (["output", "header", "gpu_producer", "gpu_import", "gpu_end", "rows",
              "rusage", "ctxsw", "pid", "mode"] if transport == "wayland" else
             ["header", "gpu_producer", "gpu_import", "pid", "attach", "rows",
              "gpu_end", "rusage", "ctxsw", "clock_check"])
    seen, observed, rows, previous, pid = set(), [], 0, None, None
    for line in text.splitlines():
        tag = None
        if "gpu_" in line:
            tag, sep, body = line[2:].partition(" ")
            if (not line.startswith("# gpu_") or not sep or tag not in expected
                    or tag in seen or fields(body) != expected[tag]):
                raise ValueError("missing, foreign, duplicate or mismatched GPU receipt")
        elif "# header" in line:
            body, sep, note = line[len("# header "):].partition(" presented_clock=")
            if (not line.startswith("# header ") or "header" in seen or not sep
                    or note != clock or fields(body) != header):
                raise ValueError("mismatched GPU header")
            tag = "header"
        elif line.startswith("# presented_clock_check"):
            if (transport != "x11" or "clock_check" in seen or
                    fields(line[len("# presented_clock_check "):]) !=
                    {"ok": str(frames + warmup), "violations": "0"}):
                raise ValueError("invalid X presentation clock check")
            tag = "clock_check"
        elif line.startswith("# "):
            tag, sep, body = line[2:].partition(" ")
            if not sep or tag in seen:
                raise ValueError("malformed or duplicate journal metadata")
            if tag == "pid":
                pid = uint(body)
                if not pid:
                    raise ValueError("zero client PID")
            elif tag in ("rusage", "ctxsw"):
                metrics = fields(body)
                keys = ({"utime_ns", "stime_ns", "minflt", "majflt"} if tag == "rusage"
                        else {"voluntary", "involuntary"})
                if metrics.keys() != keys:
                    raise ValueError("unexpected accounting metadata")
                for value in metrics.values():
                    uint(value)
            elif tag == "output":
                if fields(body) != {"name": "DP-1", "width": "1920", "height": "1080", "scale": "1"}:
                    raise ValueError("unexpected output")
            elif (tag, body) not in (("attach", "dri3_pixmap_from_buffers"), ("mode", "frame_callback")):
                raise ValueError("foreign journal metadata")
        else:
            row = line.split("\t")
            if len(row) != 6 or row[5] != source:
                raise ValueError("invalid GPU frame row or presentation source")
            frame, fill, submit, presented = map(uint, row[:4])
            if row[4] != "-1":
                uint(row[4])
            if (frame != rows + 1 or not 0 < fill <= submit <= presented or
                    previous is not None and (submit <= previous[0] or presented <= previous[1])):
                raise ValueError("invalid frame sequence or timestamp order")
            previous = submit, presented
            rows += 1
            if not observed or observed[-1] != "rows":
                observed.append("rows")
        if tag is not None:
            seen.add(tag)
            observed.append(tag)
    if observed != order or rows != frames:
        raise ValueError("missing, foreign or reordered GPU journal records")
    return {"schema": 1, "status": "receipts_valid", "transport": transport,
            "producer": "vello_wgpu", "frames": frames, "warmup": warmup, "client_pid": pid,
            "receipts": expected,
            "limits": "Self-reported receipts only; no artifact/process, display, pixel, "
                      "feedback/sampler-anchor cross-check, real import/sync/release, "
                      "cleanup or CPU qualification."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("journal", type=Path)
    parser.add_argument("--transport", choices=TRANSPORTS, required=True)
    parser.add_argument("--frames", type=int, required=True)
    parser.add_argument("--warmup", type=int, required=True)
    args = parser.parse_args()
    try:
        data = args.journal.read_bytes()
        result = qualify(data.decode("utf-8"), args.transport, args.frames, args.warmup)
        result["journal_sha256"] = hashlib.sha256(data).hexdigest()
    except (OSError, ValueError) as error:
        parser.exit(1, f"GPU receipt refusal: {error}\n")
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
