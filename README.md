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

## Provisioning

    sh tools/provision.sh [--source ABSOLUTE-SOPHIA-REPO] [--generate-lockfile]

This vendors every dependency into `.provision/` (ignored) and accepts the
result only after `check-pins` passes. The default fetches the pinned revision
from its public URL; `--source` redirects that URL to a local clone for the
script's own cargo and git children only. Afterwards every command is offline:

    cargo --config .provision/cargo-config.toml <command> --offline --locked

## Gates

    nice -n 19 cargo --config .provision/cargo-config.toml \
        test --workspace --offline --locked
    nice -n 19 cargo --config .provision/cargo-config.toml \
        clippy --workspace --all-targets --offline --locked -- -D warnings

The live Bemenu gate is ignored by default and fails closed on any missing or
mismatched input:

    cargo xtask prepare-bemenu-artifact SOURCE-REPO SIGNED-COMMIT NEW-OUTPUT-DIR
    SOPHIA_BEMENU_ARTIFACT=OUTPUT-DIR SOPHIA_BEMENU_SHA256=BINARY-SHA256 \
    SOPHIA_BEMENU_COMMIT=SIGNED-COMMIT nice -n 19 \
    cargo --config .provision/cargo-config.toml test --offline --locked \
        -p live-tests --test bemenu_files -- --ignored --nocapture
