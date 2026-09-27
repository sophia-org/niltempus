#!/usr/bin/env bash
# Provenance: moved from Sophia tools/lom_gpu_content_hardware_proof.sh at 9fcaec782ce4fe9978568c0466ee17a78b3d4571 (Sophia rule 13).
# Changes: every input is explicit (no /home default, no sibling checkout, no
# build inside a source tree). Sophia is the exact pinned tree staged from the
# SOPHIA_SOURCE checkout; Lom is a prepared artifact bound to operator-supplied
# commit and SHA-256 values; all builds go to SOPHIA_GATE_BUILD_DIR.
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
for name in SOPHIA_SOURCE SOPHIA_GATE_BUILD_DIR SOPHIA_LOM_ARTIFACT SOPHIA_LOM_COMMIT \
    SOPHIA_LOM_SHA256 SOPHIA_LOM_CONFIG_SHA256; do
    [[ -n "${!name:-}" ]] || { echo "$name is required (no default)" >&2; exit 2; }
done
[[ -c "$RENDER_NODE" ]] || { echo "Render node is not a character device: $RENDER_NODE" >&2; exit 2; }
[[ -z "$(git -C "$ROOT_DIR" status --short)" ]] || { echo "Integration source must be clean" >&2; exit 2; }
git -C "$ROOT_DIR" verify-commit HEAD >/dev/null
check_sophia_source "$SOPHIA_SOURCE"
check_build_dir "$SOPHIA_GATE_BUILD_DIR"

# WAITING ON SEAM D. At the pinned Sophia revision the proof command is
# Lom-named (`sophia-shell-gpu-content-hardware-proof`). The approved generic
# command is `sophia shell-gpu-content-proof`, armed by
# SOPHIA_SHELL_GPU_PROOF_ARM=1, reporting per-render
# `sophia_shell_gpu_content_render schema=1` and a final
# `sophia_shell_gpu_content_proof schema=1` record. It is bound only once the
# seams commit lands and pins/sophia.toml moves to it; until then this script
# refuses before building anything. The Lom expectations below (surface,
# edge, outcomes, end, input) belong to this repository, not to Sophia.
SEAM_D_BOUND=false
[[ "$SEAM_D_BOUND" == true ]] || {
    echo "waiting on seam D: the pinned Sophia revision has no generic shell-gpu-content-proof command" >&2
    exit 3
}

[[ ! -e "$EVIDENCE_DIR" ]] || { echo "Evidence directory already exists; refusing to overwrite it" >&2; exit 2; }
mkdir -p "$(dirname "$EVIDENCE_DIR")"
mkdir -m 700 "$EVIDENCE_DIR"
BUILD_DIR="$(realpath -- "$SOPHIA_GATE_BUILD_DIR")"
SOPHIA_TREE="$BUILD_DIR/sophia-tree"
SOPHIA_TARGET="$BUILD_DIR/sophia-target"
stage_sophia_tree "$SOPHIA_SOURCE" "$SOPHIA_TREE"
LOM_BIN="$EVIDENCE_DIR/lom"
LOM_CONFIG="$EVIDENCE_DIR/lom-config.kdl"
load_artifact lom "$SOPHIA_LOM_ARTIFACT" "$SOPHIA_LOM_COMMIT" "$SOPHIA_LOM_SHA256" "$LOM_BIN" \
    "$SOPHIA_LOM_CONFIG_SHA256" "$LOM_CONFIG"
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR="$SOPHIA_TARGET" nice -n 19 cargo build --locked --offline --release \
    -p sophia-cli --features native-session --manifest-path "$SOPHIA_TREE/Cargo.toml"
SOPHIA_BIN="$SOPHIA_TARGET/release/sophia"
{
    printf 'integration_commit=%s\n' "$(git -C "$ROOT_DIR" rev-parse HEAD)"
    printf 'sophia_commit=%s\n' "$(pinned_sophia_rev)"
    printf 'sophia_binary_sha256=%s\n' "$(sha256sum "$SOPHIA_BIN" | cut -d' ' -f1)"
    printf 'lom_commit=%s\n' "$SOPHIA_LOM_COMMIT"
    printf 'lom_binary_sha256=%s\n' "$(sha256sum "$LOM_BIN" | cut -d' ' -f1)"
    printf 'lom_config_sha256=%s\n' "$(sha256sum "$LOM_CONFIG" | cut -d' ' -f1)"
    printf 'seat=%s\nrender_node=%s\n' "$SEAT" "$RENDER_NODE"
} > "$EVIDENCE_DIR/identity.manifest"

echo "Evidence: $EVIDENCE_DIR"
# SOPHIA_SHELL_GPU_EXPECTED_DEVICE ("MAJ:MIN@PCI" or "none", for example
# 226:128@0000:03:00.0) is supplied by the operator when wanted; this script
# never derives or defaults it, and it reaches Sophia through the environment.
# SEAM D (approved names; bound when the seams commit is pinned). Lom's
# expectations: one 256x24 top-edge panel on a 256x64 output, presented then
# renderer-failed, the client exits on its own, discrete input granted.
env -u DISPLAY -u WAYLAND_DISPLAY -u WAYLAND_SOCKET SOPHIA_SHELL_GPU_PROOF_ARM=1 \
    "$SOPHIA_BIN" shell-gpu-content-proof \
    "--client=$LOM_BIN" --client-arg=--serve "--config=$LOM_CONFIG" \
    "--seat=$SEAT" "--render-node=$RENDER_NODE" --output=256x64 --surface=256x24 --edge=top \
    --outcomes=presented,renderer-failed --end=client-exits --discrete-input=granted \
    --timeout-ms=30000 2>&1 | tee "$LOG"
"$ROOT_DIR/tools/verify_lom_gpu_content_hardware_proof.sh" "$LOG" | tee "$EVIDENCE_DIR/verification.log"
