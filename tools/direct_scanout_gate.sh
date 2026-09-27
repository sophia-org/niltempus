#!/usr/bin/env bash
# Provenance: moved from Sophia tools/direct_scanout_gate.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
# Changes: runs this repository's port of the direct-scanout physical gate
# (crates/xtask/src/direct_scanout_gate.rs) against the explicit pinned
# SOPHIA_SOURCE, with absolute SOPHIA_SESSION_PREFLIGHT and
# SOPHIA_INTEGRATION_XTASK; the TTY is SOPHIA_SESSION_TTY or this terminal.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
exec cargo --quiet --offline --locked xtask direct-scanout-gate "$@"
