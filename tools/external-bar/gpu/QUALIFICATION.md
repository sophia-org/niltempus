# Untimed consumer qualification before the matched GPU comparison

This is the implementation contract for the next device gate, **not a launch
recipe or a native pass**. `qualify.py` and `pixels.py` are device-free validators.
`capture_sophia.py` implements one private Sophia capture per declared hold;
its fixtures do not execute a Sophia session. Colour/hold producers, parent
READY/ACK drivers and local wrappers now have source controls. Protected
handoff integration, release receipts and the native package remain unqualified.
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
`58948f4c081a5f29f45af0ebff91a71d072795912eef9a5f4b87c05ba972c976`.
This diagnostic config adds `ignore-drm-device "/dev/dri/card1"` to the earlier
`c2d0d6bc` config; earlier receipts remain unchanged. Installed-binary positive
validation and an unknown-key negative control prove syntax only, not runtime
device exclusion. Before launch the wrapper requires card0/renderD128 to map
to 03:00.0 and card1/renderD129 to 16:00.0, with the expected character-device
major/minors. A boot-order change refuses instead of selecting another GPU.
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

## External colour holds and normalization (device-free source)

The separate `wl-panel-gpu-hold` and `x-panel-gpu-hold` targets take
GPU_DIAGNOSTIC_BRIDGE, requiring a bridge built with `diagnostic-colour-hold`.
Timed-source targets use GPU_BRIDGE and a separate default-feature artifact.
Debug controls use this worktree's existing cargo pool slots 1 (default) and 2
(diagnostic), with path/hash/symbol receipts. These are not release artifacts. Their C macro is DIAGNOSTIC_COLOUR_HOLD;
ordinary clients cannot select it through a flag or environment variable.
Only three frames, zero warmup, fresh exports, two buffers and the ordinary
GPU/device/import restrictions are accepted. They additionally require a
canonical `--hold-fd N` naming an inherited same-user Unix stream. The parent
owns socketpair custody; SO_PEERCRED on this pair is not writer authentication.
No new device/consumer grant is introduced. The diagnostic header's
fill=diagnostic_colour_hold_121 is refused by the timed journal qualifier.

After each exact presentation, READY names generation, declared pattern,
presentation timestamp and a fresh random challenge. The parent checks the
consumer pixels and sends the exact compact JSON ACK for this generation and
challenge. A partial ACK cannot advance, and any extra/stale/foreign record or
EOF refuses. There is no retry. Wayland continues prepare_read/flush/IN+OUT/
read_events/dispatch_pending on its display fd while polling the control fd;
X drains its existing event handler while polling X and control readiness.
Neither path frees a buffer because of ACK: only the existing matched release
events release exports. In X COPY, IdleNotify may free the client pixmap/export
before ACK; XGetImage then observes the server screen-pixmap copy of generation
g. It does not prove client-buffer retention. Wayland retains the attachment
until replacement/detach (with actual release timing recorded separately).
After the third ACK, Wayland detaches and drains;
X drains matching IdleNotify. DONE is sent only after gpu_close verifies all
three exports released. Existing XCB synchronous calls still depend on a
responsive server; the native wrapper must impose its own process deadline.

`colour_control.run` is the reusable parent sequence, not a native launcher.
Its caller supplies the pinned client pidfd, a live client/compositor/output
identity check, and a capture callback bounded to 60 seconds. It validates
exact READY fields, increasing presentation timestamps, fresh challenges and
the existing pixel oracle before ACK. The client enforces an 80-second hold;
the caller must kill/reap on refusal and require clean client exit plus the
exact release receipts after DONE. The callback must run a new capture into
private attempt files without retries. Completion of this source API alone
does not qualify the compositor, capture provenance or physical display.

`gpu-normalize-png` uses installed libpng (1.6.59 on this host). It accepts only
a bounded regular, non-symlink 1920x1080 noninterlaced RGB8/RGBA8 PNG. RGB gains
opaque alpha; RGBA alpha is preserved. It applies no gamma/profile conversion,
refuses tRNS, unknown chunks (including APNG), CRC errors, truncation and trailing
bytes, and emits the exact RGBA PAM only after a complete decode. The niri
screenshot command is still native work: `screenshot-screen --path ABS
--show-pointer false --write-to-disk true` captures the focused screen. The
native profile must leave DP-1 as the only enabled screen at scale 1 with no
transform, pin the screenshot path, and the wrapper must
bind this run's IPC endpoint/output and the fresh PNG before normalization.
Its 1920x1080 dimensions alone do not identify DP-1.

