# Untimed consumer qualification before the matched GPU comparison

This is the implementation contract for the next device gate, **not a launch
recipe or a native pass**. `qualify.py` and `pixels.py` are device-free validators.
`capture_sophia.py` implements one private Sophia capture per declared hold;
its fixtures do not execute a Sophia session. Colour/hold producers, the parent
READY/ACK driver, wrapper integration, releases and the native package remain.
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

The source prerequisite is now implemented in Sophia `a56ace891`: private
observer stdout/stderr installed at spawn, with no new writable mount authority
or caller-selected FD. The controller contract below validates its result.
Normal recipe and Session stdout policy stay unchanged; pixel bytes never
belong in shared metadata logs or the VT. Actual same-namespace launch and
private-delivery execution remain required before sealing. Checksum-only
acceptance, writable recipes and broad FD inheritance are not fallbacks.

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

## Sophia controller contract (device-free implementation)

Sophia `a56ace891`, based on `d9116ff0`, implements the preceding private-delivery
source prerequisite under default-off `diagnostic-capture-delivery`. It replaces
stdout/stderr only for the exact admitted observer, binds its child after
authenticated startup, and records server-served pixels. It gives the child no
writable host mount. Delivery is custody-only, not per-message writer identity.
Actual authenticated execution remains unqualified.

The parent driver must start the helper as Session's sibling in the same user,
mount and PID namespaces, retain the producer READY hold, invoke it once, and
ACK only on exit zero. It must wait before stopping Session. No nsenter or outer
SSH invocation is supported:

```
python3 -I -B capture_sophia.py --expected EXPECTED.json --evidence NEW_PRIVATE_DIR
```

The exact expected object has integer `schema: 1`; `session` with `pid`,
`starttime`, absolute `executable` and `sha256`; absolute `session_log` and
`delivery_directory`; positive `output_generation`; and `hold` with positive
`generation`, `pattern`, `connection_epoch`, `content_grant_epoch` and
`presentation_epoch`. Generation/pattern pairs are 1/1, 2/2, 3/1. The parent must
derive the hold from its bound producer and owner records; the input file alone
does not authenticate a producer.

The helper pins Session by pidfd, starttime, executable hash/dev/inode, UID and
namespaces. The output ID comes only from this run's exact ready-head mapping
for `DP-1`, 1920×1080 with nominal `refresh_millihz=120000`. That log names a
connector without its PCI prefix. The receipt must carry the same output ID
and the fully qualified `pci-0000:03:00.0/DP-1`, which Session rechecks against
the live head at publication. Actual 119.880 Hz remains a separate output-readback
gate; the nominal log value does not prove it. The log-derived launch
socket must have one LISTEN entry and its inode in Session's fd table. There is
no probe connection. The CLI later reconnects by path, leaving a time-of-check
gap inside the owned 0700 runtime directory; no stronger binding is claimed.

One exact self-executable launch uses recipe `capture` and that output ID.
The launch client's stdout/stderr are private and bounded to 4096 bytes each
(one extra byte may be retained to demonstrate overflow). Its 35-second wait
and `Started` result establish readiness only. A separate 30-second terminal
wait exceeds Session's 20-second delivery deadline; all waits also share a
75-second outer limit. Inotify and pidfd readiness replace sleeps. Scheduling
and filesystem operations have no hard real-time bound. CLI success without a
terminal receipt refuses, with no retry.

The directory starts empty, then retains exactly the prior complete triples.
Acceptance needs JSON, matching full PAM SHA, an existing zero-byte stderr,
no partial/refused files, and the declared pixel oracle. Fields, sizes, modes
and ownership are exact; symlinks and hard-linked receipt files refuse. Receipt
exe dev/inode must equal the pinned Session and self executable used by launch.
Output/connector match the live mapping; incarnation and namespace stay fixed.
Launch generation, request and capture presentation generation strictly increase
across all three receipts, including the repeated first pattern at generation 3.

The launch-to-receipt join is **one serialized new receipt in the exclusive
directory**. Transaction 1 is a format check, not another join key. The launch result's
process is a diagnostic monitor PID, never equated to the receipt's inner PID.
The same owner candidate must remain latest presented before and after capture,
with append-only complete log prefixes and no observed intervening candidate
or invalidation. An incomplete final line is deferred, not interpreted; the
parent hold channel is essential, not replaced by log sampling. Capture
presentation generation is not shell presentation_epoch. The parent's hold
channel custody remains a native qualification prerequisite.

Session's three-attempt budget equals the three captures: any refusal ends the
run, with no skipped generation or retry. The helper kills/reaps only its own
launch client on failure; Session owns observer retirement and the parent owns
session cleanup. `result.json` means `capture_verified` under this contract,
not native qualification or authorization for timed comparison rows.

## Sophia parent hold channel (device-free source gate)

