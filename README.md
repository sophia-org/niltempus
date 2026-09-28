# niltempus

My desktop assembled from Sophia, Hagia, Lom and Bemenu. This repository owns
the Go installer, pinned build recipes and tests for that particular choice of
components. It also provides an example for contributors assembling their own
desktop. Sophia stays shell and WM agnostic; the C and Rust desktop SDKs remain
separate repositories.

The entrypoint is `niltempus install`. It verifies the explicitly prepared
release, or builds from the configured inputs when none is selected. See the
[installer guide](installer/README.md) for initial setup, reviewed dependencies,
the Plan-3 migration and rollback. A changed or invalid selection is refused.

The former `sophia-desktop-integration` work is consolidated here. Historical
manifest fields and installed paths retain their names for compatibility.
Conformance tests for Sophia's generic contracts stay in Sophia; tests of this
desktop's components and combinations live here.

Session recipes select shell components from the desktop profile. They no
longer pass the legacy `--shell-process-default` fallback; setting
`SOPHIA_HAGIA_SHELL_BIN` does not add a shell to the session.

New packages do not build or seal the retired `sophia-wm-demo` IPC client.
Historical runtime-identity records may still name it as `unavailable`, and
archive verifiers continue to accept their bound older binaries. The old
mixed-output physical recipe requires a Sophia pin that still contains that
demo; it is not a 9P WM/output proof. Replacing that recipe belongs to the
separate output-role migration, before moving its pin to a demo-free Sophia.

## Documentation

- [Qualified 9P candidate](docs/final-9p-candidate.md): exact binaries, gates
  and the approved preparation for installation.

- [Operations](docs/operations.md): the installed runbook, bound Nim
  dependencies and prepared physical inputs.
- [Installed candidate](docs/installed.md): package, install and verify a
  release.
- [Physical runners](docs/physical-runners.md): every attended hardware gate
  and what it requires.
- [Hagia gates](docs/hagia-gates.md) and [Hagia workspaces](docs/hagia-workspaces.md).
- [Narthex reference sheets](docs/narthex-reference-sheets.md).
- [Same-hardware desktop comparison](docs/desktop-comparison.md).
- [Lom content](docs/lom-content.md) (9P transport and scoped proof coverage).
- [Quickshell](docs/quickshell.md): the X11 trace probe and panel probe, with
  the old coverage map.
- [E1 descriptor hosts](docs/E1-descriptor-hosts.md): retained coverage of the
  descriptor host modes (a 9P seam for the Narthex half).
- [Gate mapping](docs/GATE-MAPPING.md): where every moved Sophia gate and test
  lives now, and what is retired with product IPC.
- [Sophia deletion list](docs/SOPHIA-DELETION-LIST.md): the Sophia-side half of
  the move, with the inbound hunks.

## Inputs and pins

- Sophia's public crates (`sophia-runtime`, `sophia-protocol`) come from
  `https://github.com/sophia-org/sophia.git` at the revision in
  `pins/sophia.toml`. No sibling path is named anywhere.
- `pins/c-desktop-sdk/manifest.json` is Sophia's vendored C SDK manifest at
  that revision; `pins/contracts.sha256` holds the eighteen protocol contract
  digests a client's SDK snapshot must carry.
- `assets/fonts/` holds the pinned test font and its license, copied from
  Sophia at that revision.
- A Bemenu artifact is named only by explicit arguments and environment
  variables; no client checkout path is built in.

`cargo xtask check-pins` checks, offline, that `pins/sophia.toml`, every Cargo
manifest and `Cargo.lock` agree on that one revision and that every copied
file still hashes to its pin. `cargo xtask audit-pins ABSOLUTE-SOPHIA-REPO`
re-derives the copied files and contract digests from a Sophia clone.

## Attended Lom, launcher and dock gates

