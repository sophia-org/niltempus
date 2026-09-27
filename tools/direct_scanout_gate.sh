#!/usr/bin/env bash
# Provenance: moved from Sophia tools/direct_scanout_gate.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
# Changes: runs this repository's port of the direct-scanout physical gate
# (crates/xtask/src/direct_scanout_gate.rs) against the explicit pinned
# SOPHIA_SOURCE, with absolute SOPHIA_SESSION_PREFLIGHT and
# SOPHIA_INTEGRATION_XTASK; the TTY is SOPHIA_SESSION_TTY or this terminal.
# Nothing is built here or in any checkout: the release binary and the pinned
# tree come from prepared physical inputs (tools/lib/physical_inputs.sh) in the
# private SOPHIA_GATE_BUILD_DIR, and the prebuilt recipe tool runs the gate.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=tools/lib/physical_runner.sh
source "$ROOT_DIR/tools/lib/physical_runner.sh"
runner_inputs
integration_commit="$(runner_integration_commit)"
physical_inputs_prepare --sophia-features=atomic-scanout-live
physical_inputs_bound "$integration_commit"
cd "$ROOT_DIR"
exec "$SOPHIA_INTEGRATION_XTASK" direct-scanout-gate "$@"
