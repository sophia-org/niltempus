#!/usr/bin/env bash
# Provenance: moved from Sophia tools/stop_sophia_native_session.sh at
# a6edbbcad02ad9e9bb4c790e7cfd714cf333d30b (pin pending: root's gated boundary head) (Sophia rule 13).
# Sophia's retained stop primitive takes the profile as an opaque label.
set -euo pipefail

: "${SOPHIA_ROOT:?SOPHIA_ROOT must name the Sophia tree}"
[[ "$SOPHIA_ROOT" == /* ]] || { echo "SOPHIA_ROOT must be absolute." >&2; exit 2; }
exec "$SOPHIA_ROOT/tools/stop_sophia_session.sh" native
