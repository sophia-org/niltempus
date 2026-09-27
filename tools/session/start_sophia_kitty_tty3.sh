#!/usr/bin/env bash
# Provenance: moved from Sophia tools/start_sophia_kitty_tty3.sh at
# a6edbbcad02ad9e9bb4c790e7cfd714cf333d30b (pin pending: root's gated boundary head) (Sophia rule 13).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export SOPHIA_TTY_PROFILE=kitty
exec "$ROOT_DIR/tools/session/start_sophia_tty3.sh" "$@"
