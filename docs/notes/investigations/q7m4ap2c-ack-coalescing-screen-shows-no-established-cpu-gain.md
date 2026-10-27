---
id: q7m4ap2c
date: 2026-10-26
kind: investigation
status: investigating
tags: [investigation, performance]
---
# ACK coalescing screen shows no established CPU gain

## Question

Does queuing a consumed event batch's ACK with the reissued event read reduce
continuous owned-ring presentation CPU without changing admission, custody or
completion semantics?

**Result:** the complete A1 ABBA screen has a favorable combined midpoint of
1,315 to 1,295 microseconds per frame (-1.52%), but its accounting bounds include
zero improvement. A1 remains unpromoted. XLibre/Wayland CPU parity is unmeasured.

## Evidence

Local evidence: `~/.local/state/t331/continuous-ack-coalesce-02/`. Raw captures
and binaries remain outside this repository. Fixed order: `r120b1`, `r120c1`,
`r120c2`, `r120b2` (baseline, candidate, candidate, baseline).

| Input | Source commit | Binary SHA256 prefix |
| --- | --- | --- |
| Fixed Sophia | `d9116ff0905fab5c8bc01a71228ecc9e1a7a0a6d` | `3897c98c400914b1` |
| Baseline peer recipe | `748c325bf993154d716d7caead2af4c5ec4b534a` | `e730fef6897d5ac9` |
| Candidate peer recipe | `ed491c89cd9f0103276ebd3ef9cbea41d61e692d` | `87bd35f0d00c484b` |
| Common runner | `2329fca7e39e941425920dfd7dbabf2aa22acc39` | `62df9c3095e1c3e3` |

The baseline SDK is `0706ea2c78cd16ba59ed209620354f30a2c78906`; A1 is
`8a685d0ded6ff6cec2e59bbed32aa401d34660d4`. The candidate recipe changes SDK
dependency locations only: nine SDK packages resolve to the candidate tree,
with 162 non-SDK lock records identical. Both peers retain Lom `dcb1eb88`.
No release overlap, logging change, readback reuse/direct fill, ThinLTO or host
optimization is bundled with A1. Full artifact identities are in `pins.json`.

The workload changes a 1920x24 strip on DP-1, 1920x1080 at nominal 120 Hz
(observed 119.88 Hz), VRR off. Each arm presents 2,120 owned-ring frames using
two slots. CPU accounting uses kernel completions of generations 120 and 2120:
2,000 intervals spanning about 16.68345 seconds in a roughly 60-second session.
Capture-profile and render timing remain enabled; perf, syscall tracing and
shell work-stat logging are absent.

CPUwin samples cumulative process counters every 5 ms. Inner/outer anchor
brackets include the existing two-tick quantization allowance per process.
The root audit independently reconstructed these bounds from raw CPU samples
and native kernel-completion records.

### CPU results

Microseconds per counted frame; variant midpoints average both arms.
Difference bounds are candidate lower minus baseline upper, and candidate upper
minus baseline lower. They are accounting bounds, not confidence intervals.

| Scope | Baseline midpoint | Candidate midpoint | Difference | Difference bounds |
| --- | ---: | ---: | ---: | ---: |
| Session | 838.75 | 830.00 | -8.75 (-1.04%) | [-30.0, +12.5] |
| Peer | 476.25 | 465.00 | -11.25 (-2.36%) | [-32.5, +10.0] |
| Combined | 1,315.00 | 1,295.00 | -20.00 (-1.52%) | [-62.5, +22.5] |

Combined per-arm bounds: b1 [1285, 1330], c1 [1275, 1315], c2 [1275, 1315],
b2 [1300, 1345]. Both adjacent midpoint comparisons favor A1 (-12.5 and -27.5),
but every scope's difference bounds include zero.

### Mechanism and timing

First-to-third periodic counter samples contain 1,200 `native_ready` increments
per arm. Latest completed native-frame boundaries are b1 497..1696,
c1 498..1698, c2 500..1700 and b2 501..1701. These sampled intervals differ
from the exact CPU window; they are not per-frame syscall observations.

