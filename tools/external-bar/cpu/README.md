# Untimed Lom opaque-panel capture

This is diagnostic controller source, not a sealed recipe or native evidence.
The legacy `direct`/`copy` arms use signed `lom-cpu-reference` and a 24-row
static workspace-only panel; only the renderer line differs. The fixed
`hash-on-24`, `hash-off-24`, `hash-on-512`, `hash-off-512` arms qualify checksum
policy at either height. On uses `lom-cpu-reference`, off uses ordinary `lom`,
both with `renderer "cpu"` and otherwise identical pinned configs per height.
The package must pin both executables from the same signed source. These are
static entries: cadence and checksum-policy instrumentation records refuse.
Use one GPU-denied bar and no dock or other content producer. There is no clock,
interaction, workload driver, CPU sampler, or performance comparison.

The caller runs `capture.py` beside Session in the same user, mount and PID
namespaces. It supplies the live Session PID/starttime and exact executable
hash, private session log and capture directory, independently pinned native
output incarnation, and one of those fixed arm names. The host must have the separately
receipted private-capture feature, host-admin developer launch and a single
observer recipe named `capture` for `pci-0000:03:00.0/DP-1`. The host creates the
unused private delivery directory. Ordinary Session stdout is not pixel delivery.

The caller must wait for the initial complete Lom presented record before
invoking this helper once. The helper verifies exactly one client candidate and
its host preparation/presentation, exact static workspace indicators (workspace
1 active), one denied bar component, the live DP-1 1920x1080 nominal-60-Hz head,
and every schema-3 exact-nearest 1920-by-arm-height premultiplied sampling record. Repeated
draws may have different scene generations; these are not shell candidate epochs.
Catalog and shutdown record names are distinct from component records.

The existing private controller supplies pidfd/executable/namespace checks, a
Session-owned launch listener check without a probe connection, bounded private
launch stdout/stderr, and one CLI call. Success requires a complete delivered
receipt, empty stderr, hash-bound PAM and no refusal/partial files. It checks
Session's capture-server join through that receipt; socketpair credentials do
not authenticate the writer. Transaction 1 is only a format check. A fresh
single-attempt directory joins this launch to its result. The sole `capture-N`
group uses Session's global launch generation, which may exceed the three-capture
budget; it is not an attempt ordinal. Pin it when first observed and require the
terminal receipt to name the same generation.

The portal PAM is already tightly packed, top-down RGBA8. No PNG decoder,
vertical flip or de-striding is involved. `pixels.py` checks the exact full-frame
header/length, all 46,080 pixels in rows 0..23 or all 983,040 in rows 0..511,
opaque alpha and non-grey R/B channels. After swapping R and B, a checksum-on
arm compares FNV64 with the unique schema-1 Lom candidate. An off arm requires
schema 2 and literal `unavailable`; it returns `opaque_strip_captured` with
`produced_fnv64: null`. Its capture hashes describe the saved pixels, never a
substitute producer hash or an equality result. `check` still refuses a null,
unavailable or noncanonical producer hash.
The active workspace underline makes the R/B control meaningful. No columns
are excluded. Capture SHA256 and strip SHA256 bind saved bytes; neither upgrades
the producer's FNV64 into a cryptographic producer proof.

Both heights deliver the same full-output PAM: 8,294,400 pixel bytes plus its
71-byte header. At host commit `a56ace89116c02064b93462aa8d41e2f82dd2dab`,
`sophia-protocol/src/output_capture.rs` bounds frame pixels at 64 MiB;
`sophia-session/src/diagnostics/pixel_delivery/stream.rs` validates that bound
before allocation, and `owner.rs` publishes the full frame. The shared receipt
reader allows 1920*1080*4+256 bytes; the pixel parser requires the exact header
and size. A 512-row crop is 3,932,160 bytes. These source bounds do not establish
native h512 pixel equality; that remains a separate attended gate.

