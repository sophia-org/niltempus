#!/usr/bin/env bash
# Provenance: moved from Sophia tools/native_egl_vkcube_mixed_smoke.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Changes: Sophia is the explicit pinned checkout SOPHIA_SOURCE (never this
# repository): built there, and its atomic-scanout preflight and generic
# native-EGL mixed-evidence verifier (retained in Sophia) read from it.
# shellcheck source=tools/lib/sophia_source.sh
source "$ROOT_DIR/tools/lib/sophia_source.sh"
sophia_source="$(sophia_source_repo)" || exit 2
DISPLAY_NAME="${SOPHIA_M4_DISPLAY:-:184}"
RUNTIME_MSEC="${SOPHIA_M4_RUNTIME_MSEC:-6000}"
EVIDENCE_FILE="${SOPHIA_M4_NATIVE_EGL_EVIDENCE:-${XDG_STATE_HOME:-${HOME}/.local/state}/sophia/milestone4/native-egl-mixed.log}"

if [[ ! -t 0 ]]; then
    echo "Run this diagnostic interactively from a dedicated local text TTY." >&2
    exit 1
fi
if [[ -n "${DISPLAY:-}" || -n "${WAYLAND_DISPLAY:-}" ]]; then
    echo "A graphical display is active in this shell; use a dedicated text TTY." >&2
    exit 1
fi
command -v xterm >/dev/null || {
    echo "xterm is required for the native EGL mixed diagnostic." >&2
    exit 1
}
command -v vkcube >/dev/null || {
    echo "vkcube is required for the native EGL mixed diagnostic." >&2
    exit 1
}

cargo build --quiet --release --offline --manifest-path "$sophia_source/Cargo.toml" \
    -p sophia-cli --features "atomic-scanout-live" --target-dir "$sophia_source/target"
"$sophia_source/tools/atomic_scanout_preflight.sh"

mkdir -p "$(dirname "$EVIDENCE_FILE")"
set +e
SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE=1 \
    "$sophia_source/target/release/sophia" native-egl-vkcube-mixed-smoke \
        --display="$DISPLAY_NAME" --max-runtime-ms="$RUNTIME_MSEC" \
    2>&1 | tee "$EVIDENCE_FILE"
smoke_status="${PIPESTATUS[0]}"
set -e
if (( smoke_status == 0 )); then
    "$sophia_source/tools/verify_native_egl_mixed_evidence.sh" "$EVIDENCE_FILE"
fi
exit "$smoke_status"
