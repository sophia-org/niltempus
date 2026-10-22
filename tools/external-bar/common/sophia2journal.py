#!/usr/bin/env python3
"""Convert one validated Sophia native join into a rev-5 diagnostic journal.

  sophia2journal.py STAGE_DIR --out sophia.tsv [--warmup U] [--buffers 2]

STAGE_DIR holds client.ndjson (peer events) and native-samples.json (the
xtask join), as in ~/.local/state/t331/panel23r-*. Per candidate generation:

  fill_start_ns  upload_offer.begin_usec      peer begins sealed preparation
                                              (memfd create .. seal), the same
                                              span as the reference clients'
                                              fresh lifecycle fill_start
  submit_ns      offer.submitted_monotonic_usec   candidate submitted
  presented_ns   sample.completed_monotonic_usec  kernel page-flip time on every
                                              head (xtask refuses non-kernel or
                                              partial-head completions), the
                                              qualified by the upstream join;
                                              this converter does not prove
                                              equivalence to external feedback
  pace_ns        permit.received_usec          the permit that released it

All values are CLOCK_MONOTONIC microseconds in the source, written as ns.
The first U generations are dropped as warmup and the rest renumbered 1..F;
counted frame k therefore carries generation k+U, matching the reference clients.
Owned-ring fill and due times come from the validated ring-copy records. Historical
sealed bursty journals lack due times and retain their original pixel-pattern label.

CPU is NOT taken from native-samples here: run account.py sample on the
Session PID set before and after, exactly as for River and Xorg, so all arms
use one accounting method. native-samples' own counter bounds are copied into
a '# sophia_native_cost' footer as a cross-check only.
"""
import argparse
import collections
import json
import sys
from pathlib import Path


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("stage")
    ap.add_argument("--out", required=True)
    ap.add_argument("--warmup", type=int, default=0)
    ap.add_argument("--buffers", type=int, default=2,
                    help="in-flight resource bound to declare; must equal the reference arms'")
    ap.add_argument('--idle-sec', type=int, default=0,
                    help='must match the explicit typed runner idle declaration')
    args = ap.parse_args()
    if args.warmup < 0 or args.buffers != 2 or args.idle_sec < 0:
        sys.exit("warmup must be nonnegative; the current peer has two resource ids")
    stage = Path(args.stage)

    events = collections.defaultdict(list)
    for line in (stage / "client.ndjson").read_text().splitlines():
        e = json.loads(line)
        events[e["event"]].append(e)
    if len(events['start']) != 1 or len(events['end']) != 1:
        sys.exit('requires exactly one start and end')
    start = events["start"][0]
    if start.get("wire") != "files" or start.get("source") not in {"sealed", "owned_ring"}:
        sys.exit(f"not a sealed files run: {start}")
    ring = start['source'] == 'owned_ring'
    upload_events = events['ring_offer' if ring else 'upload_offer']
    bulk = 1 if start.get("probe_bytes", 0) else 0

    uploads = {}
    for e in upload_events:
        u = e["upload"]
        if (u["width"], u["height"]) == (1920, 24):
            if u["candidate_generation"] in uploads:
                sys.exit(f"two panel uploads for generation {u['candidate_generation']}")
            uploads[u["candidate_generation"]] = u
    offers = {e["candidate_generation"]: e for e in events["offer"]}
    permits = {e["generation"]: e for e in events["permit"]}
    terminal = {e["candidate_generation"]: e for e in events["terminal"]}
    data = json.loads((stage / "native-samples.json").read_text())
    if data.get("schema") != 2 or data.get("verdict") != "joined_only":
        sys.exit("requires a schema-2 native xtask join; this converter does not establish kernel/head evidence")
    metadata = data.get('journal') or {}
    if (any(metadata.get(key) != start.get(key) for key in ('schema', 'wire', 'source', 'probe_bytes', 'interval_msec', 'retained_resources'))
            or metadata.get('workload', 'fresh') != start.get('workload', 'fresh')):
        sys.exit('raw start differs from validated native journal')
    if [e['upload'] for e in upload_events] != [u['upload'] for u in metadata.get('uploads', [])]:
        sys.exit('raw uploads differ from the validated native journal')
    copies = {}
    if ring:
        evidence = metadata.get('ring') or {}
        registration = evidence.get('registration') or {}
        if (start['schema'] != 5 or start.get('ring_slots') != registration.get('slots')
                or len(events['ring_ready']) != 1
                or {k: v for k, v in events['ring_ready'][0].items() if k not in {'schema', 'event'}} != registration
                or events['upload_offer']):
            sys.exit('raw ring registration differs from validated native journal')
        if not (len(upload_events) == len(events['ring_accepted']) == len(events['slot_released'])
                == len(evidence.get('copies', []))):
            sys.exit('ring copy evidence is incomplete')
        for offer, accepted, released, validated, upload in zip(
                upload_events, events['ring_accepted'], events['slot_released'],
                evidence['copies'], metadata['uploads']):
            identity = dict(offer=offer['offer'], resource_id=offer['upload']['resource_id'],
                            resource_generation=offer['upload']['resource_generation'])
            slot = dict(slot=offer['slot'], slot_generation=offer['slot_generation'])
            expected = dict(identity, **slot, fill_start_usec=offer['fill_start_usec'],
                            released_usec=released['released_usec'])
            if offer.get('due_usec') is not None:
                expected['due_usec'] = offer['due_usec']
            if (validated != expected
                    or any(accepted.get(k) != v for k, v in identity.items())
                    or any(released.get(k) != v for k, v in (identity | slot).items())
                    or accepted['accepted_usec'] != upload['accepted_usec']
                    or identity['offer'] in copies):
                sys.exit('raw ring copy differs from validated native journal')
            copies[identity['offer']] = validated
    if [{k: e[k] for k in ('candidate_generation', 'placements')}
            for e in events['candidate_placements']] != metadata.get('placements', []):
        sys.exit('raw placements differ from validated native journal')
    if [{k: e[k] for k in ('generation','demand_usec','received_usec')} for e in events['permit']] != metadata.get('permits'):
        sys.exit('raw permits differ from the validated native journal')
    if [e['observed_monotonic_usec'] for e in events['terminal']] != metadata.get('terminal_usec'):
        sys.exit('raw terminals differ from the validated native journal')
    timing = data.get('external_timing') or {}
    interval = metadata.get('interval_msec') or 0
    tiles = metadata.get('retained_resources') or 0
    workload = 'retained' if tiles else 'bursty' if interval else 'idle' if timing.get('workload') == 'idle' else 'fresh'
    idle = timing.get('seconds', 0) if workload == 'idle' else 0
    if args.idle_sec and args.idle_sec != idle:
        sys.exit('idle requires the matching typed runner declaration')
    if workload == 'idle' and (start['frames'] != 1 or args.warmup or idle != 60):
        sys.exit('idle requires one frame, no warmup and the approved 60-second window')
    if workload == 'bursty' and (timing.get('workload') != 'bursty' or timing.get('interval_msec') != interval or timing.get('warmup') != args.warmup):
        sys.exit('bursty requires the matching typed runner declaration')
    if workload == 'retained':
        pool = [u for u in metadata.get('uploads', []) if u['purpose'] == 'retained']
        if tiles != 8 or len(pool) != 8 or any((u['upload']['width'], u['upload']['height']) != (240, 24) for u in pool):
            sys.exit('retained requires eight validated 240x24 tiles')
    for name, table in [('offer', offers), ('permit', permits), ('terminal', terminal)]:
        if len(table) != len(events[name]):
            sys.exit(f'duplicate {name} generation')
    samples = {s["candidate_generation"]: s for s in data["samples"]}
    if len(samples) != len(data["samples"]):
        sys.exit("duplicate native sample generation")
    for generation, sample in samples.items():
        offer = offers.get(generation)
        if offer is None or sample.get("heads", 0) <= 0 or any(sample.get(key) != offer.get(key) for key in ("connection_epoch", "content_grant_epoch", "output", "submitted_monotonic_usec")):
            sys.exit(f"native sample {generation} does not match its exact offer")

    gens = sorted(offers)
    if gens != list(range(1, len(gens) + 1)) or len(gens) != start['frames'] or set(samples) != set(gens):
        sys.exit("offered generations not contiguous from 1")
    if any(events['end'][0].get(k) != len(gens) for k in ('offered', 'completed')):
        sys.exit('raw end count differs from validated native journal')
    for g in gens:
        required = (("permit", permits), ("terminal", terminal))
        if workload != 'retained':
            required += (("upload", uploads),)
        for name, table in required:
            if g not in table:
                sys.exit(f"generation {g} has no {name}")
    if args.warmup >= len(gens):
        sys.exit("warmup consumes every frame")
    counted = gens[args.warmup:]

    with open(args.out, "x") as j:
        j.write(
            "# header arm=sophia width=1920 height=24 buffers=%d fill=%s "
            "frames=%d warmup=%d bulk=%d bulk_width=%d bulk_height=%d lifecycle=%s "
            "workload=%s tiles=%d interval_ms=%d damage=%s idle_sec=%d "
            "clock=CLOCK_MONOTONIC_ns presented_clock=kernel_flip_usec_all_heads_xtask_join\n"
            % (args.buffers, 'bursty_static_background_v1' if ring and workload == 'bursty' else 'shade_mod128_a255',
               len(counted), args.warmup, bulk, 1024 if bulk else 0, 1024 if bulk else 0, 'reuse' if ring else 'fresh',
               workload, tiles, interval, '96x24+1824+0' if workload == 'bursty' else 'full', idle)
        )
        j.write(f"# source {stage.resolve()}\n# mode sophia_files_{start['source']}\n")
        if ring:
            j.write('# ownership reusable producer slots; fresh immutable accepted copies; slot release is not resource release\n')
        if workload == 'bursty' and not ring:
            j.write('# note unqualified: received producer changes the whole frame; scheduled due times unavailable\n')
        for k, g in enumerate(counted, 1):
            fill_usec = (permits[g]['received_usec'] if workload == 'retained' else
                         copies[g]['fill_start_usec'] if ring else uploads[g]['begin_usec'])
            pace_ns = ((copies[g]['due_usec'] * 1000 if ring else -1) if workload == 'bursty'
                       else permits[g]['received_usec'] * 1000)
            presented = -1
            if terminal[g]["outcome"] == "presented" and g in samples:
                presented = samples[g]["completed_monotonic_usec"] * 1000
            j.write("%d\t%d\t%d\t%d\t%d\t%s\n" % (
                k,
                fill_usec * 1000,
                offers[g]["submitted_monotonic_usec"] * 1000,
                presented,
                pace_ns,
                "sophia_kernel_flip",
            ))
        diag = (data.get("costs") or {}).get("diagnostics", {})
        for role in ("session", "peer", "combined"):
            if role in diag:
                d = diag[role]
                j.write(f"# sophia_native_cost role={role} cpu_ns_lower={d['cpu_ns']['lower']} "
                        f"cpu_ns_upper={d['cpu_ns']['upper']} minflt_lower={d['minor_faults']['lower']} "
                        f"minflt_upper={d['minor_faults']['upper']}\n")
        # Preserve native counter bounds as bounds above. They are not a
        # measured getrusage value; missing client CPU/faults stay unavailable.
    print(f"wrote {len(counted)} counted frames (warmup {args.warmup}) -> {args.out}")


if __name__ == "__main__":
    main()
