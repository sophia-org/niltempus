#!/usr/bin/env bash
# Provenance: moved from Sophia tools/check_sophia_firefox_lifecycle_verifier.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE="$ROOT_DIR/tools/fixtures/physical_firefox_lifecycle_pass.log"
TEMP_FILE="$(mktemp)"
trap 'rm -f -- "$TEMP_FILE"' EXIT

"$ROOT_DIR/tools/verify_sophia_firefox_lifecycle_physical.sh" "$FIXTURE"
for pattern in 'checkpoint=after_normal_close' 'action=CloseFocused' 'checkpoint=after_forced_close' 'status=complete page_ready=true'; do
    grep -Fv "$pattern" "$FIXTURE" >"$TEMP_FILE"
    if "$ROOT_DIR/tools/verify_sophia_firefox_lifecycle_physical.sh" "$TEMP_FILE"; then
        echo "focused lifecycle verifier accepted missing evidence: $pattern" >&2
        exit 1
    fi
done
echo 'focused Firefox lifecycle verifier fixtures passed'
