# Untimed consumer qualification before the matched GPU comparison

This is the implementation contract for the next device gate, **not a launch
recipe or a native pass**. `qualify.py` and `pixels.py` are device-free validators.
The diagnostic colour/hold producers, capture controller, diagnostic core, release
artifacts and sealed native package are not implemented by these validators.
No existing CPU package or timed grey-strip binary may be replaced or reused as
if it already implemented this contract.

## Fixed inputs and identity

Keep the existing Vello/wgpu/Lom export and adapter import paths, XR24/LINEAR,
1920×24, fresh allocation per generation, and at most two retained frames.
No producer readback, unsafe redraw, single-submit, W2 or destination reuse is
introduced. Qualification is untimed and must not produce comparison rows.

All paths use PCI `0000:03:00.0`, renderD128. Bind the opened node's dev_t, sysfs
physical PCI and Vulkan adapter PCI. For niri, bind installed binary SHA256
`5b8c21910f17ab1eaed6562bc54a09532c8f15dbd3abf563817158511acbe986`
and `niri.kdl` SHA256
`c2d0d6bc372046891c5126086906e8395cfd3a1a8562b1e5c6202ab10c2c7a5c`.
Its source commit remains unknown. Require actual linux-dmabuf main and compatible
tranche devices to resolve to that PCI; retain a render-device log if emitted.
X's DRI3Open FD must resolve to it, with DRI3 >=1.2 and advertised LINEAR.
Bind actual DP-1 mode 1920×1080 at 119.880 Hz, scale 1, VRR off, other outputs off.
Configuration validation alone proves none of these runtime properties.

Build receipts must bind signed source, lock/feature closure and the exact
release-profile bridge, client and host binaries. The static bridge is part of
the final client executable: bind its library hash in the link receipt, then the
executable hash to the sampled TGID/starttime, device/inode and argv. A journal's
client_pid is only supporting evidence. Preserve the existing local-pin exception
explicitly. Do not overwrite any package-pinned cargo slot or built artifact.

## Colour and hold handshake

Add a separate, explicit diagnostic build/entry point using the same exporter
and import code. It must identify its non-grey scene in its receipts and fail
the ordinary timed receipt gate. The timed grey scene and APIs remain unchanged.
Do not silently select the diagnostic by an environment variable.

Three fresh generations: 1 is left red/right blue; 2 is left green/right white;
3 repeats pattern 1. Split at x=960, all colours opaque, strip at x=0,y=0.
The primary-channel pattern detects R/B swaps, missing channels, grey fallback
and stale pattern 1 when pattern 2 is expected. No interpretation of XR24's X
padding as alpha is allowed. This follows the existing Sophia EGL oracle's two
patterns, scaled from 256×64 to the benchmark strip.

For each generation, wait for the actual consumer presentation/completion,
publish a structured READY record with generation and event identity, and stop
advancing until the controller acknowledges a successful **consumer** capture.
The controller uses bounded pipe/socket readiness, not sleeps or timestamps as
acknowledgments. Reject duplicate, stale or out-of-order READY/ACK and EOF.
A fixed outer deadline bounds driver/client/capture waits. Timeout refuses;
it does not manufacture capture, completion or release.

Keep processing the events needed to settle consumer releases during a hold.
Never confuse presentation with release. The existing native release identity
checks still own frame disposal. Retain a trace of generation/token, imported
object, completion, matching release and final close for this diagnostic only.
The first and third captures matching does not prove source-snapshot independence;
the existing EGL retained-snapshot oracle is separate evidence for that property.

## Read pixels from each consumer

| Stack | Capture route | Required binding and limits |
|---|---|---|
| niri | Installed `niri msg action screenshot-screen --path ABS --show-pointer false`, after the held generation's presentation | Same run's IPC socket and bound niri process; focused screen must be the only enabled DP-1. Capture must finish before ACK. Use the compositor's output readback for actual mode. |
| XLibre | XGetImage of the root/composed output in the same display, after exact Present completion | Verify root geometry/visual/masks, window position and output; convert through the reported RGB masks to packed RGBA. This is server-side output content, not photons or a claim of scanout timing. |
| Sophia | Admitted output observer using the existing capture role, with a separately qualified pixel-delivery helper | Same run's launch socket, namespace/caller admission, output incarnation and presentation generation joined to the held bar candidate. Read the Session's composed capture, not Lom readback. |

Sophia requires a **separate diagnostic core/profile**. The frozen matched profile
has no capture recipe. Explicit host-admin/developer-launch and an observer recipe
must restrict capture to `pci-0000:03:00.0/DP-1`. **At d9116ff0, no recipe can
admit a writable host destination.** A `capture --output /tmp/file.pam` would
write into the observer's private tmpfs, lost on exit. The recipe grammar's
`source` property does not add filesystem authority. Do not present a writable
destination as configuration available today.

