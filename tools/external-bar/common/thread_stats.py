"""Endpoint thread counters. These cannot observe threads born and lost between samples."""
from pathlib import Path


def read_threads(pid):
    root = Path(f'/proc/{pid}/task')
    tids = sorted(p.name for p in root.iterdir())
    result = {}
    for tid in tids:
        stat = (root/tid/'stat').read_text()
        identity = int(stat[stat.rindex(')') + 2:].split()[19])
        status = dict(line.split(':', 1) for line in (root/tid/'status').read_text().splitlines())
        result[tid] = dict(starttime=identity,
                          voluntary=int(status['voluntary_ctxt_switches']),
                          involuntary=int(status['nonvoluntary_ctxt_switches']))
        again = (root/tid/'stat').read_text()
        if int(again[again.rindex(')') + 2:].split()[19]) != identity:
            raise RuntimeError('thread identity changed during snapshot')
    if not tids or sorted(p.name for p in root.iterdir()) != tids:
        raise RuntimeError('thread set changed during snapshot; no complete sample')
    return result


def endpoint_delta(before, after):
    if not before or not after or before.keys() != after.keys():
        return None
    total = dict(voluntary=0, involuntary=0)
    for tid in before:
        if before[tid]['starttime'] != after[tid]['starttime']:
            return None
        for key in total:
            value = after[tid][key] - before[tid][key]
            if value < 0:
                raise ValueError('thread counter decreased')
            total[key] += value
    # A matching endpoint set still cannot prove no short-lived threads existed.
    return total
