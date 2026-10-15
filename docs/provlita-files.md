# Provlita's protected file-export join

`crates/live-tests/tests/provlita_files.rs` runs the real Provlita executable
against Sophia's production shell file export. The production supervisor
launches it in a Bubblewrap metadata-shell domain. The host admits the peer
using that supervisor's PID evidence and a persistent-catalog content profile.
This is supplied conformance admission, not Session's factotum launch path.

Provlita must negotiate revision 8 over 9P2000.L and then refuse startup because
it has no direct-GPU grant. The test repeats with a fresh connection and content
grant, and requires empty content accounting after each exit. No render node,
display socket or GPU grant is exposed. There is no CPU rendering substitute.

The gate is ignored by ordinary tests and requires explicit absolute binary
and configuration paths, each with an expected SHA-256. It checks both hashes
before and after execution. Source-to-binary provenance comes from the recorded
build; these hash checks do not reproduce it or prevent concurrent file changes.
Do not rebuild the executable while the gate runs.

After provisioning the private Cargo home, run under a device-hidden wrapper:

```sh
SOPHIA_PROVLITA_BINARY=/absolute/path/to/provlita \
SOPHIA_PROVLITA_SHA256=EXPECTED_BINARY_SHA256 \
SOPHIA_PROVLITA_CONFIG=/absolute/path/to/config.kdl \
SOPHIA_PROVLITA_CONFIG_SHA256=EXPECTED_CONFIG_SHA256 \
CARGO_HOME=/absolute/private/cargo-home \
cargo-slot niltempus bwrap --bind / / --dev /dev --tmpfs /tmp \
  --tmpfs /run/user --dir /run/user/1000 --chmod 0700 /run/user/1000 \
  --setenv XDG_RUNTIME_DIR /run/user/1000 \
  --unshare-net --unshare-pid --proc /proc \
  --unsetenv DISPLAY --unsetenv WAYLAND_DISPLAY \
  cargo test --offline --locked -p live-tests --test provlita_files \
  -- --ignored --nocapture
```

Use the operator's UID in the runtime-directory arguments. The private `/proc`
must match the private PID namespace for nested protected launches to work.

## October 15 evidence

The external test uses Sophia's pinned public crates at `316d969507cdeb6ef061ea2d9645758113c53db6`.
Provlita's clean, signed source is `da4a15adb9a6555441653a9dd9ec0e4d69f3d6af`.
The existing offline release build supplied:

- Binary: `~/.cache/cargo-slots/provlita/1/release/provlita`, SHA-256
  `1bfa663d7583014d049ad5695e8ea869df33d964dbc90a60672e7feb9568be6b`.
- Configuration: Provlita's `examples/minimal/config.kdl`, SHA-256
  `c46db4786c9021a9a65d3537b53c8761c65226ab089aacab47d6e46170e192ef`.

Both epochs, 61 and 62, pass. Replacing the host's persistent-catalog profile
with a native-launcher profile fails negotiation (`UnsupportedRevision`);
restoring the profile passes. The first fixture attempt reused content-grant
generation 1 and was correctly refused as stale on the second launch. The
fixture now advances both generations. Logs are retained under
`~/.local/state/sophia/development-evidence/t252-provlita-20261015/`:
`join.log`, `join-final.log`, `wrong-profile-negative.log` and `restored.log`.

This proves protected executable negotiation, the missing-GPU boundary and
reservation reclamation on this exact pair. It does not prove authenticated
Session startup, GPU rendering, upload, Presented, input, native retirement,
three-component coexistence or performance. It also does not qualify the newer
Sophia t330 repair or change the installed release. Those remain separate
t252 checks before dock rollout.

## Authenticated pair, October 15

The external Cargo pin now selects signed Sophia
`58a481ebc756b50fe97a890b968a6c9b70863496`, including t330. The C SDK manifest
and all sixteen contract bindings are refreshed from that tree; the two added
bindings cover namespace discovery and launch authentication. Pin audit,
provisioning, device-hidden workspace tests and strict workspace Clippy pass.
The supplied-admission Provlita gate also passes against the updated pin.
The desktop flake and installed release are separate selections and have not
been changed by this Cargo update.

Sophia's existing generic `supplied_component_client_authenticates_across_launches`
test now also passes with the actual Provlita executable above, installed Lom
and installed Bemenu. This uses the real Factotum owner and authenticated
protected launch path. Provlita and Lom negotiate and then refuse the missing
GPU grant; Bemenu remains alive until explicit revocation. Each runs twice with
fresh component epochs and launch registrations, and each process is reaped
before replacement. A Provlita launch provisioned for the bar role fails with
authentication refusal 13 before shell negotiation.