A second snapshot must match the first, with an append-only log and the same
live Session/endpoint. The native wrapper must retain the host until the helper
finishes or refuses, bound readiness and total runtime, and audit clean shutdown.
Final postflight must refuse any later candidate/indicator change and validate
the shutdown schema-3 head separately; `static.snapshot` is live-only. Bind the
actual Lom executable/TGID, no-input guard, profile, fonts, config, output readback
and all cleanup evidence in the package. Keep captures private, outside git.

Direct/copy equality is a later package check requiring configs identical except
the renderer, identical admitted model inputs and the two accepted checksums.
For checksum on/off, the package must instead compare the actual captured strip
bytes returned by `strip`, after independently accepting both deliveries and
binding equal configs, inputs, geometry and pinned executable roles. Require
the on arm's produced-checksum match, keep the off arm's producer hash null, and
report captured-strip equality separately. Never fill the off result from the
on arm or a saved checksum constant.
A mismatch is a finding, never a reason to retry. This gate covers one opaque,
scale-1 static bar. Alpha blending, scaled sampling, changing content, tearing,
other outputs, latency, CPU benefit and default adoption remain unqualified.

Offline checks (no display or GPU):

```sh
python3 -I -B tools/external-bar/test/cpu_pixels.py
python3 -I -B tools/external-bar/test/cpu_capture.py
python3 -I -B tools/external-bar/test/gpu_capture.py
```

## Fixed region-update phases

`sequence_capture.py` uses the same private delivery and executable/namespace
checks for the separately named `lom-cpu-regions-full` and `lom-cpu-regions`
entries. Its declaration fixes `arm` to `full-24`, `regions-24`, `full-512` or
`regions-512`, and `group` to 0, 1 or 2. Groups designate phases [0,1,2], [3,4,5]
and [6,7,8] respectively; there is no arbitrary phase list or retry. The caller
starts it after the first phase is Presented and retains Session through all
nine phases and shutdown, including when this group's captures finish earlier.

The held a56 host consumes at most three attempts per Session, including failed
ones. Full nine-phase coverage therefore needs twelve predeclared Sessions:
three groups for both roles at both heights. Every Session executes the same
nine embedded fixture phases. The package must bind the same executable per
role and byte-identical panel/theme inputs across groups, and audit all nine
candidate/phase records and their upload predictions in every final log.

`sequence.py` requires the fixture header before any candidate, exact phase
order, schema-2 unavailable checksums, the predicted region counters and host
Prepared/Presented joins. Both log brackets must identify the same designated
Presented phase; a next installation during capture refuses. Before/after logs,
their hashes, wall-clock capture brackets and the private delivery receipt are
saved per phase. Prior receipts/PAMs remain immutable across the three captures.
The receipt's presentation generation is the native output frame identity;
Lom's presentation epoch is the shell-candidate domain. They are not equated.

The controller budget is nine 10-second holds plus 30 seconds of slack (120
seconds); each capture still must fit entirely inside its designated hold.
This budget does not extend the Session's 120-second lifetime. The package must
start the controller promptly after observing phase 0 and record that observation
and controller start on the monotonic clock. The phase line is untimestamped, so
this records observation delay, not the exact time since Presented. A late or
straddling capture refuses without retry. Saved checksum/opaque static runs took
0.98–1.07 seconds from controller-declaration write to delivery JSON and 1.01–1.48
seconds through verification. The narrower launch-log-to-delivery spans were
0.36–0.45 seconds, but those logs are saved after launch readiness. These are
descriptive filesystem-mtime spans, not exact syscall timings or future latency
guarantees.

The capture is the visible primary-head composition excluding Session's pointer
sprite (a56 `docs/portal-client-files.md`, image-extent/occlusion/cursor contract).
Only rows [0,height) are compared. The package must compare actual full/region
strip bytes at every phase, reject equality for adjacent phases within each
group, and require equality for phases 4 and 6 (the same model, tiled versus full
fallback). Any mismatch is a finding, never a reason to repeat a run. No producer
checksum, production-entry capture or performance conclusion is supplied.

```sh
python3 -I -B -W error tools/external-bar/test/region_capture.py
```
