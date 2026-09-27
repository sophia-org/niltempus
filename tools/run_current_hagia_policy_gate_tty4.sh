#!/usr/bin/env bash
# Provenance: moved from Sophia tools/run_current_hagia_policy_gate_tty4.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
set -euo pipefail

# Changes: Sophia is the explicit pinned checkout SOPHIA_SOURCE (never this
# repository), Hagia and Narthex are explicit checkouts (no sibling default),
# this repository is bound too, and Sophia's launcher receives absolute
# SOPHIA_BIN and SOPHIA_SESSION_PREFLIGHT (tools/lib/physical_runner.sh).
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=tools/lib/proof_checkout.sh
source "$ROOT_DIR/tools/lib/proof_checkout.sh"
# shellcheck source=tools/lib/physical_runner.sh
source "$ROOT_DIR/tools/lib/physical_runner.sh"
HAGIA_ROOT="${SOPHIA_HAGIA_ROOT:-}"
NARTHEX_ROOT="${SOPHIA_NARTHEX_ROOT:-}"

if [[ ! -t 0 ]]; then
    echo "Switch to tty4 with Ctrl+Alt+F4, log in, and run:" >&2
    echo "  $ROOT_DIR/tools/run_current_hagia_policy_gate_tty4.sh" >&2
    exit 1
fi
runner_tty /dev/tty4
runner_inputs
if ! proof_checkout_root "$HAGIA_ROOT"; then
    echo "Hagia checkout not found at $HAGIA_ROOT" >&2
    echo "Set SOPHIA_HAGIA_ROOT to its checkout path." >&2
    exit 1
fi
if ! proof_checkout_root "$NARTHEX_ROOT"; then
    echo "Narthex checkout not found at $NARTHEX_ROOT" >&2
    echo "Set SOPHIA_NARTHEX_ROOT to its checkout path." >&2
    exit 1
fi
if [[ -n "$(git -C "$SOPHIA_SOURCE" status --short)" ]]; then
    echo "Sophia worktree must be clean before the physical proof." >&2
    exit 1
fi
integration_commit="$(runner_integration_commit)"
if [[ -n "$(git -C "$HAGIA_ROOT" status --short)" ]]; then
    echo "Hagia worktree must be clean before the physical proof." >&2
    exit 1
fi
if [[ -n "$(git -C "$NARTHEX_ROOT" status --short)" ]]; then
    echo "Narthex worktree must be clean before the physical proof." >&2
    exit 1
fi

sophia_commit="$(git -C "$SOPHIA_SOURCE" rev-parse HEAD)"
hagia_commit="$(git -C "$HAGIA_ROOT" rev-parse HEAD)"
narthex_commit="$(git -C "$NARTHEX_ROOT" rev-parse HEAD)"
for repo_and_commit in "$SOPHIA_SOURCE:$sophia_commit" "$HAGIA_ROOT:$hagia_commit" "$NARTHEX_ROOT:$narthex_commit"; do
    repo="${repo_and_commit%:*}"
    commit="${repo_and_commit##*:}"
    git -C "$repo" verify-commit "$commit" >/dev/null 2>&1 || {
        echo "Physical-proof HEAD lacks a valid signature: $repo" >&2
        echo "  Run tools/check_proof_preconditions.sh first to see all three." >&2
        exit 1
    }
    # The archive verifies this exact signed identity, not its publication state.
done
hagia_bin="${TMPDIR:-/tmp}/hagia-policy-${hagia_commit:0:12}"
hagia_shell_bin="${TMPDIR:-/tmp}/narthex-${narthex_commit:0:12}"
hagia_nimcache="${TMPDIR:-/tmp}/hagia-policy-nimcache-${hagia_commit:0:12}"
hagia_shell_nimcache="${TMPDIR:-/tmp}/narthex-nimcache-${narthex_commit:0:12}"

# These proofs build, hash and run the host executable at
# SOPHIA_SOURCE/target/release/sophia. A cross-compilation target would put the
# build elsewhere and leave that path bound to whatever was there before.
if [[ -n "${CARGO_BUILD_TARGET:-}" ]]; then
    echo "Unset CARGO_BUILD_TARGET: this physical proof builds the host Sophia binary it binds." >&2
    exit 1