The 9P Lom runners (see [Lom content](docs/lom-content.md)) moved
from Sophia (rule 13): the Lom GPU/content proof, the tty4 panel,
launcher and dock runners, their transcript verifiers, the workload budgets
and fixtures, and the `xtask dock` profile generator and verifier. See
[tools/probes/README.md](tools/probes/README.md). Every input is explicit:

    cargo xtask prepare-product-artifact lom|provlita|hagia REPO SIGNED-COMMIT NEW-DIR \
        --build-dir=/ABS [--nim-deps=/ABS --nim-deps-sha256=SHA]   # Hagia: reviewed deps required
    cargo xtask prepare-bemenu-artifact BEMENU-REPO SIGNED-COMMIT NEW-DIR
    SOPHIA_LOM_NATIVE_GATE_ARM=1 SOPHIA_SOURCE=/abs/sophia SOPHIA_GATE_BUILD_DIR=/abs/build \
    CARGO_HOME=/abs/private-cargo-home SOPHIA_INTEGRATION_XTASK=/abs/xtask \
    SOPHIA_SESSION_PREFLIGHT=/abs/active-session-preflight \
    SOPHIA_LOM_ARTIFACT=DIR SOPHIA_LOM_COMMIT=REV SOPHIA_LOM_SHA256=SHA SOPHIA_LOM_CONFIG_SHA256=SHA \
    SOPHIA_HAGIA_ARTIFACT=DIR SOPHIA_HAGIA_COMMIT=REV SOPHIA_HAGIA_SHA256=SHA \
    [SOPHIA_BEMENU_ARTIFACT=DIR SOPHIA_BEMENU_COMMIT=REV SOPHIA_BEMENU_SHA256=SHA] \
    [SOPHIA_PROVLITA_ARTIFACT=DIR SOPHIA_PROVLITA_COMMIT=REV SOPHIA_PROVLITA_SHA256=SHA \
     SOPHIA_PROVLITA_CONFIG_SHA256=SHA] \
        tools/run_current_lom_panel_gate_tty4.sh [launcher|dock]

These runs are manual (tty4, real hardware). They also require
`SOPHIA_GATE_BUILD_DIR` (an absolute private directory outside every source
tree; every build and the staged Sophia tree live there) and, per product,
`SOPHIA_<PRODUCT>_SHA256` (and `_CONFIG_SHA256` for Lom and Provlita), the
digests printed by the prepare command. `SOPHIA_SOURCE` must be a clean
checkout whose HEAD is exactly the signed revision in `pins/sophia.toml`; the
gate stages that revision's exact tree (`git archive`, tree hash proven) and
reads Sophia only from there. Source authorization (the signed revision) and
artifact binding (the expected digests) are separate checks. Rust products
build `--offline --locked`, so run `cargo fetch --locked` in the product
repository first. Their verifier
self-tests are part of the offline gate (`crates/xtask/tests/verifier_self_tests.rs`).

## Provisioning

    sh tools/provision.sh --cargo-home ABSOLUTE-DIR [--source ABSOLUTE-SOPHIA-REPO] \
        [--generate-lockfile | --update-lockfile]

This fetches every dependency into an explicit private `CARGO_HOME` that must
lie outside this repository and the Sophia source (inside it, Cargo's upward
workspace search from the pinned checkout's vendored SDK reaches this
repository's workspace and skips those packages), seeded by copying the operator's `~/.cargo/registry` index, cache
and src (never credentials or config, never a link back), and accepts it
(`.provision/accepted`, binding the pin's url and rev, the `Cargo.lock`
sha256 and the `CARGO_HOME` path) only after `check-pins` passes. `.provision/provision.log` records
whether the registry index or git database changed and every crate that was
downloaded. The default fetches the pinned revision from its public URL;
`--source` redirects that URL to a local clone for the script's own cargo and
git children only. A private `CARGO_HOME` rather than `cargo vendor` is used
because `sophia-conformance` builds only from an exact git checkout.
Afterwards every command is offline:

    CARGO_HOME=/abs/private-cargo-home cargo <command> --offline --locked

## Gates

All with a private target outside this tree, two jobs, low priority and a hard
timeout:

    export CARGO_HOME=/abs/private-cargo-home CARGO_TARGET_DIR=/abs/private-target CARGO_BUILD_JOBS=2
    timeout -s KILL 600 nice -n 19 cargo run --offline --locked -p xtask -- check-pins
    timeout -s KILL 600 nice -n 19 cargo run --offline --locked -p xtask -- check-provision
    timeout -s KILL 3600 nice -n 19 cargo test --workspace --offline --locked
    timeout -s KILL 3600 nice -n 19 cargo clippy --workspace --all-targets --offline --locked -- -D warnings
    cargo fmt --check
    timeout -s KILL 600 nice -n 19 bash tools/check_lom_gpu_content_proof_verifiers.sh

The live Bemenu gate is ignored by default and fails closed on any missing or
mismatched input:

    CARGO_BUILD_JOBS=2 nice -n 19 cargo xtask prepare-bemenu-artifact SOURCE-REPO SIGNED-COMMIT NEW-OUTPUT-DIR
    SOPHIA_BEMENU_ARTIFACT=OUTPUT-DIR SOPHIA_BEMENU_SHA256=BINARY-SHA256 \
    SOPHIA_BEMENU_COMMIT=SIGNED-COMMIT CARGO_BUILD_JOBS=2 nice -n 19 \
    cargo test --offline --locked -p live-tests --test bemenu_files -- --ignored --nocapture
