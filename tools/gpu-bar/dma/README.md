# gpu-bar-dma

One peer binary for two matched t339 arms on the same current Sophia host.
Both render the same GPU scene through `../common/gpu_canvas.rs`, over the
files wire at shell revision 10, alternating two resources per frame.

| `source` | Arm | Image path | Journal |
|---|---|---|---|
| `dma_buf` | G | `render_dma` exports the producer-completed frame as a DMA-BUF; Session captures it. | schema 2 |
| `owned_ring` | R | `Canvas::fill` reads the frame back into a `Vec` (`read_rgba`, as arm S does), then `image_upload_ring` copies it into a slot of one persistent SDK owned-ring registration. | schema 7 |

R therefore pays one GPU readback and one CPU copy into the slot per frame,
on top of the shared render; G pays neither. That copy is part of what an
R/G comparison measures.

Configuration is exactly `wire` (`files`), `output`, `frames`, `height`,
`hold_msec` and `source`, plus `ring_slots` (2..=4) only for `owned_ring`.
Optional `dma_reuse` (boolean) is accepted only for `dma_buf`; absent or
`false` is the unchanged fresh G arm. `true` is diagnostic only and requires
`capture_profile: true` (and so its bounds), so no comparison plan can treat
reused backing as fresh G before a separate measured acceptance.

Optional `capture_profile: true` enables cumulative producer DMA-stage timing
for a 24-row, 2120-frame, 60000-ms diagnostic. The runner must also pin
`instrumentation: "capture-profile"`; it enables the host's render counters.
Timing is disabled by default. Profile captures are attribution evidence and
are refused by cpuplan as comparison arms. Timing records go to stderr after
the counted window, leaving the image journal unchanged.

The current producer submits the foreign release immediately after drawing on
the same queue. Its final fence covers both, followed by bounded wgpu cleanup
before export. `draw` now measures encoding/submission; `release` includes the
combined wait, cleanup poll and source drop. Compare `draw + release` against
older profiles that waited between submissions; individual stage times changed
meaning. Fresh mode still uses a fresh allocation every frame.

With `dma_reuse: true` (`src/dma_frame.rs`), frame 1 is a fresh export. Each
later frame redraws that same export through Lom's unsafe
`render_dma_buf_reusing`, and only after the previous offer settled Accepted
with a capture-complete (disposition 2) release and `image_forget`
succeeded. Offers, resource identities and the schema 2 journal are
unchanged; Session sees the same buffer every frame. Uncertain begin or
settle outcomes keep the export allocated until exit, as in fresh mode. The
retained export is destroyed after the counted window. Every `dma_buf` run
prints `gpu_bar_dma_mode schema=1 mode=fresh|reuse` to stderr. The reuse
profile is `gpu_bar_dma_timing schema=2 mode=reuse`: it requires exactly 2000
reuse stages in generations 121..2120, and no change at all to the setup,
draw, release, export or destroy totals. The fresh profile stays schema 1 and
refuses any reuse change.

Schema 7 keeps schema 5's ring records. `admitted_usec` and `slot_generation`
are null because the SDK exposes neither, and `slot` is the SDK's actual slot,
which is 0 every frame while offers settle one at a time. RingReady `slots`
and `pixel_bytes` are the configured registration bounds. Every timestamp is
when the peer observed the event, so acceptance and release may be equal.

Tests live in `tests/support/` and are compiled as private `#[cfg(test)]`
modules of the binary.