`gpu-capture-x` calls XGetImage on the sole screen's 1920x1080 depth-24 root.
It requires ZPixmap/32-bit pixels and exact RGB masks. XGetPixel handles endian
order and scanline padding; the writer supplies alpha 255 and emits RGBA PAM.
This is composed-root capture, not producer readback or a scanout measurement.
The native wrapper must require no compositing manager redirecting windows,
pin DISPLAY/XAUTHORITY and X server process/output,
run it in a dedicated private stdout pipe/file, require exit zero, and preserve
that raw result. Failed or partial output is never an accepted capture.

Fixtures use real local socketpairs and a fake event fd to check holds without
opening a display/GPU. Diagnostic variants of the adapter tests call the shared
Wayland fresh-submission path and X presentation path against mock exports and
presentation callbacks. They assert hold g before render g+1; deleting either
production barrier fails its fixture. They do not run a compositor or the full
client main loop. The X fixture delivers IdleNotify before completion/hold. PNG fixtures exercise filters 0..4, RGB/RGBA, alpha,
gamma metadata and corruption refusals; synthetic XImages cover both byte
orders, padded scanlines and wrong masks. Real Wayland/X event dispatch, fresh
consumer screenshots, GPU import, delayed release, authenticated Sophia
capture and full PAM/VT cleanup remain native qualifications. The complete
native controller/wrapper and source/build receipts remain the next gate.

## External native wrapper contract

`run_colour_external.py --recipe ABS --evidence NEW_ABS` is an explicit untimed
entry point inside the existing attended, unprivileged PAM/text-VT handoff.
It never enters another namespace or starts a CPU sampler. The outer handoff
must retain the existing seat/VT ownership checks, process-tree cleanup and
restoration receipts. Local child exits do not replace those gates.
Use a short private evidence path: the wrapper refuses niri when its runtime
path plus the 40-byte socket-name allowance exceeds 107 bytes. This is checked
before creating the directory or launching anything.

The exact recipe has `schema: 1`, `stack: "niri"|"xlibre"`, `display` (null for
niri, an unoccupied integer 1..99 for X), and `pins`. Each pin uses the existing
`{executable: ABS, sha256: LOWER_HEX}` shape, including the config and library.
Both variants require `server`, `client`, `capture`, `output`, `config`, `bridge`;
X additionally requires `xauth`. The niri `output` pin must equal `server`.
`client` is the separate colour-hold executable. `capture` is gpu-normalize-png
for niri and gpu-capture-x for X; `output` is the pinned xrandr binary for X.
The config must match this source's niri.kdl or xorg.conf byte for byte. The
X server must be the unprivileged installed prefix binary; its module directory
is derived using the existing xlibre-25 prefix convention. The package must pin
that module/config closure and bind the diagnostic static library to the client
through signed release/link receipts; a library hash alone proves no link.
X's package closure must also name xkbcomp, xkeyboard-config data and the
libinput input module, or retain their explicit host-unpinned status. The local
wrapper records that limit; it does not silently expand its six artifact pins
into a claim about every Xorg dependency.

The wrapper creates a private runtime and logs, opens/hashes each executable
and executes that retained inode via `/proc/self/fd`, including short helpers.
It pins the direct server/client by pidfd, starttime, user/namespaces and exe
dev/inode. It finds listeners in that server's fd table and `/proc/net/unix`
without a probe connection, checks listener ownership again around captures,
and refuses server DRM FDs outside 0000:03:00.0. Discovery still has a local
path-to-connect gap; the private runtime and repeated server ownership checks
bound it, not a claim that paths are immutable capabilities.
For X the filesystem listener must be unique in the network namespace; any
abstract listener at the same display address must also belong to this server.
Xtrans local connections can prefer that abstract address over the filesystem
fallback. A second or foreign listener refuses rather than borrowing its pixels.
The opened executable FD intentionally survives exec and may be inherited by
descendants; fd audits must account for this regular-file descriptor. Startup
uses a bounded kernel-listener-table recheck because bind precedes listen and
listen emits no inotify event. X does not require /tmp/.X11-unix to exist before
server startup. Niri's own server logging is separate from the helpers' quiet
RUST_LOG=error environment.

Niri validates the pinned config, then runs without `--session` or startup
commands. The single-output readback requires DP-1 1920x1080 at 119880 mHz,
scale 1, normal transform, origin 0/0, VRR false, all other outputs disabled.
X gets a fresh private cookie and the pinned non-VRR/no-PageFlip/glamor config;
one xrandr setting selects DP-1 and disables other outputs. Every readback
requires the sole 1920x1080 primary at 0/0, identity transform and 119.88 Hz.
Before/after capture readbacks use `--current --verbose` to avoid connector
re-probing during a hold; only the initial `--query` probes outputs.
The X refresh is rounded xrandr evidence; `vrr_capable` is not an enabled-state
proof. No WM or compositor is launched. X capture also refuses an owner of
`_NET_WM_CM_S0`; this is not proof against an unregistered redirector.

