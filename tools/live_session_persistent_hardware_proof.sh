#!/usr/bin/env bash
# Provenance: moved from Sophia tools/live_session_persistent_hardware_proof.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Sophia is the explicit pinned checkout (SOPHIA_SOURCE): built there unless a
# caller already built and bound the absolute SOPHIA_BIN (SKIP_BUILD=1), and
# its atomic-scanout preflight is read from there.
# shellcheck source=tools/lib/sophia_source.sh
source "$ROOT_DIR/tools/lib/sophia_source.sh"
sophia_source="$(sophia_source_repo)" || exit 2
EVIDENCE_FILE="${SOPHIA_LIVE_SESSION_PERSISTENT_EVIDENCE:-/tmp/sophia-live-session-persistent.log}"
DISPLAY_NAME="${SOPHIA_LIVE_SESSION_DISPLAY:-:181}"
RUNTIME_MSEC="${SOPHIA_LIVE_SESSION_RUNTIME_MSEC:-5000}"
SKIP_PREFLIGHT="${SOPHIA_ATOMIC_SCANOUT_SKIP_PREFLIGHT:-0}"
SKIP_BUILD="${SOPHIA_LIVE_SESSION_SKIP_BUILD:-0}"
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

# Compile before taking DRM/KMS ownership so build time is never presented as
# a blank native frame. Persistent rendering evidence must measure optimized
# code; the debug CPU compositor is intentionally not a performance target.
if [[ "$SKIP_BUILD" != "1" ]]; then
    cargo build --quiet --release --offline --manifest-path "$sophia_source/Cargo.toml" -p sophia-cli \
        --features "atomic-scanout-live" --target-dir "$sophia_source/target"
    SOPHIA_BIN="$sophia_source/target/release/sophia"
fi
[[ "${SOPHIA_BIN:-}" == /* && -x "${SOPHIA_BIN:-}" ]] || {
    echo "SOPHIA_BIN must name an absolute Sophia binary (set it, or let this proof build one)." >&2
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
    "$sophia_source/tools/atomic_scanout_preflight.sh"
fi

set +e
(
    cd "$sophia_source"
    SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE=1 \
        "$SOPHIA_BIN" \
        session run --display="$DISPLAY_NAME" --native-scanout \
        --max-runtime-ms="$RUNTIME_MSEC" "${input_proof_args[@]}" "$@"
) 2>&1 | tee "$EVIDENCE_FILE"
proof_status="${PIPESTATUS[0]}"
set -e

if [[ "$proof_status" -eq 0 && "$VERIFY_MODE" == generic ]]; then
    "$ROOT_DIR/tools/verify_live_session_persistent_evidence.sh" "$EVIDENCE_FILE"
fi

exit "$proof_status"