fi
echo "Building exact physical-proof binaries before DRM takeover..."
echo "Sophia: $sophia_commit"
echo "Hagia:  $hagia_commit"
echo "Narthex: $narthex_commit"
(
    cd "$HAGIA_ROOT"
    nim c -d:release --path:src --nimcache:"$hagia_nimcache" \
        -o:"$hagia_bin" src/hagia.nim
)
(
    cd "$NARTHEX_ROOT"
    nim c -d:release --path:src --nimcache:"$hagia_shell_nimcache" \
        -o:"$hagia_shell_bin" src/narthex.nim
)
(
    cd "$SOPHIA_SOURCE"
    # The executable hashed and run below, whatever CARGO_TARGET_DIR says.
    cargo build --quiet --release --offline -p sophia-cli \
        --features native-session --target-dir "$SOPHIA_SOURCE/target"
)
desktop_profile="$HAGIA_ROOT/examples/config/default.kdl"
[[ -f "$desktop_profile" ]] || {
    echo "Hagia's canonical default profile is missing: $desktop_profile" >&2
    exit 1
}
"$hagia_bin" config check --config="$desktop_profile"
"$SOPHIA_SOURCE/target/release/sophia" config check \
    --desktop-profile="$desktop_profile"

if [[ -n "$(git -C "$SOPHIA_SOURCE" status --short)" \
    || -n "$(git -C "$ROOT_DIR" status --short)" \
    || -n "$(git -C "$HAGIA_ROOT" status --short)" \
    || -n "$(git -C "$NARTHEX_ROOT" status --short)" \
    || "$(git -C "$SOPHIA_SOURCE" rev-parse HEAD)" != "$sophia_commit" \
    || "$(git -C "$ROOT_DIR" rev-parse HEAD)" != "$integration_commit" \
    || "$(git -C "$HAGIA_ROOT" rev-parse HEAD)" != "$hagia_commit" \
    || "$(git -C "$NARTHEX_ROOT" rev-parse HEAD)" != "$narthex_commit" ]]; then
    echo "Sophia, integration, Hagia, or Narthex source identity changed during the physical-proof build." >&2
    exit 1
fi
git -C "$SOPHIA_SOURCE" verify-commit "$sophia_commit" >/dev/null 2>&1 || {
    echo "Sophia signature no longer verifies after the build." >&2
    exit 1
}
git -C "$HAGIA_ROOT" verify-commit "$hagia_commit" >/dev/null 2>&1 || {
    echo "Hagia signature no longer verifies after the build." >&2
    exit 1
}

sophia_bin="$SOPHIA_SOURCE/target/release/sophia"
sophia_sha256="$(sha256sum "$sophia_bin" | awk '{ print $1 }')"
hagia_sha256="$(sha256sum "$hagia_bin" | awk '{ print $1 }')"
hagia_shell_sha256="$(sha256sum "$hagia_shell_bin" | awk '{ print $1 }')"
echo "Sophia binary: $sophia_sha256"
echo "Hagia binary:  $hagia_sha256"
echo "Hagia Shell:   $hagia_shell_sha256"

export SOPHIA_HAGIA_BIN="$hagia_bin"
export SOPHIA_HAGIA_SHELL_BIN="$hagia_shell_bin"
export SOPHIA_DESKTOP_PROFILE="$desktop_profile"
# No profile identity is exported here. This gate's session runs with
# `--no-config`, which loads the compiled profile, so exporting a mode and the
# digest of a file on disk made Sophia print an identity for a profile it never
# loaded. The profile above is still checked by both `config check` calls, which
# is what it is for. `tools/run_current_hagia_native_gate_tty4.sh` binds a
# profile it actually passes to the session and keeps its identity.
export SOPHIA_HAGIA_ROOT="$HAGIA_ROOT"
export SOPHIA_HAGIA_PHYSICAL_SOURCE_COMMIT="$sophia_commit"
export SOPHIA_HAGIA_PHYSICAL_HAGIA_COMMIT="$hagia_commit"
export SOPHIA_HAGIA_PHYSICAL_SOPHIA_SHA256="$sophia_sha256"
export SOPHIA_HAGIA_PHYSICAL_HAGIA_SHA256="$hagia_sha256"
export SOPHIA_HAGIA_PHYSICAL_NARTHEX_SHA256="$hagia_shell_sha256"
export SOPHIA_HAGIA_PHYSICAL_NARTHEX_COMMIT="$narthex_commit"
export SOPHIA_LIVE_SESSION_SKIP_BUILD=1
# Sophia's launcher receives the binary this run built and bound, absolute.
export SOPHIA_BIN="$sophia_bin"
exec "$ROOT_DIR/tools/start_sophia_hagia_policy_tty4.sh"
