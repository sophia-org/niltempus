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
. "$ROOT_DIR/tools/lib/physical_inputs.sh"
SEAT="${SOPHIA_LOM_GPU_SEAT:-seat0}"
RENDER_NODE="${SOPHIA_LOM_GPU_RENDER_NODE:-/dev/dri/renderD128}"
EVIDENCE_DIR="${SOPHIA_LOM_GPU_EVIDENCE_DIR:-$ROOT_DIR/.artifacts/lom-gpu-content-proof/$(date -u +%Y%m%dT%H%M%SZ)}"
LOG="$EVIDENCE_DIR/proof.log"

[[ "${SOPHIA_LOM_GPU_PROOF_ARM:-0}" == 1 ]] || {
    echo "Set SOPHIA_LOM_GPU_PROOF_ARM=1 to use the admitted render node." >&2
    exit 2
}
for name in SOPHIA_SOURCE SOPHIA_GATE_BUILD_DIR SOPHIA_LOM_ARTIFACT SOPHIA_LOM_COMMIT \
    SOPHIA_LOM_SHA256 SOPHIA_LOM_CONFIG_SHA256 SOPHIA_INTEGRATION_XTASK; do
    [[ -n "${!name:-}" ]] || { echo "$name is required (no default)" >&2; exit 2; }
done
[[ -c "$RENDER_NODE" ]] || { echo "Render node is not a character device: $RENDER_NODE" >&2; exit 2; }
[[ -z "$(git -C "$ROOT_DIR" status --short)" ]] || { echo "Integration source must be clean" >&2; exit 2; }
git -C "$ROOT_DIR" verify-commit HEAD >/dev/null
check_sophia_source "$SOPHIA_SOURCE"
check_build_dir "$SOPHIA_GATE_BUILD_DIR"

# Seam D: Sophia's generic `shell-gpu-content-proof`, armed by
# SOPHIA_SHELL_GPU_PROOF_ARM=1, reports per-render
# `sophia_shell_gpu_content_render schema=1` records and a final
# `sophia_shell_gpu_content_proof schema=1` record. The Lom expectations below
# (surface, edge, outcomes, end, input, pixels) belong to this repository.

[[ ! -e "$EVIDENCE_DIR" ]] || { echo "Evidence directory already exists; refusing to overwrite it" >&2; exit 2; }
mkdir -p "$(dirname "$EVIDENCE_DIR")"
mkdir -m 700 "$EVIDENCE_DIR"
BUILD_DIR="$(realpath -- "$SOPHIA_GATE_BUILD_DIR")"
if [[ -n "${SOPHIA_PHYSICAL_INPUTS:-}" ]]; then
    physical_inputs_use "$SOPHIA_PHYSICAL_INPUTS" "${SOPHIA_PHYSICAL_INPUTS_SHA256:-}"
else
    physical_inputs_prepare --sophia-features=native-session
fi
SOPHIA_BIN="${PI[SOPHIA_BIN]}"
SOPHIA_TREE="${PI[SOPHIA_ROOT]}"
[[ "${PI[SOPHIA_INTEGRATION_COMMIT]}" == "$(git -C "$ROOT_DIR" rev-parse HEAD)" ]] || {
    echo "Prepared inputs belong to another integration commit" >&2; exit 2;
}
LOM_BIN="$EVIDENCE_DIR/lom"
LOM_CONFIG="$EVIDENCE_DIR/lom-config.kdl"
load_artifact lom "$SOPHIA_LOM_ARTIFACT" "$SOPHIA_LOM_COMMIT" "$SOPHIA_LOM_SHA256" "$LOM_BIN" \
    "$SOPHIA_LOM_CONFIG_SHA256" "$LOM_CONFIG"
# Verify all prepared files and the source tree before execution.
physical_inputs_verify_exported
{
    printf 'integration_commit=%s\n' "$(git -C "$ROOT_DIR" rev-parse HEAD)"
    printf 'sophia_commit=%s\n' "$(pinned_sophia_rev)"
    printf 'physical_inputs_manifest_sha256=%s\n' "$SOPHIA_PHYSICAL_INPUTS_SHA256"
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
# Lom's expectations: one 256x24 top-edge panel on a 256x64 output, the full
# surface raster pattern, presented then renderer-failed, the client exits on
# its own, discrete input granted. full-surface-raster proves only that the
# pattern crossed the content path; GPU execution rests on the protected
# grant plus Lom's own lom_gpu_admission evidence (see the verifier).
env -u DISPLAY -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \
    -u SOPHIA_SHELL_SOCKET -u SOPHIA_SHELL_9P_SOCKET SOPHIA_SHELL_GPU_PROOF_ARM=1 \
    "$SOPHIA_BIN" shell-gpu-content-proof --transport=9p2000.L \
    "--client=$LOM_BIN" --client-arg=--serve "--config=$LOM_CONFIG" \
    "--seat=$SEAT" "--render-node=$RENDER_NODE" --output=256x64 --surface=256x24 --edge=top \
    --pixels=full-surface-raster --outcomes=presented,renderer-failed --end=client-exits \
    --discrete-input=granted \
    --timeout-ms=30000 2>&1 | tee "$LOG"
# Tree check: after the proof, before its result is trusted.
physical_inputs_verify_exported
"$ROOT_DIR/tools/verify_lom_gpu_content_hardware_proof.sh" "$LOG" | tee "$EVIDENCE_DIR/verification.log"