`gpu-bar-colour-hold` is a separate binary requiring the
`diagnostic-colour-hold` Cargo feature. It accepts only three fresh GPU DMA
frames at 1920x24, with no profiling, reuse or timed hold. The shared scene
renders patterns 1, 2, 1. Ordinary `gpu-bar-dma --serve` keeps the grey scene.
A held frame has already been presented and the exact predecessor's explicit
ResourceReleased has been consumed. The DMA source was released at image
settlement: the oracle checks Sophia's imported copy, not continued scanout
from the producer buffer.

`hold_sophia.py` derives the sole shell listener owned by the pinned Session
from `/proc/net/unix` and its fd table. The stderr announcement is a hint only.
It opens the existing 0700 role directory, binds `hold.pending` through that
dirfd, calls listen, then publishes `hold.sock` with renameat2 NOREPLACE.
The peer watches CREATE/MOVED_TO and connects once. The parent pins the accepted
peer with SO_PEERPIDFD, starttime and executable identity, and verifies the
single-child peer -> bwrap -> Session chain. The inner peer checks only the
controller UID; its namespace cannot establish the outer controller PID.

Each READY carries the exact grant, output, candidate, presentation epoch and
a fresh 128-bit challenge. The parent calls `capture_sophia.py` once and sends
an exact generation/challenge ACK only after the helper verifies the capture
and the before/after held-state checks still pass. Unexpected peer data, EOF,
process exit, reused challenge, a second endpoint hint, changed grant or an
explicit peer refusal ends the run. A failed peer may be restarted by Session;
that is a refusal, never a new attempt. Three attempts equal three captures.
The diagnostic profile must have exactly this one shell-content producer:
the grant check includes status=outputs and all other shell-content records;
another producer's different grant is a refusal, not a separately tracked arm.

While waiting, the peer follows SDK 0706ea2's external event loop: poll_io,
drain typed content, honor immediate readiness, poll session IN/OUT, image
readiness and the hold/inotify descriptor, and bound the wait by wake_deadline
and the hold deadline. Every content record during this phase is unexpected
and refuses; none is silently discarded or deferred into the next present.
There is no cloned 9P socket or sleeping hold. Peer JSON journals are already
buffered as complete lines. Shared GPU stderr diagnostics and the endpoint
hint/refusal are now formatted before a bounded write_all, rather than writing
individual formatting fragments into Session's log.

Generation domains are deliberately separate. READY.shell_output_generation
is the shell ContentOutputId generation. The recipe's `output_generation` and
the result's `capture_output_incarnation` are the native
OutputCaptureFrameInfo incarnation. The head log exposes no native incarnation,
so this is an explicit diagnostic input, not inferred from the shell generation
or an earlier run. Session's capture receipt must match it; a mismatch refuses.
The fixture deliberately uses shell generation 90 and native incarnation 1.
The capture request and native presentation generation must advance independently
across all three captures. No equality with shell presentation_epoch is assumed.

`run_hold_sophia.py --recipe ABS --evidence NEW_ABS` is an untimed local wrapper,
not a sealed native recipe. Its JSON has exactly schema=1, host/peer/supervisor
(each executable+sha256), argv (starting session/run), and output_generation.
It starts Session and the driver as siblings in the inherited namespaces,
adds a fresh diagnostic capture directory, and retains bounded private logs.
The driver unlinks its owned hold socket before atomically publishing
captures_verified.json; the wrapper then stops Session and waits for the
driver. Failure stops the driver first so endpoint cleanup precedes Session
shutdown. SIGTERM follows the same cleanup path. Forced kills are recorded and
refuse success. Local child exit is not PAM, VT, cgroup or device cleanup proof.

The Python fixtures exercise real local Unix sockets, pidfds, process ancestry,
listener publication/removal, three ACKs, refusal without ACK, and the wrapper's
child shutdown. The Rust tests cover configuration, ACK matching and scene
bounds. They do not open a GPU or exercise a protected metadata-shell peer.
The forthcoming native recipe must pin the diagnostic feature/core/profile,
resolve the live capture incarnation, verify actual 119.880 Hz readback and
prove authenticated observer delivery plus outer cleanup. External Wayland/X
colour holds and capture normalization are still separate prerequisites.

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
   and parent READY/ACK plus same-namespace wrapper wiring. Use the private
   delivery and controller source gates; prove remaining paths device-free.
2. Produce separate signed release/link/build receipts and artifact argument
   preflights, preserving all frozen pools. Seal a fresh bounded diagnostic package.
3. Mason launches through the existing attended SSH/PAM/VT handoff. Preserve failed
   attempts; no rerun or in-place repair. Audit identity, output, captures, native
   refusal, release/close and cleanup from saved data.
4. Only a fully qualified gate permits a new matched GPU comparison package using
   the ordinary grey scene and no colour/hold/capture hooks. This diagnostic has
   no CPU comparison result. CPU shared-memory evidence and W2 remain frozen.
