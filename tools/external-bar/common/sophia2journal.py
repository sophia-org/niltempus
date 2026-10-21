#!/usr/bin/env python3
"""Convert one Sophia shell_timing_peer run into an external-bar journal (rev 4).

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
counted frame k therefore carries generation k+U (shade (k+U)%128): same byte
count and write pattern as the reference clients, different shade value.

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
    args = ap.parse_args()
    if args.warmup < 0 or args.buffers != 2:
        sys.exit("warmup must be nonnegative; the current peer has two resource ids")
    stage = Path(args.stage)

    events = collections.defaultdict(list)
    for line in (stage / "client.ndjson").read_text().splitlines():
        e = json.loads(line)
        events[e["event"]].append(e)
    start = events["start"][0]
    if start.get("wire") != "files" or start.get("source") != "sealed":
        sys.exit(f"not a sealed files run: {start}")
    bulk = 1 if start.get("probe_bytes", 0) else 0

    uploads = {}
    for e in events["upload_offer"]:
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
    samples = {s["candidate_generation"]: s for s in data["samples"]}
    if len(samples) != len(data["samples"]):
        sys.exit("duplicate native sample generation")
    for generation, sample in samples.items():
        offer = offers.get(generation)
        if offer is None or sample.get("heads", 0) <= 0 or any(sample.get(key) != offer.get(key) for key in ("connection_epoch", "content_grant_epoch", "output", "submitted_monotonic_usec")):
            sys.exit(f"native sample {generation} does not match its exact offer")

    gens = sorted(offers)
    if gens != list(range(1, len(gens) + 1)):
        sys.exit("offered generations not contiguous from 1")
    for g in gens:
        for name, table in (("upload", uploads), ("permit", permits), ("terminal", terminal)):
            if g not in table:
                sys.exit(f"generation {g} has no {name}")
    if args.warmup >= len(gens):
        sys.exit("warmup consumes every frame")
    counted = gens[args.warmup:]

    with open(args.out, "w") as j:
        j.write(
            "# header arm=sophia width=1920 height=24 buffers=%d fill=shade_mod128_a255 "
            "frames=%d warmup=%d bulk=%d bulk_width=%d bulk_height=%d lifecycle=fresh "
            "clock=CLOCK_MONOTONIC_ns presented_clock=kernel_flip_usec_all_heads_xtask_join\n"
            % (args.buffers, len(counted), args.warmup, bulk, 1024 if bulk else 0, 1024 if bulk else 0)
        )
        j.write(f"# source {stage.resolve()}\n# mode sophia_files_sealed\n")
        for k, g in enumerate(counted, 1):
            presented = -1
            if terminal[g]["outcome"] == "presented" and g in samples:
                presented = samples[g]["completed_monotonic_usec"] * 1000
            j.write("%d\t%d\t%d\t%d\t%d\t%s\n" % (
                k,
                uploads[g]["begin_usec"] * 1000,
                offers[g]["submitted_monotonic_usec"] * 1000,
                presented,
                permits[g]["received_usec"] * 1000,
                "sophia_kernel_flip",
            ))
        diag = data.get("costs", {}).get("diagnostics", {})
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
