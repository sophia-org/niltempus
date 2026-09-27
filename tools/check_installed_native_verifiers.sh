#!/usr/bin/env bash
# Provenance: moved from Sophia tools/check_installed_native_verifiers.sh at a6edbbcad02ad9e9bb4c790e7cfd714cf333d30b (original copy; source unchanged at the pin de776c68) (Sophia rule 13).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

checks=(
    tools/check_installed_login_cycle_verifier.sh
    tools/check_installed_xterm_verifier.sh
    tools/check_truecolor_verifier.sh
    tools/check_installed_fallback_verifier.sh
    tools/check_installed_native_chrome_verifier.sh
    tools/check_installed_session_lifecycle_verifier.sh
    tools/check_installed_watchdog_recovery.sh
    tools/check_installed_hagia_ledger.sh
)
for check in "${checks[@]}"; do
    "$check"
done

echo "installed native verifier fixtures passed"
