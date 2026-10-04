# niltempus installer

Go installer and release assembler for the niltempus desktop. Its source lives
alongside the Rust packaging and qualification tools in this repository.
Sophia owns the compositor and session mechanisms; Hagia owns WM policy; Lom
and Bemenu provide this desktop's panel and launcher. Narthex is packaged for
Sophia's existing shell support. Desktop composition belongs here, outside
those repositories.

## Commands

Build retries reuse Cargo targets under the private cache's `package-targets`
directory. Each attempt keeps its own logs and release output. The build lock
serializes access; Cargo checks source and compiler changes on every attempt,
and signed source, toolchain and release verification still run.

If a system update changes a reviewed tool such as Bubblewrap, the build refuses
it and reports the executable path and expected/actual hashes. Generate and
review replacement dependency manifests, update their configured paths and
digests, then run `niltempus plan` and `niltempus build` again. Keep the old
manifests as evidence for releases built with the earlier toolchain.

For the next session, run this as your normal user:

```sh
niltempus install
```

It verifies and installs the prepared release, prompting for sudo. If no release
has been prepared, it builds from the configured sources first. Select
**Sophia niltempus Desktop** at your next login. A failed build or validation
stops before installation.

New releases are 9P-only (plan schema 3). The one login entry, **Sophia
niltempus Desktop**, runs Hagia, Lom and Bemenu over 9P2000.L. The release
contains one preflighted `desktop.kdl`. Installation writes no current-IPC
entry, and removes one left by an older release when that release is not the
selected one.

Plan schemas 1 and 2 (external release manifest schema 6, with the
**(current IPC)** entry) remain readable only so that installed releases can
be verified, re-activated when recorded, and rolled back to. Their original
files and checksums are unchanged. Plan schema 3 is the Go desktop plan. It
requires external (Sophia/integration) release manifest schema 7, a
separate numbering.

The individual steps remain available:

```sh
niltempus plan
niltempus build
niltempus prepare /path/to/an/existing/release
niltempus verify /path/printed/by/build
niltempus install /path/printed/by/build
niltempus status
niltempus rollback
```

`build` automatically selects its verified output for the next `install`.
`prepare` selects an existing release without rebuilding it. The selection in
user state binds its path, release ID and manifest digest. `install` verifies
these again; a missing or changed selected release is refused without silently
building or choosing a different one. `install DIRECTORY` remains an explicit
one-off installation and does not change the prepared selection.

**Upgrading from a Plan-2 selection.** A selection made before plan schema 3
(a Plan-2 release, with the current-IPC entry) is refused by `install`,
because new installations require plan schema 3. There is no silent
fallback: `install` neither rebuilds nor picks another release. To move on,
prepare a Plan-3 release explicitly, either with `niltempus build` using the
explicit `inputs`, or with `niltempus prepare DIRECTORY` for an existing
Plan-3 release. Until then, `prepared.json` is left exactly as it is.

### CPU and kleis lock-provider candidate

Sophia `e9757abfa3b4dc6bf714a21df0c63d29ca4d1a96` vendors the C desktop SDK
v0.8.0 (`b2a254dcb792e5f9d66f78bdd73f645153504507`) and Rust SDK v0.3.0
(`a111b5590d27fe2dc079220e821f0327f47baefb`) on master `fb4b81f92`. That master
carries t289's qualified CPU harness, repaint attribution and capture
configuration reuse, the lock provider role (t294) and the lock model (t296).
The capture change measured 39-40% less capture-path process CPU in an
offscreen benchmark; whole-desktop savings are not measured.

Both SDK releases add an experimental lock provider role: the lock contract
is still revision 1 (draft), and the C SDK declares `lock_files=false`. The
fourteen paired contracts add the three lock files, and the C SDK manifest
moves to v0.8.0.