| Counter per native-ready event | Baseline mean | Candidate mean | Change |
| --- | ---: | ---: | ---: |
| Owner passes | 12.062917 | 12.025000 | -0.314% |
| FD-ready returns | 10.063333 | 10.025000 | -0.381% |
| Deadline expirations | approximately 1 | 1 | Essentially unchanged |

This is about 46 fewer FD-ready returns per 1,200 native-ready events, not one
fewer wake per frame. A stricter selection within native frames 500..1699 agrees
in direction. No syscall count or CPU attribution follows. The roughly ten
FD-ready returns and one deadline per frame remain; post-count idle intervals
are excluded.

Every arm's counted kernel-completion gaps have median 8,342 and maximum
8,347 microseconds, with no skipped refresh. Submission-to-completion medians
are 7,512 / 7,514 / 7,517 / 7,511 microseconds in ABBA order; prepare-start to
completion medians are 8,394 / 8,394 / 8,394 / 8,393. No material cadence or
latency regression was observed. Submission-to-completion is not input latency;
elapsed stage timings include waiting and are not CPU savings.

### Custody and the failed first attempt

All four replacement arms passed: 2,120 exact presentation joins each, correct
identities, zero late sampler slots, quiescent cleanup and restored tty4.
VT sessions c104..c107 had child status zero and no residual processes.
The manifest's 78 entries and all 29 external pins verified after capture.
The known VT receipt mode of 0664 is disclosed; audit outputs use 0700/0600.

`continuous-ack-coalesce-01` remains failed. Its last baseline arm completed all
presentations and shutdown, but the observer rejected the bwrap supervisor's
executable evidence roughly 42 seconds after the counted window. The old error
collapsed missing, deleted and mismatched observations, so its exact cause
cannot be recovered from that message.

The replacement uses the same corrected runner in all four arms. A deterministic
FIFO fixture reproduces an exit between live-state and executable checks. Only
raw ENOENT followed by an absent process or the same identity in an ended state
can now yield `Ended`. PID reuse, deleted executables, mismatches, permission
failures and live missing executables remain errors. The fix passed 54 focused
tests, formatting and Clippy. It does not eliminate pre-zombie, command-line or
namespace teardown windows.

All 190 saved files from the failed package were independently verified
unchanged. No failed rows were replaced or pooled into this comparison.

## Finding and resolution

The isolated SDK fixture proves an eligible batch can send Tread and ACK together
(one 62-byte send instead of separate 23- and 39-byte sends). The native screen
shows only a small change in sampled owner counters and does not establish a
CPU reduction under the declared bounds criterion. Do not promote A1 on this
evidence or credit it with removing an owner pass per frame.

## Validation and remaining work

Keep all four arms and the failed predecessor. Further qualification requires a
separately declared capture; do not select runs by CPU result. No visual oracle,
input-latency test, native syscall trace or matched external server comparison
was performed. Kernel-worker CPU is outside Session/peer accounting.

Retained documents relative to the evidence root (SHA256):

| Path | SHA256 |
| --- | --- |
| `MANIFEST.sha256` | `3e8b138b31dc93ae142abff6c7cd6e8aa2e993ac43001add47440180589c0c19` |
| `analysis/summary.json` | `1bd056b21ba6a963acf7b3169c1a1a70b1fd4806326180c66b47df5d3ea22021` |
| `analysis/root/cpu-audit.json` | `35e8fa85b3ce86fcc71cfa38636c03d84819da18ee6453a281dbd5ecbffb569a` |
| `analysis/root/decision.json` | `6b2f964c21d07062fafe0dea515728f708d252827a70f4661abaf14c84f15799` |
| `analysis/transport/audit.json` | `6a3541b8bf597e4f6cb06c62a6fbae5b211659bfeb29e8f2040ec2674faa0e38` |
| `analysis/producer/mechanism.json` | `09796028ef259b5ffb3c6b763eeb5dd8bdc01a66fceb9096a461858e87cb554a` |

## Connections

- [Physical runners](../../physical-runners.md): identity checks, failed-attempt
  preservation and the distinction between captured evidence and a pass.
- [Desktop comparison](../../desktop-comparison.md): external pixel-path
  comparisons require matched workload and completion semantics.
