# Matched GPU import adapters

`wl-panel-gpu` and `x-panel-gpu` are explicit build targets. The ordinary CPU
targets have no GPU dependency or behavior change. Both GPU clients use the
same in-process static bridge and scene as Sophia's peer, with two fresh
1920×24 XR24/LINEAR exports and physical GPU `0000:03:00.0` / renderD128.
No readback, SHM upload, unsafe redraw, explicit syncobj or import fallback is
available in these targets. The existing CPU workload variants are refused.

From the repository root, in a serialized cargo slot:

```
CARGO_BUILD_JOBS=2 cargo-slot niltempus sh -c '
  cargo build --manifest-path tools/gpu-bar/bridge/Cargo.toml --locked --lib &&
  make -C tools/external-bar gpu GPU_BRIDGE="$CARGO_TARGET_DIR/debug/libgpu_bar_bridge.a"
'
make -C tools/external-bar gpu-check
python3 -I -B tools/external-bar/test/gpu_cli.py
```

The GPU clients require `--lifecycle fresh --buffers 2 --workload fresh`.
Wayland also requires an explicit `--output`. Existing journal, frames and warmup
options are retained. These are compile/test instructions, not native launch
approval. No benchmark harness or qualification package is provided here.

## Import and ownership

Wayland binds linux-dmabuf **v4**, requests surface feedback and requires the main
device and a tranche advertising XR24/LINEAR to resolve to the pinned PCI device.
DRM primary/render node aliases are resolved through sysfs, not compared as
raw dev_t values. Format tables and indices are bounded and checked. Feedback
updates must complete and remain compatible; incomplete updates wait at most
five seconds before refusing. Missing tranche flags mean zero; unknown bits
and duplicate flags refuse.

Each `create_immed` has a fatal-on-failure listener retained until that buffer's
release. libwayland owns a queued copy of the FD; the client closes its duplicate
after params.add. There is no extra creation round trip. After all counted
presentation feedback arrives, the client detaches its last buffer and waits
for every release before closing the bridge. A timeout is failure, never release.

X requires DRI3 1.2, the depth-24 XR24 visual, an advertised LINEAR modifier and
a DRI3Open FD resolving to the same pinned GPU. A fresh PixmapFromBuffers is
followed by plain PresentPixmap with no fences. XCB owns/closes the outgoing FD.
The exact event ID, window, pixmap and serial must match IdleNotify before the
bridge token is released; completion alone releases nothing. Asynchronous X
errors, skipped/duplicate/foreign completions and stale releases refuse. Both
slots drain under a five-second deadline. No PresentPixmapSynced path exists.

These stacks have different retention points: X COPY can issue IdleNotify before
CompleteNotify; Wayland may retain the displayed buffer until replacement or
detach; Sophia releases its producer source after capture. Two is a common upper
bound, not a claim of identical instantaneous retention or zero-copy behavior.
Fresh allocation/import costs are intentionally inside this separate GPU lane.

## Evidence contract for the future harness

GPU headers identify `fill=vello_shade_mod128_opaque_rgb`; XR24 padding is not
an alpha assertion. `fill_start` precedes scene render/export and producer
completion, not a CPU fill. CPU headers remain unchanged.

A GPU qualifier must require exactly one of each `# gpu_producer`, `# gpu_import`
and `# gpu_end`, with exact fields/values for that transport and rendered/released
equal to frames plus warmup. It must bind the client/bridge source and binary,
device and display identities, and the existing timestamps/journal/cleanup.
These lines are self-reported supporting evidence, not authenticated custody.
The current CPU comparison harness is not extended or qualified for GPU rows.

`gpu-check` uses a mock bridge, synthetic feedback/X events, fake sysfs and a
socket-pair timeout. It never connects to a display or opens a GPU. The mock
exports memfds for FD-contract tests, not DMA-BUFs; it is never linked into either
GPU binary. Native pixel correctness (including a non-grey channel-order oracle),
real imports, implicit synchronization, release/drain and installed niri device
configuration remain separate qualification gates. Failure listener presence
does not prove that a compositor successfully imports the real export.

The XML is the installed stable linux-dmabuf-v1 protocol copied with its license.
Generated code stays in ignored build output. The bridge's local signed Lom
`file://` pin remains an experimental provenance exception, not publication or
packaging approval.