The packaged Narthex `56d513fa` fails authenticated startup: it predates the
authentication implementation. Signed Narthex
`11eb0332859ed0e0b9244dd3ddafcea70c2b1b6e`, built from a clean tree, passes
`supplied_descriptor_client_authenticates_and_recovers` on the same Sophia.
It negotiates descriptor revision 8, survives the idle observation, and recovers
after revocation at two fresh epochs. Its binary SHA-256 is
`699505779b729084de04f25fc8752b417cd40671048258cd9b35a5d89d43e536`.
Select this authenticated revision for the next fallback package; the old
installed package is not evidence of compatibility with the new server.

Logs and input hashes are under
`~/.local/state/sophia/development-evidence/t252-authenticated-20261015/`.
The original Narthex failure and wrong-role negative control are retained.
The new Narthex build needed an explicit read-only bind into the outer test
sandbox because that sandbox hides `/tmp`; the first attempt without that bind
ran no client. All successful authentication checks hide display devices and
sockets. Authentication and cleanup do not establish catalog actions, content,
GPU work, native presentation or B9 performance.

## Packaged pair, October 15

Integration `77c8e96` builds both selections with Sophia `58a481ebc`,
authenticated Narthex `11eb0332` and Provlita `da4a15ad`:

- Three components: `niltempus-ad35e88b86b247648c71`, store path
  `/nix/store/89qm2775hicw4bmmxxsd4pl315vjcigh-niltempus-desktop-niltempus-ad35e88b86b247648c71`.
- Bar and launcher: `niltempus-339765fe1cc1dd8166f5`, store path
  `/nix/store/93wyap1f0lr3m3njlvnvc6vi8wfrfg16-niltempus-desktop-niltempus-339765fe1cc1dd8166f5`.

The first build omitted Provlita's embedded font; the package source filter now
includes it. A subsequent protected launch exposed an invalid configuration
mount under the Nix store. The profile now names the configuration included in
the installed release. The store remains read-only; no sandbox rule was relaxed.
The supplied-export fixture also avoids redundant mounts inside the already
visible store, allowing it to test the packaged executable directly.

Both builds pass configuration and session-profile preflight. Their complete
release checksums pass after installation into a private Bubblewrap `/opt`.
Installing the baseline, installing the dock candidate, rolling back and
returning to the dock candidate all pass. The live `/opt` was never changed.
The installer's existing corruption, locking and pruning checks also pass.

Packaged Provlita passes the supplied-export gate at epochs 61 and 62. Its
SHA-256 is `3c12afe3fbf2c5e2d942658b6c0c5a3cc3bb06c24870e8725d1372508c462103`.
It also passes the authenticated two-launch check from the private installed
path with its installed configuration. Packaged Narthex passes authentication
and revocation recovery. These checks hide devices and display sockets; Provlita
still stops at the required missing-GPU boundary.

Evidence remains in `t252-authenticated-20261015/`: `desktop-pair-final-build.log`,
`release-pair-rollback.log`, `desktop-tool-check.log`,
`provlita-packaged-export-final.log`, `provlita-isolated-installed-auth.log` and
`narthex-packaged-auth.log`. The failed attempts are retained separately.
This proves packaging and isolated release switching, not a working native dock
or attended rollback. GPU/content lifetime, three-client isolation, exact input,
B9 measurements and attended acceptance remain required.

## Render-node content and revocation

`crates/live-tests/tests/provlita_gpu.rs` is a separate ignored hardware gate.
It requires the same binary/configuration paths and hashes as the device-free
gate, plus `SOPHIA_DOCK_GPU_ARM=1`, an exact `SOPHIA_DOCK_RENDER_NODE` and its
expected `SOPHIA_DOCK_PCI_BUS_ID`. It snapshots only that node and uses Sophia's
public `ShellComponentLaunch` to revalidate identity and build the protected GPU
grant. It refuses KMS nodes and mismatched PCI identities before launch.

Run the test with `cargo-slot niltempus`, the provisioned Cargo home, and the
device-hidden wrapper above, adding only
`--dev-bind /dev/dri/renderD129 /dev/dri/renderD129` for the explicitly selected
node. Select `--test provlita_gpu -- --ignored --nocapture`. The node number is
host-specific; the October 15 run explicitly selected PCI `0000:16:00.0`, the
development GPU excluded from the live desktop. Do not expose the whole `/dev`
or infer permission from a node found by discovery. Configuration must be outside
the store because the production launch owner binds that exact private file.

The packaged Provlita identified above passes at fresh epochs 71 and 72. Its
renderer reports Vulkan on RADV, device 226:129, with only `renderD129` visible.
The actual client requests its bottom allocation, rasterizes, uploads immutable
pixels and submits a catalog-bound candidate with an authorized terminal target.
The second process uploads while the first revoked process's pixels remain held
and readable. The old completion owner refuses the live replacement's grant.
Explicit failed-render completion settles each unpresented submission; bytes
remain until the consumer drops its lease, then all accounting becomes quiescent.

