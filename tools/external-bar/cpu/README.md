# Untimed Lom opaque-panel capture

This is diagnostic controller source, not a sealed recipe or native evidence.
Use the existing signed `lom-cpu-reference` for both CPU selections, with the
same static workspace-only panel and theme; only the renderer line differs.
Use one GPU-denied bar and no dock or other content producer. There is no clock,
interaction, workload driver, CPU sampler, or performance comparison.

The caller runs `capture.py` beside Session in the same user, mount and PID
namespaces. It supplies the live Session PID/starttime and exact executable
hash, private session log and capture directory, independently pinned native
output incarnation, and `direct` or `copy`. The host must have the separately
receipted private-capture feature, host-admin developer launch and a single
observer recipe named `capture` for `pci-0000:03:00.0/DP-1`. The host creates the
unused private delivery directory. Ordinary Session stdout is not pixel delivery.

The caller must wait for the initial complete Lom presented record before
invoking this helper once. The helper verifies exactly one client candidate and
its host preparation/presentation, exact static workspace indicators (workspace
1 active), one denied bar component, the live DP-1 1920x1080 nominal-60-Hz head,
and every schema-3 exact-nearest 1920x24 premultiplied sampling record. Repeated
draws may have different scene generations; these are not shell candidate epochs.
Catalog and shutdown record names are distinct from component records.

The existing private controller supplies pidfd/executable/namespace checks, a
Session-owned launch listener check without a probe connection, bounded private
launch stdout/stderr, and one CLI call. Success requires a complete delivered
receipt, empty stderr, hash-bound PAM and no refusal/partial files. It checks
Session's capture-server join through that receipt; socketpair credentials do
not authenticate the writer. Transaction 1 is only a format check. A fresh
single-attempt directory joins this launch to its result.

The portal PAM is already tightly packed, top-down RGBA8. No PNG decoder,
vertical flip or de-striding is involved. `pixels.py` checks the exact full-frame
header/length, all 46,080 pixels in rows 0..23, opaque alpha and non-grey R/B
channels. After swapping R and B it compares FNV64 with the unique Lom candidate.
The active workspace underline makes the R/B control meaningful. No columns
are excluded. Capture SHA256 and strip SHA256 bind saved bytes; neither upgrades
the producer's FNV64 into a cryptographic producer proof.

A second snapshot must match the first, with an append-only log and the same
live Session/endpoint. The native wrapper must retain the host until the helper
finishes or refuses, bound readiness and total runtime, and audit clean shutdown.
Final postflight must refuse any later candidate/indicator change and validate
the shutdown schema-3 head separately; `static.snapshot` is live-only. Bind the
actual Lom executable/TGID, no-input guard, profile, fonts, config, output readback
and all cleanup evidence in the package. Keep captures private, outside git.

Direct/copy equality is a later package check requiring configs identical except
the renderer, identical admitted model inputs and the two accepted checksums.
A mismatch is a finding, never a reason to retry. This gate covers one opaque,
scale-1 static bar. Alpha blending, scaled sampling, changing content, tearing,
other outputs, latency, CPU benefit and default adoption remain unqualified.

Offline checks (no display or GPU):

```sh
python3 -I -B tools/external-bar/test/cpu_pixels.py
python3 -I -B tools/external-bar/test/cpu_capture.py
python3 -I -B tools/external-bar/test/gpu_capture.py
```