The products move to that SDK: Hagia `0a6b4a52d40d073d289fa01d30860bfe5ac0ed30`,
Bemenu `8f8c766b8ac10e41ff3e30e97e75e61847a02832` and kleis
`342a4a50b6c43806f7f32c0b9e45c3e08c1ebac1`. Hagia and kleis need reviewed Nim
dependency manifests for those commits (`inputs.hagia_nim_deps`,
`inputs.kleis_nim_deps`), and `inputs.hagia_c_sdk_revision` names v0.8.0.

Installation does not replace the personal Hagia, and every component that
vendors the C SDK must match the installed desktop's SDK. So the components
are prepared after installation, and the lock provider is added last. Run
these as your normal user from a TTY or after logout, in this order:

```sh
niltempus install
niltempus prepare-component hagia
niltempus prepare-component bemenu
niltempus prepare-component kleis
```

Then add the `lock-provider` block and a `session:lock` binding to
`~/.config/sophia/desktop.kdl`. Login refuses a profile that names a lock
provider while no kleis is selected. Do not reload the profile in a session
started before the installation: its Sophia refuses the `lock-provider` node.
Log out and log into **Sophia niltempus Desktop**.

### Earlier lock-test candidate with Present timing repairs

Sophia `53b2833708c3e4349b7c3b82cb4168fb28e424c6` includes the Engine lock
cover, Session lock state and input routing, and the Session-owned factotum
and PAM helper (t291-t293). The release packages both authenticator binaries;
the installed launcher passes their paths to the Hagia session. The later
lock-provider role (t294) and C SDK lock-role work (t295) are not included.

This revision also carries t289's owner-service and Present timing work, with
two release repairs. A visible head whose sequence ioctl is unsupported uses
an Unclocked source instead of the hidden-window 1 Hz clock. Pixmaps retain
their fence and buffer custody, Complete uses actual retirement UST with a
plateau MSC, and NotifyMSC waits one mode field. New visible windows select
their clock from Session placement before their first pixels have committed,
including pending admissions and popup map/remap. Hidden surfaces stay on the
background clock. The W1 admission cache remains separate and is not included.

The isolated Sophia full gate passed (6,964 printed passing tests, zero
failures), along with the release build and exact-freeze negative controls.
The frozen binary completed 111/110 Presents per 10 seconds on two virtio
windows, versus 106/106 before t289. The startup composition queue gap was
77.307 ms, versus 719.983 ms before the first-frame repair and 54.745 ms at
base. All recorded bindings were Unclocked, Complete and Idle matched sent,
and completion USTs matched actual retirements. These are single guest
correctness samples; they do not qualify CPU performance, physical vblank
behavior, or live locking. Evidence: `t289-clockless-01/GATE-02.json` and
`FIRST-FRAME-GUEST-COMPARISON.json`.

Hagia stays at `03be1d1f2b1c9e555773578ec1001dd7c098830a`, with C SDK
`4608010f31848c153d5c0918b149b9f86a239048` (0.7.0). The eleven paired
contracts, C SDK manifest, fonts and Nim dependency manifests are unchanged.
The vendored Rust SDK includes the queued-secret write fix from `9e59d78`.