Each READY causes output readback, a fresh consumer capture, pixel checking and
another output readback before ACK. Niri uses an exact fresh screenshot path
with pointer false. An inotify watch installed before the action requires
CLOSE_WRITE or MOVED_TO for that file before decoding; create alone is not
completion. The PNG hash is checked before and after normalization. Helper
stdout and stderr are drained concurrently under byte/time bounds, and nonzero
exit or stderr refuses. Niri config validation suppresses informational logs
with RUST_LOG=error, not by dropping stderr. On helper failure its owned process
group is killed before reaping the leader. No capture or failed run is retried.

After DONE, the client must exit zero. The separate colour journal parser fixes
3 frames, zero warmup and the diagnostic fill, retains all existing strict
receipt/order/release checks, and joins its client PID and three presentation
timestamps exactly to the pinned child and READYs. The normal journal parser
still refuses colour headers. Successful local output is named
`colour_source_path_completed`, not a device qualification or benchmark row.
The controller requires a fresh evidence directory and saves refused attempts;
its cleanup records owned children and escalation only. The attended package
must still audit descendants, PAM/cgroup/VT restoration, real import refusal,
delayed release and the Sophia authenticated capture route before qualification.

## Protected-handoff staging (device-free)

`handoff.py stage --recipe ABS --destination NEW_ABS --run NEW_SHORT_ABS`
prepares one **unsealed** external arm. It does not invoke sudo, query seats,
open devices or start a session. The exact CPU02 niri1 `ssh-run.py`,
`vt_session.py`, `remote_user.py` and pin inventory are checked against frozen
hashes before reuse. The PAM/VT program is byte-identical; no namespace entry,
mount, device grant or authentication rule is added. Sophia uses the fixed
configuration path below; authenticated protected execution remains unqualified.

The new command invokes only `run_colour_external.py` with its explicit recipe
and fresh run directory. Wrapper/PAM/outer deadlines are 300/360/420 seconds,
so the enclosing cleanup can outlive the inner deadline. An exclusive attempt
record is created before getty is stopped; even a failed attempt cannot retry.
Check-only also refuses a consumed attempt. The remote-user log describes an
untimed diagnostic, not a CPU comparison. Output parents must exist; the run
directory must be separate from the handoff directory and short enough for
niri's Unix socket names.

Staging deliberately creates no `MANIFEST.sha256`. The generated root entry
requires exactly all nine staged input files in that manifest, then verifies
their hashes and every external pin **before importing** the privileged control
helper. A missing, empty or incomplete seal refuses. A future reviewed package
must supply the seal together with signed release/link and dependency receipts;
checksum validity alone does not establish release provenance or permission
to launch. The staging helper is not a package sealer.

The inherited system/helper hashes are retained, not silently refreshed on
drift. Legacy CPU arm and artifact paths are excluded; the explicit diagnostic
artifacts, current GPU Python helpers and configs are pinned instead. The
inventory is not a new transitive dependency proof. XKB/xkbcomp/input-module
closure and static-bridge link provenance still need the package's receipt.
The original sealed CPU package remains immutable and is an offline template
dependency; missing or changed template files refuse preparation.

`handoff.py cleanup HANDOFF_DIRECTORY` reads completed receipts only. It requires
successful PAM close/credential deletion/pam_end/TTY restoration, no residual
occupants or sessions and no outer kills, plus the wrapper's matching client
and server identities and clean local exits. It refuses a local refusal file,
forced termination, unexpected server exit, missing evidence and bool-as-int
status aliases. Its output is `handoff_cleanup_verified` with input hashes,
not a device qualification. The controller must call this after the root
handoff returns, including after failure to retain the cleanup outcome.

Fixtures render the actual pinned templates but execute only the pre-import
seal/pin checks and receipt predicates. They never run PAM, openvt, getty,
elogind or a display. Actual attended launch, server shutdown calibration,
protected Sophia capture and the device controls below remain unqualified.

## Fixed Sophia diagnostic recipe (device-free)

`sophia_recipe.py prepare --inputs ABS --directory NEW_ABS` materializes the
single-producer configuration. Inputs have exactly schema=1, a positive native
`output_generation`, and `artifacts` pins for host, peer, supervisor (bwrap),
wm, factotum, pam and environment_receiver (xtask). Each pin is `{executable: ABS, sha256: LOWER_HEX}`.
The native incarnation is a declared diagnostic expectation, not inferred
from the shell generation. The capture controller still verifies it live.