The first run omitted submission completion and correctly retained an epoch even
after its pixel lease was dropped. The corrected gate checks both obligations;
it does not label an invented completion as native presentation. Wrong PCI
identity fails before any client GPU admission; an unarmed invocation fails
before device lookup. Device-hidden workspace tests,
strict Clippy and provisioning/pin checks also pass. The added public Session
dependency uses the existing Sophia revision; the pin fixture now includes it.

Evidence: `dock-gpu-final.log`, `dock-gpu-wrong-device.log`, `dock-gpu-unarmed-negative.log`,
`dock-gpu-workspace-final.log`, `dock-gpu-clippy.log`,
`dock-gpu-clippy-final.log` and `dock-gpu-provision-final.log` in the existing
October 15 evidence directory. This proves real GPU raster/upload and resource
custody across replacement. Geometry and catalog decisions are supplied, admission
uses supervisor evidence, and no Presented event is issued. It does not prove
native frame retirement, physical input, three-client coexistence or B9 bounds.

## Three-client progress under dock backpressure

`crates/live-tests/tests/shell_trio.rs` launches the candidate's actual Lom,
Bemenu and Provlita executables in separate protected domains. All three use
one 64 MiB content registry with Session's three-role capacity, nominal role
envelopes and public reconnect-limit selector. The first fixture attempt used
the registry's two-role default and correctly refused the third admission;
the test now selects the same explicit three-role capacity as Session.

On October 15, all three clients uploaded content. The test then sent SIGSTOP
to the dock and verified its stopped state through `/proc`. Catalog publication
reached bounded backpressure after 192 accepted updates. While the dock could
not read, Lom supplied another frame and Bemenu responded to a supplied text
edit with a new candidate. Revoking and replacing the dock left both neighbors'
grants unchanged; they supplied further frames while the replacement uploaded
with the old dock's pixels still held. Releasing those pixels reclaimed the old
epoch, and final teardown left all accounting quiescent.

The exact package is `niltempus-ad35e88b86b247648c71` above. Lom uses its
`examples/minimal/live-shell.kdl` (SHA-256
`a0c88fc211df32888b8ec19df06dd839f925adcbdb5e0fc7a73b71a22db754fa`);
Provlita uses this repository's `profiles/provlita.kdl` (SHA-256
`0f52a212b6a68254467600d7eafcd5ae6886c127aa7154b5954df3aff71b1369`).
Lom and Bemenu retain the exact binary hashes recorded in the authenticated-pair
evidence; the dock hash is recorded above. The sole exposed render node is the
same development GPU, PCI `0000:16:00.0`.

To run, set `SOPHIA_TRIO_GPU_ARM=1`, `SOPHIA_TRIO_RENDER_NODE`,
`SOPHIA_TRIO_PCI_BUS_ID`, and `SOPHIA_TRIO_{LOM,BEMENU,PROVLITA}_BINARY` with each
matching `_BINARY_SHA256`. Lom and Provlita also require `_CONFIG` and
`_CONFIG_SHA256` under their respective prefixes. Use the render-node wrapper
above and select `--test shell_trio -- --ignored --nocapture`. Paths are explicit
and hashes are checked before and after the run. Ordinary tests leave this gate
ignored; unarmed and wrong-device invocations fail before launch.

Evidence in the existing October 15 directory: `trio-backpressure.log`,
`trio-wrong-device.log`, `trio-unarmed.log`, `trio-ordinary.log` and
`trio-clippy.log`. The ordinary invocation verifies exclusion, not a hardware
pass. Strict Clippy and formatting pass. The failed initial admission is retained
as `trio-run1.log`; `trio-run2.log` is the earlier pass without forced saturation.

This is actual-client content and slow-reader isolation evidence with supplied
admission, geometry, completion and launcher input. Logs saying `presented`
reflect those supplied receipts, not a physical display. Post-revocation client
connection errors are expected during teardown. Native lifetime, exact physical
input/reservations, the separate baseline, B9 and attended acceptance remain.

## Separate two-component baseline

`two_component_baseline_keeps_bar_progress_during_menu_replacement` in
`shell_trio.rs` runs separately with the Lom and Bemenu binaries from baseline
release `niltempus-339765fe1cc1dd8166f5`. It selects the two-role registry capacity
and Session's two-role byte envelopes; it does not reuse the dock's reduced
launcher allowance. The explicit artifact/hash variables are the same as above,
but Provlita inputs are unnecessary. Filter this test by `two_component_baseline`.

The October 15 run passes: both clients upload, a supplied text edit produces
another menu candidate, and Lom continues across menu revocation and replacement
without changing its grant. The replacement uploads while the old menu's pixels
remain readable. Releasing those pixels reclaims the old epoch; final teardown
is quiescent. `baseline-run.log` records the exact package paths and hashes.
The three-component case passes again after sharing the envelope selection
helper (`trio-after-baseline.log`), as do strict Clippy and formatting.

This supplies separate baseline content/lifecycle evidence. Its completion and
input are simulated just as in the trio gate; physical baseline acceptance and
the measured B9 comparison remain open.
