#!/usr/bin/env bash
# Provenance: moved from Sophia tools/live_session_persistent_hardware_proof.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Nothing is built here: the caller hands over the absolute SOPHIA_BIN it bound
# from its prepared physical inputs (tools/lib/physical_inputs.sh), and the
# atomic-scanout preflight is read from the pinned Sophia tree SOPHIA_ROOT.
# shellcheck source=tools/lib/sophia_source.sh
source "$ROOT_DIR/tools/lib/sophia_source.sh"
# shellcheck source=tools/lib/physical_inputs.sh
source "$ROOT_DIR/tools/lib/physical_inputs.sh"
sophia_root="$(sophia_pinned_root)" || exit 2
EVIDENCE_FILE="${SOPHIA_LIVE_SESSION_PERSISTENT_EVIDENCE:-/tmp/sophia-live-session-persistent.log}"
DISPLAY_NAME="${SOPHIA_LIVE_SESSION_DISPLAY:-:181}"
RUNTIME_MSEC="${SOPHIA_LIVE_SESSION_RUNTIME_MSEC:-5000}"
SKIP_PREFLIGHT="${SOPHIA_ATOMIC_SCANOUT_SKIP_PREFLIGHT:-0}"
VERIFY_MODE="${SOPHIA_LIVE_SESSION_VERIFY_MODE:-generic}"

case "$VERIFY_MODE" in
    generic|caller) ;;
    *)
        echo "SOPHIA_LIVE_SESSION_VERIFY_MODE must be generic or caller" >&2
        exit 2
        ;;
esac

mkdir -p "$(dirname "$EVIDENCE_FILE")"
: > "$EVIDENCE_FILE"

echo "Sophia persistent live-session hardware proof"
echo "This proof requires exclusive DRM/KMS ownership on the active TTY."
echo "Evidence: $EVIDENCE_FILE"

# The prepared release binary (built before any DRM/KMS ownership, so build
# time is never presented as a blank native frame; persistent rendering
# evidence measures optimized code).
[[ "${SOPHIA_BIN:-}" == /* && -x "${SOPHIA_BIN:-}" ]] || {
    echo "SOPHIA_BIN must name an absolute Sophia binary from the prepared physical inputs (no default)." >&2
    exit 2
}

input_proof_args=(--inject-text=sophia)
for arg in "$@"; do
    case "$arg" in
        --inject-text=*|--expect-physical-text=*)
            input_proof_args=()
            ;;
    esac
done

if [[ "$SKIP_PREFLIGHT" != "1" ]]; then
    physical_inputs_preflight "$SOPHIA_BIN" "$sophia_root" "$EVIDENCE_FILE.preflight.log"
fi

set +e
(
    cd "$sophia_root"
    SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE=1 \
        "$SOPHIA_BIN" \
        session run --display="$DISPLAY_NAME" --native-scanout \
        --max-runtime-ms="$RUNTIME_MSEC" "${input_proof_args[@]}" "$@"
) 2>&1 | tee "$EVIDENCE_FILE"
proof_status="${PIPESTATUS[0]}"
set -e

if [[ "$proof_status" -eq 0 && "$VERIFY_MODE" == generic ]]; then
    "$sophia_root/tools/verify_live_session_persistent_evidence.sh" "$EVIDENCE_FILE"
fi

exit "$proof_status"
