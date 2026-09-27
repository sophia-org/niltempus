#!/usr/bin/env bash
# Provenance: moved from Sophia tools/run_current_hagia_policy_gate_tty4.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
set -euo pipefail

# Changes: Sophia is the explicit pinned checkout SOPHIA_SOURCE (never this
# repository), Hagia and Narthex are explicit checkouts (no sibling default),
# this repository is bound too, and Sophia's launcher receives absolute
# SOPHIA_BIN and SOPHIA_SESSION_PREFLIGHT (tools/lib/physical_runner.sh).
# Nothing is built here or in any checkout: the three binaries, Hagia's default
# profile and the pinned Sophia tree come from prepared physical inputs built
# from the signed trees in the private SOPHIA_GATE_BUILD_DIR, Hagia and Narthex
# from their REVIEWED dependency manifests (SOPHIA_HAGIA_NIM_DEPS[_SHA256],
# SOPHIA_NARTHEX_NIM_DEPS[_SHA256]); tools/lib/physical_inputs.sh.
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
# Archives bind this signed integration commit of this checkout.
export SOPHIA_INTEGRATION_COMMIT="$integration_commit" SOPHIA_INTEGRATION_SOURCE="$ROOT_DIR"
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
echo "Preparing exact physical-proof binaries before DRM takeover..."
echo "Sophia: $sophia_commit"
echo "Hagia:  $hagia_commit"
echo "Narthex: $narthex_commit"
nim_options="$(physical_inputs_nim_options hagia "$HAGIA_ROOT" &&
    physical_inputs_nim_options narthex "$NARTHEX_ROOT")" || exit 2
mapfile -t nim_options <<<"$nim_options"
physical_inputs_prepare --sophia-features=native-session "${nim_options[@]}" \
    --profile=hagia:examples/config/default.kdl
physical_inputs_bound "$integration_commit"
if [[ "${PI[SOPHIA_COMMIT]}" != "$sophia_commit" || "${PI[SOPHIA_HAGIA_COMMIT]}" != "$hagia_commit" \
    || "${PI[SOPHIA_NARTHEX_COMMIT]}" != "$narthex_commit" ]]; then
    echo "The prepared inputs are not the bound Sophia, Hagia and Narthex commits." >&2
    exit 1
fi
sophia_bin="${PI[SOPHIA_BIN]}"
hagia_bin="${PI[SOPHIA_HAGIA_BIN]}"
hagia_shell_bin="${PI[SOPHIA_NARTHEX_BIN]}"
# The launcher reads Sophia's retained files from the staged pinned tree.
SOPHIA_ROOT="${PI[SOPHIA_ROOT]}"
export SOPHIA_ROOT
# Hagia's canonical default profile, from Hagia's staged signed tree.
desktop_profile="${PI[SOPHIA_PROFILE_DIR]}/hagia/examples/config/default.kdl"
[[ -f "$desktop_profile" ]] || {
    echo "Hagia's canonical default profile is missing: $desktop_profile" >&2
    exit 1
}
"$hagia_bin" config check --config="$desktop_profile"
"$sophia_bin" config check --desktop-profile="$desktop_profile"

if [[ -n "$(git -C "$SOPHIA_SOURCE" status --short)" \
    || -n "$(git -C "$ROOT_DIR" status --short)" \
    || -n "$(git -C "$HAGIA_ROOT" status --short)" \
    || -n "$(git -C "$NARTHEX_ROOT" status --short)" \
    || "$(git -C "$SOPHIA_SOURCE" rev-parse HEAD)" != "$sophia_commit" \
    || "$(git -C "$ROOT_DIR" rev-parse HEAD)" != "$integration_commit" \
    || "$(git -C "$HAGIA_ROOT" rev-parse HEAD)" != "$hagia_commit" \
    || "$(git -C "$NARTHEX_ROOT" rev-parse HEAD)" != "$narthex_commit" ]]; then
    echo "Sophia, integration, Hagia, or Narthex source identity changed while the physical inputs were prepared." >&2
    exit 1
fi
git -C "$SOPHIA_SOURCE" verify-commit "$sophia_commit" >/dev/null 2>&1 || {
    echo "Sophia signature no longer verifies after the inputs were prepared." >&2
    exit 1
}
git -C "$HAGIA_ROOT" verify-commit "$hagia_commit" >/dev/null 2>&1 || {
    echo "Hagia signature no longer verifies after the inputs were prepared." >&2
    exit 1
}

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
# Sophia's launcher receives the prepared binary this run bound, absolute.
export SOPHIA_BIN="$sophia_bin"
exec "$ROOT_DIR/tools/start_sophia_hagia_policy_tty4.sh"