The schema-2 core has the trusted-host shell-timing catalog and exactly one
developer-launch observer recipe, `capture`, limited to
`pci-0000:03:00.0/DP-1`. It grants no writable path. The schema-1 desktop
profile explicitly enables host-admin control, excludes GPU 16:00.0 and
contains one GPU-direct bar. DP-1 is pinned to GPU 03:00.0, nominal 120 Hz
and VRR off, with no inherited outputs. The peer config declares three fresh
GPU DMA frames, height 24, hold 0 and profiling off; no reuse, W2 or other
producer is enabled. DMA source-kinds are derived by Session's capture
configuration, not invented as a profile key.

`preflight --directory ABS --evidence NEW_ABS` invokes the exact pinned host's
`--validate-session-args`, including the diagnostic capture-directory flag,
with bounded output and a 15-second deadline. It requires the exact accepted
argument count, empty stderr and no creation of the capture directory. All
generated configuration bytes and artifact hashes are checked before either
preflight or run. This parses the configuration; it does not prove live
namespace admission, the peer's behaviour, or an actual output refresh rate.

`handoff.py stage-sophia --directory ABS --destination NEW_ABS --run NEW_ABS`
pins all five generated files and all seven artifacts, then uses the same
unsealed PAM/VT staging path. Its command is explicitly `sophia_recipe.py run`.
That entry refuses root or a non-text-VT context, filters the environment to
the PAM/session identity variables plus fixed PATH/locale and the native
scanout opt-in, and installs interruption handlers so the existing sibling
Session/driver wrapper can clean up. It does not use nsenter or move either
sibling into another process's namespaces. The host limit is 280 seconds,
inside the wrapper's 300 and PAM/outer 360/420-second bounds.

Sophia cleanup qualification additionally requires the driver's terminal
`session_exited_after_captures`, its three-capture terminal record, matching
host argv, no refusal file, no forced exits, and host status 0 or SIGTERM plus
driver status 0. These checks do not reclassify the capture records as native
qualification or replace their pixel/custody audit.

Package gates still require signed host build receipts proving
`native-session,experimental-shell-capture,diagnostic-capture-delivery` and a
separate feature-required colour peer release. Argument acceptance alone does
not prove the capture feature closure. The available debug-host grammar
control uses a non-launched stand-in peer and is not a release-ready recipe.
Actual 119.880 Hz has a startup-only authenticated source gate. The fixed
argv includes `--output-environment-readback`; the wrapper supplies exactly
`SOPHIA_OUTPUT_ENVIRONMENT_ARM=1` and its private socket destination. Preflight
supplies the same opt-ins with an uncreated socket path and must create neither
that socket nor the capture directory.

The separately pinned xtask runs `shell-timing receive-environment SOCKET 15000
--ready`. The acknowledgement follows socket binding and SO_PASSCRED setup;
path existence alone is not readiness. The original receiver still checks
kernel credentials, exactly one transferred live TGID pidfd, truncation and
its sticky failure/count limits. The optional acknowledgement changes no old
invocation. stdout/stderr are bounded private files, never Session's log.

Session is spawned only after receiver readiness. The hold driver then runs
concurrently: GL identity may depend on rendering the first frame. A 15-second
startup collection window must finish with exit 0, empty stderr and exactly
one output plus one renderer record. Missing, duplicate, malformed or extra
records refuse. The wrapper pins the live Session PID/starttime/executable
and namespaces around collection, requires credentials for that host, checks
sender and receiver timestamps inside its live interval, and joins both rows
on the same output/card/readback timestamp. This is not per-hold hardware
readback. A slow startup with no complete pair refuses, without extending the
window or retrying.

The exact row is output 1/card0/DP-1, 1920x1080, nominal 120000 but programmed
timing 119880 mHz, scale 1000 and VRR off; the renderer must be amdgpu with
canonical, nonempty GL identity and no software renderer marker. The existing
DRM layout check binds card0/renderD128 to PCI 0000:03:00.0. All three capture
READY output IDs must equal the observed output. Native capture incarnation
remains independently verified by the capture controller, not inferred from
the readback's output ID or the shell generation. Saved cleanup checks bind
the environment reduction and original receiver stream by hash to the recipe.
They do not turn a synthetic fixture into native evidence.

No authenticated native environment/capture result exists yet. Preserve the
receiver's separate signed release receipt and the host feature closure. Real
import/refusal/lifetime controls remain before an attended sealed GPU package;
CPU direct-fill design/adoption now takes priority over further GPU work.


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
