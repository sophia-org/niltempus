# Matched GPU producer bridge

This diagnostic static library runs Lom/Vello/wgpu inside the external panel
client process. It shares `../common/gpu_scene.rs` with Sophia's GPU peer:
1920×24 opaque shade `generation % 128`. It exports fresh, producer-completed,
single-plane XR24/LINEAR buffers. It does not reuse or redraw exported buffers.

`include/gpu_bar_bridge.h` is the C contract. The caller owns the duplicate FD
returned on success; the bridge retains the original frame until its exact token
is released. At most two frames may be retained. Generations start at one and
advance once per successful render. Releases may arrive out of order. Unknown,
foreign and repeated tokens refuse without dropping anything.

Open with the recipe's exact render node and PCI address. The matched recipe must
pin `/dev/dri/renderD128` and `0000:03:00.0`. The bridge checks the sysfs physical
device both before and after Lom checks the opened FD and selected Vulkan
adapter's DRM device numbers. The Vulkan PCI address must be present and agree.
This is device-identity agreement, not a claim that Vulkan submits through the
retained FD: the driver opens its own node. External evidence carries no Sophia
grant or authority fields.

Every refusal stops the benchmark. Render failure poisons the owner; close then
refuses so uncertain renderer state remains until process exit. An unexpected
Rust panic is caught at the C boundary and aborts the process. Valid, disjoint C
storage and serialized calls are mandatory; this API cannot validate arbitrary
or dangling C pointers. Consumer release remains the transport adapter's duty.

## Device-free checks

Run from the repository root, serialized with other benchmark builds:

```
CARGO_BUILD_JOBS=2 cargo-slot niltempus sh tools/gpu-bar/bridge/check.sh
```

Checks cover fake sysfs identities (including both rig GPUs), exact token custody,
capacity, failure state, bounded C errors, and panic abort. The linked C test
checks the actual ABI and refusal calls without opening a GPU. They do not prove
successful Vulkan construction, buffer pixels/layout, import, consumer release,
or display. Those remain device qualification gates before any CPU comparison.

## Local provenance exception

The bridge and DMA peer pin signed local Lom commit
`1a0dc2329e18fe17f9bdf08e0d9ddcb03f57fbc7` via `file://`. This is an unpublished
experimental source closure, not a public dependency or packaging approval.
Registry and renderer versions remain pinned in each Cargo.lock. The Wayland and
XLibre transports are not wired by this change. No frozen benchmark artifact is
replaced.


## Diagnostic colour entry point

Only a build with `diagnostic-colour-hold` exports
`gpu_bar_bridge_render_colour`. The ordinary symbol still renders the grey
scene. The diagnostic entry uses the same fresh export and token/FD custody,
but accepts only generations 1..3 of the shared 1920x24 patterns 1,2,1.
The C declaration requires `DIAGNOSTIC_COLOUR_HOLD`; linking that client against
a default bridge must fail. `check.sh` tests/clippies both modes and builds the default C ABI control,
without a GPU. The diagnostic C ABI link is checked separately in its isolated
slot so it never replaces the default static library. Use separate `wl-panel-gpu-hold` / `x-panel-gpu-hold`
targets; they are not benchmark artifacts.
The timed Make rules inspect the supplied library with `nm` and refuse the
diagnostic symbol before linking, including after an interrupted bridge build.
