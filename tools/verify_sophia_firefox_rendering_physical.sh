#!/usr/bin/env bash
# Provenance: moved from Sophia tools/verify_sophia_firefox_rendering_physical.sh at a6edbbcad02ad9e9bb4c790e7cfd714cf333d30b (original copy; source unchanged at the pin de776c68) (Sophia rule 13).
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STATE_HOME="${XDG_STATE_HOME:-$HOME/.local/state}"
SESSION_LOG="${1:-${SOPHIA_HAGIA_LOG_DIR:-$STATE_HOME/sophia/managed-session}/session.log}"
[[ -s "$SESSION_LOG" ]] || { echo "Missing Firefox rendering log: $SESSION_LOG" >&2; exit 1; }
awk -f "$ROOT_DIR/tools/lib/verify_firefox_rendering.awk" "$SESSION_LOG"
echo "Firefox changing-content/native-retirement canary verified: $SESSION_LOG"
