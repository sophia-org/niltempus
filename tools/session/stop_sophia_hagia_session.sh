#!/usr/bin/env bash
# Stop the managed (Hagia) desktop session: Sophia's retained stop primitive
# with the same opaque label run_desktop_session.sh gives the wrapper. (Sophia
# printed a hint for this script at a6edbbcad02ad9e9bb4c790e7cfd714cf333d30b
# but never had it; at the pin de776c68 its hint names the primitive instead.)
set -euo pipefail

: "${SOPHIA_ROOT:?SOPHIA_ROOT must name the Sophia tree}"
[[ "$SOPHIA_ROOT" == /* ]] || { echo "SOPHIA_ROOT must be absolute." >&2; exit 2; }
exec "$SOPHIA_ROOT/tools/stop_sophia_session.sh" managed
