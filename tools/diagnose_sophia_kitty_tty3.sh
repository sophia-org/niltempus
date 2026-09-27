#!/usr/bin/env bash
# Provenance: moved from Sophia tools/diagnose_sophia_kitty_tty3.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Changes: launches through this repository's tools/session launcher (which
# requires absolute SOPHIA_ROOT, SOPHIA_BIN and SOPHIA_SESSION_PREFLIGHT), and
# reads Sophia's retained native-composition pixel verifier from SOPHIA_ROOT.
: "${SOPHIA_ROOT:?SOPHIA_ROOT must name the absolute pinned Sophia tree}"
[[ "$SOPHIA_ROOT" == /* ]] || { echo "SOPHIA_ROOT must be absolute." >&2; exit 2; }
LAUNCH_LOG=/tmp/sophia-kitty-tty3-launch.log

export SOPHIA_NATIVE_COMPOSITION_PIXEL_TRACE=1
set +e
"$ROOT_DIR/tools/session/start_sophia_kitty_tty3.sh" "$@"
session_status=$?
set -e

if "$SOPHIA_ROOT/tools/verify_sophia_native_composition_pixels.sh" "$LAUNCH_LOG"; then
    evidence_status=0
else
    evidence_status=$?
fi
if (( session_status != 0 )); then
    exit "$session_status"
fi
exit "$evidence_status"
