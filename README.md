# Sophia desktop integration

Whole-stack tests that combine Sophia with named desktop clients, and the
build recipes for the client artifacts they run. Sophia itself stays shell
and WM agnostic (Sophia `AGENTS.md` rule 13, `docs/style-guide.md`); this
repository owns what that rule moves out.

The first slice is Bemenu: signed-revision artifact preparation and the live
native launcher gate over Sophia's production 9P file export.

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

Moved from Sophia (rule 13): the Lom GPU/content proof, the tty4 panel,
launcher and dock runners, their transcript verifiers, the workload budgets
and fixtures, and the `xtask dock` profile generator and verifier. See
[tools/probes/README.md](tools/probes/README.md). Every input is explicit:

    cargo xtask prepare-product-artifact lom|provlita|hagia REPO SIGNED-COMMIT NEW-DIR
    cargo xtask prepare-bemenu-artifact BEMENU-REPO SIGNED-COMMIT NEW-DIR
    SOPHIA_LOM_NATIVE_GATE_ARM=1 SOPHIA_SOURCE=/abs/sophia \
    SOPHIA_LOM_ARTIFACT=DIR SOPHIA_LOM_COMMIT=REV \
    SOPHIA_HAGIA_ARTIFACT=DIR SOPHIA_HAGIA_COMMIT=REV \
    [SOPHIA_BEMENU_ARTIFACT=DIR SOPHIA_BEMENU_COMMIT=REV] \
    [SOPHIA_PROVLITA_ARTIFACT=DIR SOPHIA_PROVLITA_COMMIT=REV] \
        tools/run_current_lom_panel_gate_tty4.sh [launcher|dock]

These runs are manual (tty4, real hardware). The Sophia checkout's shared
session runner and native launcher catalog must match
`pins/sophia-shared.sha256`. Rust products build `--offline --locked`, so run
`cargo fetch --locked` in the product repository first. Their verifier
self-tests are part of the offline gate (`crates/xtask/tests/verifier_self_tests.rs`).

## Provisioning

    sh tools/provision.sh [--source ABSOLUTE-SOPHIA-REPO] [--generate-lockfile]

This vendors every dependency into `.provision/` (ignored) and accepts the
result only after `check-pins` passes. The default fetches the pinned revision
from its public URL; `--source` redirects that URL to a local clone for the
script's own cargo and git children only. Afterwards every command is offline:

    cargo --config .provision/cargo-config.toml <command> --offline --locked

## Gates

    CARGO_BUILD_JOBS=2 nice -n 19 cargo --config .provision/cargo-config.toml \
        test --workspace --offline --locked
    CARGO_BUILD_JOBS=2 nice -n 19 cargo --config .provision/cargo-config.toml \
        clippy --workspace --all-targets --offline --locked -- -D warnings

The live Bemenu gate is ignored by default and fails closed on any missing or
mismatched input:

    CARGO_BUILD_JOBS=2 nice -n 19 cargo xtask prepare-bemenu-artifact SOURCE-REPO SIGNED-COMMIT NEW-OUTPUT-DIR
    SOPHIA_BEMENU_ARTIFACT=OUTPUT-DIR SOPHIA_BEMENU_SHA256=BINARY-SHA256 \
    SOPHIA_BEMENU_COMMIT=SIGNED-COMMIT CARGO_BUILD_JOBS=2 nice -n 19 \
    cargo --config .provision/cargo-config.toml test --offline --locked \
        -p live-tests --test bemenu_files -- --ignored --nocapture
