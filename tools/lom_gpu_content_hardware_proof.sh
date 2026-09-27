#!/usr/bin/env bash
# Provenance: moved from Sophia tools/lom_gpu_content_hardware_proof.sh at 9fcaec782ce4fe9978568c0466ee17a78b3d4571 (Sophia rule 13).
# Changes: every source is explicit (no /home default and no sibling checkout):
# the Sophia checkout under test is SOPHIA_SOURCE, and Lom is a prepared,
# signed-revision artifact (cargo xtask prepare-product-artifact lom ...).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=tools/lib/artifacts.sh
. "$ROOT_DIR/tools/lib/artifacts.sh"
SEAT="${SOPHIA_LOM_GPU_SEAT:-seat0}"
RENDER_NODE="${SOPHIA_LOM_GPU_RENDER_NODE:-/dev/dri/renderD128}"
EVIDENCE_DIR="${SOPHIA_LOM_GPU_EVIDENCE_DIR:-$ROOT_DIR/.artifacts/lom-gpu-content-proof/$(date -u +%Y%m%dT%H%M%SZ)}"
LOG="$EVIDENCE_DIR/proof.log"

[[ "${SOPHIA_LOM_GPU_PROOF_ARM:-0}" == 1 ]] || {
    echo "Set SOPHIA_LOM_GPU_PROOF_ARM=1 to use the admitted render node." >&2
    exit 2
}
for name in SOPHIA_SOURCE SOPHIA_LOM_ARTIFACT SOPHIA_LOM_COMMIT; do
    [[ -n "${!name:-}" ]] || { echo "$name is required (no default)" >&2; exit 2; }
done
[[ -c "$RENDER_NODE" ]] || { echo "Render node is not a character device: $RENDER_NODE" >&2; exit 2; }
[[ -z "$(git -C "$ROOT_DIR" status --short)" ]] || { echo "Integration source must be clean" >&2; exit 2; }
git -C "$ROOT_DIR" verify-commit HEAD >/dev/null
check_sophia_source "$SOPHIA_SOURCE"

# WAITING ON SEAM D. Sophia's proof command is Lom-named at the pinned
# revision (`sophia-shell-gpu-content-hardware-proof`, armed by
# SOPHIA_LOM_GPU_PROOF_ARM). The proposed generic replacement is
# `sophia shell-gpu-content-proof` with the SOPHIA_SHELL_GPU_* environment.
# Until the director relays that API's approval this script refuses before
# building anything; the invocation below is written against the PROPOSED
# names and is not yet bound.
SEAM_D_APPROVED=false
[[ "$SEAM_D_APPROVED" == true ]] || {
    echo "waiting on seam D: Sophia's generic shell-gpu-content-proof command is not approved yet" >&2
    exit 3
}

[[ ! -e "$EVIDENCE_DIR" ]] || { echo "Evidence directory already exists; refusing to overwrite it" >&2; exit 2; }
mkdir -p "$(dirname "$EVIDENCE_DIR")"
mkdir -m 700 "$EVIDENCE_DIR"
LOM_BIN="$EVIDENCE_DIR/lom"
LOM_CONFIG="$EVIDENCE_DIR/lom-config.kdl"
load_artifact lom "$SOPHIA_LOM_ARTIFACT" "$SOPHIA_LOM_COMMIT" "$LOM_BIN" "$LOM_CONFIG"
CARGO_BUILD_JOBS=2 nice -n 19 cargo build --locked --offline --release -p sophia-cli \
    --features native-session --manifest-path "$SOPHIA_SOURCE/Cargo.toml"
SOPHIA_BIN="$SOPHIA_SOURCE/target/release/sophia"
{
    printf 'integration_commit=%s\n' "$(git -C "$ROOT_DIR" rev-parse HEAD)"
    printf 'sophia_commit=%s\n' "$(git -C "$SOPHIA_SOURCE" rev-parse HEAD)"
    printf 'sophia_binary_sha256=%s\n' "$(sha256sum "$SOPHIA_BIN" | cut -d' ' -f1)"
    printf 'lom_commit=%s\n' "$SOPHIA_LOM_COMMIT"
    printf 'lom_binary_sha256=%s\n' "$(sha256sum "$LOM_BIN" | cut -d' ' -f1)"
    printf 'lom_config_sha256=%s\n' "$(sha256sum "$LOM_CONFIG" | cut -d' ' -f1)"
    printf 'seat=%s\nrender_node=%s\n' "$SEAT" "$RENDER_NODE"
} > "$EVIDENCE_DIR/identity.manifest"

echo "Evidence: $EVIDENCE_DIR"
# SEAM D (proposed names, not yet bound): command and arming variable.
env -u DISPLAY -u WAYLAND_DISPLAY -u WAYLAND_SOCKET SOPHIA_SHELL_GPU_PROOF_ARM=1 \
    "$SOPHIA_BIN" shell-gpu-content-proof \
    "--client=$LOM_BIN" "--config=$LOM_CONFIG" "--seat=$SEAT" \
    "--render-node=$RENDER_NODE" 2>&1 | tee "$LOG"
"$ROOT_DIR/tools/verify_lom_gpu_content_hardware_proof.sh" "$LOG" | tee "$EVIDENCE_DIR/verification.log"
