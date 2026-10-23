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

Optional `capture_profile: true` enables cumulative producer DMA-stage timing
for a 24-row, 2120-frame, 60000-ms diagnostic. The runner must also pin
`instrumentation: "capture-profile"`; it enables the host's render counters.
Timing is disabled by default. Profile captures are attribution evidence and
are refused by cpuplan as comparison arms. Timing records go to stderr after
the counted window, leaving the image journal unchanged.

Schema 7 keeps schema 5's ring records. `admitted_usec` and `slot_generation`
are null because the SDK exposes neither, and `slot` is the SDK's actual slot,
which is 0 every frame while offers settle one at a time. RingReady `slots`
and `pixel_bytes` are the configured registration bounds. Every timestamp is
when the peer observed the event, so acceptance and release may be equal.

Tests live in `tests/support/` and are compiled as private `#[cfg(test)]`
modules of the binary.
