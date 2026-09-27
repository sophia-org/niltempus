#!/usr/bin/env bash
# Provenance: moved from Sophia tools/check_firefox_m10_primary_kitty_probe.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROBE="$ROOT_DIR/tools/fixtures/firefox_m10_primary_kitty_probe.sh"
TEMP_DIR="$(mktemp -d)"
trap 'rm -rf -- "$TEMP_DIR"' EXIT

printf 'wrong\nsophia-firefox-primary\n' |
    env SOPHIA_FIREFOX_M10_KITTY_PROBE_DIR="$TEMP_DIR" "$PROBE" >"$TEMP_DIR/probe.out"

[[ -e "$TEMP_DIR/checkpoint-primary-received" ]]
[[ "$(grep -Fc 'stale or incomplete' "$TEMP_DIR/probe.out")" == 1 ]]
grep -Fq 'sophia-kitty-primary' "$TEMP_DIR/probe.out"
! grep -Fq 'CLIPBOARD' "$TEMP_DIR/probe.out"
echo 'focused Firefox PRIMARY Kitty coordinator passed'
