#!/usr/bin/env bash
# Provenance: moved from Sophia tools/live_session_milestone4_hardware_proof.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Changes: Sophia is the explicit pinned checkout SOPHIA_SOURCE (never this
# repository): built there and its atomic-scanout preflight read from it; the
# proof runs the absolute SOPHIA_BIN it built.
# shellcheck source=tools/lib/sophia_source.sh
source "$ROOT_DIR/tools/lib/sophia_source.sh"
sophia_source="$(sophia_source_repo)" || exit 2
EVIDENCE_DIR="${SOPHIA_M4_EVIDENCE_DIR:-${XDG_STATE_HOME:-${HOME}/.local/state}/sophia/milestone4}"
DISPLAY_NAME="${SOPHIA_M4_DISPLAY:-:184}"
RUNTIME_MSEC="${SOPHIA_M4_RUNTIME_MSEC:-6000}"
SOFTWARE_EVIDENCE="$EVIDENCE_DIR/software-xterm.log"
GPU_EVIDENCE="$EVIDENCE_DIR/gpu-vkcube.log"
KERNEL_BEFORE="$EVIDENCE_DIR/kernel-before.log"
KERNEL_AFTER="$EVIDENCE_DIR/kernel-after.log"
ENVIRONMENT_EVIDENCE="$EVIDENCE_DIR/environment.log"

if [[ ! -t 0 ]]; then
    echo "Run this proof interactively from a dedicated local text TTY." >&2
    exit 1
fi
if [[ -n "${DISPLAY:-}" || -n "${WAYLAND_DISPLAY:-}" ]]; then
    echo "A graphical display is active in this shell; use a dedicated text TTY." >&2
    exit 1
fi
command -v xterm >/dev/null || {
    echo "xterm is required for the Milestone 4 proof." >&2
    exit 1
}
command -v sudo >/dev/null || {
    echo "sudo is required to retain the AMDGPU validator record." >&2
    exit 1
}
VKCUBE="$(command -v vkcube || true)"
if [[ -z "$VKCUBE" ]]; then
    echo "vkcube is required for the Milestone 4 GPU proof." >&2
    exit 1
fi

mkdir -p "$EVIDENCE_DIR"
: >"$SOFTWARE_EVIDENCE"
: >"$GPU_EVIDENCE"
sudo -v
{
    uname -a
    printf 'vkcube=%s\n' "$VKCUBE"
    command -v lspci >/dev/null && lspci -nnk | grep -A3 -E 'VGA|3D|Display' || true
    command -v modinfo >/dev/null && modinfo amdgpu | grep -E '^(filename|version|srcversion|vermagic):' || true
} >"$ENVIRONMENT_EVIDENCE" 2>&1
sudo dmesg -T >"$KERNEL_BEFORE"

echo "Sophia Milestone 4 software + Vulkan hardware proof"
echo "This proof requires exclusive DRM/KMS ownership on the active TTY."
echo "Evidence: $EVIDENCE_DIR"

cargo build --quiet --release --offline --manifest-path "$sophia_source/Cargo.toml" \
    -p sophia-cli --features "atomic-scanout-live" --target-dir "$sophia_source/target"
"$sophia_source/tools/atomic_scanout_preflight.sh"
SOPHIA_BIN="$sophia_source/target/release/sophia"
export SOPHIA_BIN

SOPHIA_ATOMIC_SCANOUT_SKIP_PREFLIGHT=1 \
SOPHIA_LIVE_SESSION_SKIP_BUILD=1 \
SOPHIA_LIVE_SESSION_PERSISTENT_EVIDENCE="$SOFTWARE_EVIDENCE" \
SOPHIA_LIVE_SESSION_DISPLAY="$DISPLAY_NAME" \
SOPHIA_LIVE_SESSION_RUNTIME_MSEC="$RUNTIME_MSEC" \
    "$ROOT_DIR/tools/live_session_persistent_hardware_proof.sh" \
        --inject-surface-resize=800x600

set +e
(
    cd "$sophia_source"
    SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE=1 \
        "$SOPHIA_BIN" session run \
        --display="$DISPLAY_NAME" --native-scanout \
        --max-runtime-ms="$RUNTIME_MSEC" --secondary-terminal \
        --terminal-exec="$VKCUBE" \
        --m4-first-acquire-delay-ms=150 \
        --m4-reject-first-present
) 2>&1 | tee "$GPU_EVIDENCE"
proof_status="${PIPESTATUS[0]}"
if ! sudo dmesg -T >"$KERNEL_AFTER"; then
    echo "Failed to retain post-run kernel log in $KERNEL_AFTER" >&2
    proof_status=1
fi
set -e

if (( proof_status == 0 )); then
    "$ROOT_DIR/tools/verify_live_session_milestone4_evidence.sh" "$GPU_EVIDENCE"
fi

exit "$proof_status"