Inherited stdout is **not** a suitable capture channel either. The observer
inherits Session's fd 1, not the `sophia launch` caller's pipe. At d9116ff0 the
session wrapper may discard it, write it to explicitly untrusted output, or tee
it into shared logs and the operator VT. Atomic chunks do not identify their
writer. Neither shared Session stdout nor the untrusted-output branch is capture
acceptance evidence. Do not change an environment variable to bypass that rule.

The immediate prerequisite is a separately reviewed, bounded diagnostic delivery
channel with a private endpoint bound by the process owner to this observer.
Prefer a dedicated observer stdout pipe at spawn over new writable mount authority;
this needs host/supervisor diagnostic plumbing and its own source/build/proof gate.
The normal recipe and Session stdout policy must stay unchanged. The launch caller
cannot nominate an arbitrary FD or destination through the public launch request.
The existing capture CLI emits only metadata/checksum, so a diagnostic pixel
delivery helper or CLI change is required too. Bind source/executable, process,
request and output generation; enforce total size, sequencing, deadline and full
frame checksum, rejecting truncation, duplicates and EOF. Keep pixel bytes out of
Session metadata logs and the VT; persist PAM only through the trusted private
collector. A complete same-namespace launch and private-delivery execution proof
is required before sealing. Neither checksum-only acceptance, a writable recipe
extension nor broad FD inheritance is an implicit fallback.

Confirm the exact host artifact contains the role/launch paths and DMA
feature/capability gate.
The launch helper must run in the host's user, mount and PID namespaces with a live
kernel peer; an outer SSH helper is not sufficient. Discover the socket only from
this run's Session log. No retry, guessed socket or broader namespace grant.
These prerequisites must be demonstrated device-free before sealing.

Normalize captures to the exact full-output RGBA8 PAM header accepted by
`pixels.py`, preserving channel order and dimensions. Sophia already writes this
format. A niri PNG needs a pinned software-only decoder (the rig has ffmpeg, not
Pillow); qualify its PNG→RGBA/PAM conversion with coloured fixtures before use.
Do not resize, crop, colour-correct or overwrite an existing alpha channel to
make a capture pass. A format explicitly lacking alpha (X's depth-24 root, for
example) normalizes to alpha=255; record that synthesized channel, not a measured
source alpha. X's RGBA writer needs independent red/blue mask and byte-order controls.

`pixels.py CAPTURE.pam --pattern 1|2` checks every top-strip pixel except columns
959 and 960 at the split edge, including captured alpha=255. It ignores desktop
pixels below row 23. It hashes the original capture and strip. Its result is
**pixels_match only**: no device, source/process, generation, synchronization,
release, presentation or performance proof. Never infer the expected pattern
from the captured image. Keep all three original captures and generation joins.

## Refusal, lifetime and cleanup controls

First qualify normal fresh import, completion and matching release on each stack.
Then use a separate diagnostic client instance to inject an invalid import after
the common producer has rendered (for example a correctly sized non-DMA memfd in
place of the export FD). The consumer must report an actual import/protocol error
with no fallback. A local layout refusal or a timeout is not evidence of a real
consumer refusal. Preserve the real export until the failing process exits;
never issue an invented release or continue the failed instance.

On success, stop submissions, detach the last Wayland buffer and drain exact
release events; X drains matching IdleNotify; Sophia drains its source/resource
lifecycles. Clean bridge close requires no retained frames or poison. Record
actual delay and ownership rather than requiring identical retention across
stacks: X COPY may release before completion, Wayland may retain its displayed
buffer, and Sophia releases the source after capture. A diagnostic event-delivery
delay must still end at the real matching event; time alone cannot free a frame.

Device-free controls precede native launch: corrupt each receipt/binding; swap
channels and patterns; reject stale ACK/release and mismatched token; exercise
timeout/EOF with custody retained; prove diagnostic binaries cannot qualify as
timed ones. Native controls require the genuine device and are recorded separately.

## Exit and sequence

1. Implement/review the diagnostic colour/hold entry points, capture normalization
   and process/namespace controller, including Sophia's still-open pixel-delivery
   route. Prove the refusal/identity paths device-free.
2. Produce separate signed release/link/build receipts and artifact argument
   preflights, preserving all frozen pools. Seal a fresh bounded diagnostic package.
3. Mason launches through the existing attended SSH/PAM/VT handoff. Preserve failed
   attempts; no rerun or in-place repair. Audit identity, output, captures, native
   refusal, release/close and cleanup from saved data.
4. Only a fully qualified gate permits a new matched GPU comparison package using
   the ordinary grey scene and no colour/hold/capture hooks. This diagnostic has
   no CPU comparison result. CPU shared-memory evidence and W2 remain frozen.