After qualification, plan review and the package audit, publish the matching
CLI and update the configured Sophia and niltempus references. Then run
`niltempus install` as the normal user from a TTY or after logout, and log in
again. Installation preserves the matching personal Hagia; no `prepare-hagia`
is needed. Lock testing additionally needs the operator's root-owned
`/etc/pam.d/sophia-lock` stack (see Sophia's `examples/pam.d`) and a
`session:lock` profile binding. The authenticator starts only when its PAM
stack exists. Profile editing and attended lock/unlock acceptance are operator
steps; this candidate does not change the personal profile.

### Earlier obsolete-control recovery candidate

Sophia `b6ad18cff13f84ae05626bb060626cb01ab96699` repairs stale Session
controls after a client unmaps or withdraws its admission. The incident on
`niltempus-9de41ea905db10201b9e` ended with `control_rejected`; its archive
did not retain the control kind or rejection reason.

Obsolete controls now retire only their matching transaction and surface's
obligations. Superseded focus claims leave newer focus authority intact.
Invalid commands and genuine authority failures retain their fatal handling.
Archives now retain the control kind, generation and scalar outcome.

The Sophia full gate passed with 6,625 printed passing tests and zero failures,
including real-writer lifecycle and Session settlement regressions. Five
existing architecture models and their two negative controls passed their
expected checks. These are isolated proofs; installed-session acceptance of
this repair remains open. Evidence: `session-exit-9de41ea9-01/repair-01` and
`t290-release-01`. The t289 CPU implementation is not included.

Hagia remains `03be1d1f2b1c9e555773578ec1001dd7c098830a`, with C SDK
`4608010f31848c153d5c0918b149b9f86a239048` (0.7.0). The WM contracts,
SDK manifest, Nim dependency manifests and approved profile are unchanged.
For an installation already using this personal Hagia, run as your normal
user from a TTY or after logout, then log into **Sophia niltempus Desktop**:

```sh
niltempus install
```

The installer requests sudo itself and preserves the matching personal Hagia.
No `prepare-hagia` is needed. Update the configured Sophia and niltempus
references to the reviewed candidate before planning, and publish the matching
qualified CLI before handing off installation. Installation itself uses the
verified prepared release.

### Earlier cross-head switcher recovery candidate

Sophia `f650e68831f15a7e8b767dac88c598aebe515e51` repairs the renderer image
custody failure exposed by multi-head Alt+Tab on `niltempus-d99119dbc46629083eea`.
The earlier session exited when a preview referenced an image owned only by
another output's renderer worker.

Foreign previews now carry bounded immutable snapshots. Queued local reads
keep their images alive, ordinary moved windows acquire a local copy, and
preview updates do not change the source window's Present clock. Attributed
preview failures withdraw the publication and recover unsubmitted frames;
other renderer and device faults remain session-fatal, with typed diagnostics. Cross-device
previews remain unsupported and are refused.

The full Sophia gate, strict Clippy, architecture models, 35 named fault
controls, and offscreen pixel/worker tests on both render nodes passed.
Native owner-loop, physical KMS/input and attended multi-head Alt+Tab
acceptance remain open. Evidence: `t279-alt-tab-session-exit-01` and
`t284-release-01`. The repair retains the renderer performance, idle-wakeup
and chording changes from the previous release.

Hagia stays at `03be1d1f2b1c9e555773578ec1001dd7c098830a`, with C SDK
`4608010f31848c153d5c0918b149b9f86a239048` (0.7.0). Keep the existing Hagia
Nim manifest and `inputs.hagia_c_sdk_revision`. The approved personal profile
and switcher bindings are unchanged. For an installation already using this
personal Hagia, run the following from a TTY or after logout, then log into
**Sophia niltempus Desktop**:

```sh
niltempus install
```

Run it as your normal user; the installer requests sudo itself. Installation
preserves the matching personal Hagia, so this repair needs no
`prepare-hagia`. An installation still using the older SDK 0.6.0 Hagia needs
the paired preparation procedure below.

### Earlier renderer and held-switcher candidate

Sophia `9679b820dabf5ca1406eacb919003232de6b88c9` combines t278's renderer
improvements with t279's held keyboard capture. Pair it with Hagia
`03be1d1f2b1c9e555773578ec1001dd7c098830a` and C SDK
`4608010f31848c153d5c0918b149b9f86a239048` (0.7.0). Both Sophia changes
retain the existing idle-wakeup and generic chording work.

Captures reuse bounded EGL execution resources while each exported image keeps
fresh immutable storage. CPU raster reuse ignores GPU-only changes. Damage
history follows preparation identities and repaints fully when it cannot prove
a precise update. Three matched 60-fps capture benchmark pairs use 91.4–91.7%
less process CPU per frame; this is a path measurement, not total live desktop
CPU. The combined workspace gate, strict Clippy, architecture models and pixel
equivalence on both render nodes pass. Evidence: `render-performance-01` and
`t278-t279-release-01`.

The held switcher now accepts Escape, Return/space, arrows, Home/End and scope
keys. The existing session close operation targets the selected candidate.
No global Alt+q shortcut is added. Same-application filtering, labels, urgency
and debounce remain later work. Its first attended multi-head Alt+Tab ended
the session; the recovery candidate above addresses that failure.

Set the source references, reviewed Hagia dependency manifest and
`inputs.hagia_c_sdk_revision` together before planning or building. Use the
paired `install` then `prepare-hagia` procedure below from outside the old
graphical session. Installation alone preserves the existing personal WM.
An explicitly approved personal profile, including any staged switcher
bindings, is sealed by `build`; the installer does not invent those bindings.

### Earlier idle-wakeup and chording candidate

Sophia `9d3a41904ac2994bf00980b3399bffb27c43eb04` combines t276's idle
notifications with t277's generic chord lifecycle, modifier taps, holds and
sequences. Pair it with Hagia `252ee7efb6951bc81e5bedd6f6d1c3263ac72b97`
and C SDK `8f59a9cd3221776246aad70c3a178c5ba941fb0d` (0.6.0). The Sophia
workspace passed 6,206 tests across 405 binaries with all features and targets,
plus strict Clippy, layout, generation and both SDK checks. Evidence is in
`t276-t277-integration-01`; live chording and CPU improvement still need
attended acceptance. A profile-reload cancellation may await the next owner
maintenance turn, whose requested wait is capped at 25 ms.

Set the explicit source references, reviewed Nim manifests and
`inputs.hagia_c_sdk_revision` to the matching SDK commit before `build`.
Building verifies the new Sophia/Hagia pair and selects the release without
changing the running session. Installation preserves an existing personal
Hagia, so this SDK upgrade also needs:

```sh
niltempus install && niltempus prepare-hagia
```

Run the line as your normal user; `install` requests sudo itself. Complete both
commands before the next login. `prepare-hagia` verifies against
the installed release, so it must follow installation. It selects the matching
personal WM without signalling the running one. Until logout, avoid a WM
restart in the old session: a restart would use the newly selected Hagia.
If preparation fails, finish repairing the pair before logging into the new
release. Desktop rollback alone preserves the personal Hagia selection.

Personal shortcut bindings remain user-owned. Hagia's shipped profile binds
Alt+Tab and Alt+Shift+Tab, but an existing personal profile does not acquire
those bindings from installation.

### Explicit helper inputs

Plan schema 3 packages with the current helper CLI, so its configuration
names every input explicitly under `inputs`. There are no defaults:

- `hagia_nim_deps` and `narthex_nim_deps`: each is the path of the REVIEWED
  Nim dependency manifest plus its sha256, supplied independently of the
  file.
- `hagia_c_sdk_revision`: the C SDK revision that Hagia vendors.

`plan` refuses a missing, malformed or mismatched input before anything is
staged, and `build` checks them again before creating any directory. Each
manifest must:
- be a regular file at an absolute path;
- hash to its supplied digest;
- start `nim-deps schema=1 status=reviewed`;
- name its product and the plan's exact source commit.

A draft manifest authorizes nothing. The build copies the reviewed manifests
into its private directory, checks them again, and passes them to
`prepare-wm-pair` with `--build-dir`, `--hagia-nim-deps`,
`--hagia-nim-deps-sha256`, `--narthex-nim-deps`, `--narthex-nim-deps-sha256`
and `--hagia-c-sdk-rev`. It then runs `package-desktop` with
`--wm-pair-profile-sha256` and `--wm-pair-c-sdk-rev`. The plan binds the
paths, the digests and the revision, so they are part of the release
identity.

### Release verification

A schema-3 release must carry an external manifest of schema 7 that:
- records `hagia_c_sdk_revision` (equal to the plan's) and
  `hagia_c_sdk_manifest_sha256`;
- seals `share/sophia-policy/hagia/c-sdk.manifest.json`, which must hash to
  that digest and name that revision.

Missing, repeated, malformed or mismatched values are refused. With
`hagia_included=false`, the SDK fields and the sealed manifest must be
absent. During a build, the source-bound Rust `verify-release` command checks
the package before the Go assembler adds its desktop files and seals the final
release. The Go verifier retains independent checks for that final shape and
for installed legacy releases. Neither step executes a candidate's bundled
verifier as its source of trust.

Activation follows the integration activator's history in
`/opt/sophia-niltempus-desktop/activated-releases`, whose lines are
`release_id manifest_sha256 SHA256SUMS_sha256`:
- A release activated before, and unchanged since, keeps its own
  verification. This is how an installed legacy release stays a rollback
  target.
- A recorded ID whose contents changed is refused.
- A release never activated must be a plan-schema-3 candidate.
- An installation without a ledger counts its current and previous
  releases once. After that, only ledger entries count.

`plan` is the default and reads local refs only. It never fetches, pulls,
checks out a branch in a working repository, or takes uncommitted edits.
Update each repository's local `master` deliberately before planning if newer
sources are wanted. Change a `reference` to an exact SHA to hold that component.
Each plan records the resolved commits and Git signature status (`G`: valid
trusted signature; `N`: unsigned). Present signatures must verify. Unsigned
commits remain visible as such.
The niltempus packager requires trusted signatures for its own source,
Sophia, Hagia and Narthex.

`build` checks out those exact commits in private clones. The signed
niltempus Rust tool prepares the WM pair and packages Sophia;
the personal assembler adds Lom, Bemenu and the single 9P-only login profile. Changed or
unexpected cached inputs are refused, never reset over. Bemenu uses a fresh checkout because Make
does not track its embedded commit's compiler flags. Every attempt records the
source paths, plan, stage logs and elapsed times; cached checkouts may advance on
the next build, but these records and sealed releases remain.

WM-pair builds run serially and build only the packaged executables;
Builds inherit the caller's priority and use available CPUs by default.
Set `CARGO_BUILD_JOBS` to a positive integer to override Cargo, Nim and Make
parallelism. Git checkout and compiler children use umask
022, independently of the caller, so a permissive shell cannot create a profile
that Hagia rejects as group-writable. The original repositories and profile are
never chmodded. The WM pair binds Hagia's shipped profile by its digest, and
the assembled release's profiles are checked before sealing.
Sealing sets release directories to mode 0755, including directories added by
the Go assembler under a private umask. Verification checks these modes before
installation, so root ownership cannot make the session profile inaccessible.
Private cache and state parents retain their permissions.
Build and profile-check commands run without network access at the caller's
priority under Bubblewrap with DRM
devices, display sockets, runtime sockets and inherited Sophia variables hidden.
The repositories' own build dependencies must be available; Cargo builds are
offline. Build does not install, switch a release, start a session or probe GPUs.

The explicit `niltempus` repository entry selects both this installer and the
packaging tools. The retired `integration` setting is refused for new plans.
The installer's Go build information must identify the same clean Git revision;
an independently built or dirty installer cannot plan a release.
The committed Sophia pin must equal the selected Sophia commit. Provision
that niltempus revision explicitly before planning; its `.provision/accepted`
must match the committed lockfile and a private Cargo home outside the source
trees. The plan records that home, lockfile digest and signed tool commit.
Build revalidates the acceptance and runs the tool's pin/provision checks;
it never downloads dependencies or provisions a cache implicitly.

Packaging and activation scripts come from that niltempus revision.
The outer manifest records the five product commits, the niltempus binding,
the source profile hash, the
installer executable hash, and every packaged file's hash and permissions.
Release IDs identify these inputs, not a claim of bit-for-bit reproducible
compiler output. A reused release is fully verified before being returned.

## Profile and login

Settings live in `~/.config/sophia-niltempus-desktop/config.json` (override with
`--config FILE` before the command). The default profile input is
`~/.config/sophia/desktop.kdl`; it is read, never edited.

Each release contains a generated copy with packaged Hagia, Lom and Bemenu paths
pinned to that exact release, `Super+o` bound to `policy:toggle-overview`, and
`session { control "host-admin"; }` enabled. A conflicting or duplicate binding
is refused, as is ambiguous or malformed control configuration. Other profile
settings remain in the copy,
including the external Lom config path and application choices. This personal
assembler expects one WM, one bar and one application launcher. Includes must
first be flattened with Sophia's `config print-effective` command.

KDL transformations use the pinned [kdl-go parser](https://github.com/calico32/kdl-go),
followed by the newly built Sophia `config check-session-profile` preflight
against the newly built Hagia. Parser success alone is not profile acceptance.
The installer supplies a Hagia-specific policy adapter to Sophia's generic
`--policy-checker` interface and requires exactly one `policy=validated`
verdict. Missing validation and explicit deferral both refuse the build.

`install` uses the prepared release; `install DIRECTORY` selects an
explicit already built release for that invocation. Installation requests sudo, installs into
`/opt/sophia-niltempus-desktop`, and
registers **Sophia niltempus Desktop** as a separate login choice. It leaves the
existing `/opt/sophia` installation and session entries in place. The new launcher
selects the generated profile and explicitly overrides the WM with the user-owned
Hagia at `~/.local/state/sophia-niltempus-desktop/development/hagia`, honoring
`XDG_STATE_HOME`. Installation seeds that path from the verified package only
when it is absent. An existing owned executable and its preparation metadata
remain unchanged, including their permissions and timestamps. Use
`prepare-hagia` to replace the personal WM deliberately. Sophia's native
launcher retains native scanout for overview.
Before each launch, the selected personal binary must confirm the generic
`SOPHIA_WM_POLICY_*` environment contract through its bounded
`config check-environment-contract` probe. Older personal binaries fail closed;
the installer also checks this probe before building a paired release or
publishing a development WM.
The normal login entry needs no separate development variant; installation
removes the earlier development entry.
Activation selects `current`; the former release is retained as `previous`.
`rollback` selects that previous release and preserves the personal Hagia.
Neither command
logs out or restarts a running session. External application and Lom config
files stay user-owned and are not rolled back with the release.

## Component updates

See [Independent component updates](../docs/component-updates.md) for the full
selection, verification and restart contract. After one login with an updated
launcher, use `niltempus reload lom`, `niltempus reload bemenu`, or
`niltempus reload hagia`. `restart NAME` skips the build;
`prepare-component NAME` selects without signalling, and
`rollback-component NAME` restores the previous component.

The generated login profile selects stable user-owned component paths. The
sealed desktop and its launch policies stay intact. Existing sessions using
sealed executable paths require one new login to adopt this setup.

### Hagia restart

The normal **Sophia niltempus Desktop** login uses the user-owned Hagia path.
Sophia and the base packaged components remain root-owned under `/opt`. The
packaged Hagia is retained for provenance and initial setup; restarting the WM uses
the prepared personal binary, with no temporary release swap.

For WM source changes, run:

```sh
niltempus reload-hagia
```

This builds only signed Hagia source with its reviewed dependencies, validates it against installed Sophia
and the installed desktop profile, atomically replaces the user-owned executable,
and requests restart through Sophia's control IPC. It never invokes Cargo or
rebuilds Sophia or the shells. An unchanged, hash-verified Hagia build is reused.
`prepare-hagia` performs preparation alone, without IPC or any live action.

When the desired binary is already prepared, the direct command is:

```sh
sophia msg session restart-wm
```

Sophia enables the socket from the generated profile at startup and exports
`SOPHIA_CONTROL_SOCKET` to host applications. A session started before this
configuration was installed must be logged out once and started again. Setting
an environment variable cannot turn on the endpoint in an existing session.
The `host-admin` setting deliberately admits session control from authorized
host processes; it does not expose a WM-owned socket or delegate raw input.

The helper requires an explicit control endpoint and exactly one matching
user-owned Hagia with an owned checkpoint. It checks the replacement PID and
binary hash after the CLI confirms owner settlement. An IPC refusal stops the
helper; it does not fall back to signals, sudo or pathname swaps. Overview
closes on replacement. Sophia and application processes continue running.

The first system installation needs sudo to update the release and normal login
entry. Subsequent Hagia preparation and IPC reloads require no sudo. Tests use
private processes and a CLI fixture for executable replacement, and private
mounts for the real installation/rollback scripts. Live acceptance is separate.

## Building and contributing

Build from a clean, signed niltempus revision. Go 1.25.5 or newer is required.
Provision the pinned Go dependencies explicitly first; the build below refuses
network downloads. Put the binary outside the checkout so it stays clean:

```sh
cd /path/to/niltempus/installer
go mod download
mkdir -p "$HOME/.local/bin"
GOPROXY=off GOSUMDB=off go build -mod=readonly -trimpath -buildvcs=true \
    -o "$HOME/.local/bin/niltempus" .
```

The config, cache, state and system release paths retain their historical
names so existing installations and rollback records remain usable. Chezmoi may
manage personal settings; it no longer owns the installer source or launcher.
Rust is not required to compile the Go installer itself.

Start from [examples/config.json](examples/config.json), replace every source
path and revision, and supply your own Sophia profile and application choices.
This assembles niltempus's selected components; Sophia and its independent C and
Rust SDK repositories remain usable with other desktops.

Caches live in `~/.cache/sophia-niltempus-desktop`; clones, logs and uninstalled
release artifacts live in `~/.local/state/sophia-niltempus-desktop`, except the
private source clones under the cache's `sources` directory. XDG base
directory overrides apply. These generated files are not tracked by chezmoi.

```sh
cd /path/to/niltempus/installer
go test ./...
go vet ./...
```

Tests cover profile preservation, exact executable selection, control opt-in, overview-binding
conflicts, release tampering, provenance changes, symlink refusal, inherited
session-variable removal and exclusive build ownership. Headless validation
does not establish physical GPU or live-session acceptance.

Nested Bubblewrap passed with the actual reviewed-dependency WM-pair build
and the installed Hagia component builder. See the
[component update release evidence](../docs/component-update-release.md).

Build regressions cover a real Git checkout under umask 0002, safe child output
permissions, unchanged source inode/mtime retention across commits, dirty-cache
refusal, source-repository isolation and failure timing in logs. Compiler caches
are disposable; removing them causes a cold build. An unchanged, already sealed
release is hash-verified and reused without invoking compilers.

The optional integration control uses a real built package and the real CLI to
exercise installation, repeated installation, a second release and rollback.
It replaces `/opt` and the display-manager entry directory with private mounts,
makes the host filesystem read-only, hides devices, and substitutes an
unprivileged sudo fixture. It cannot install into the host's `/opt`:

```sh
go build -mod=readonly -trimpath -buildvcs=true -o ~/.cache/sophia-niltempus-desktop/installer .
DESKTOP_INSTALLER_TEST_RELEASE=/path/printed/by/build \
DESKTOP_INSTALLER_TEST_BINARY="$HOME/.cache/sophia-niltempus-desktop/installer" \
    go test -run '^TestInstallAndRollbackInPrivateMounts$' -v
```

Commit source changes here. Keep personal configuration, credentials, provisioned
caches and built releases out of Git. The build identity check is intentional:
after changing this installer, commit it and rebuild before planning a release.
