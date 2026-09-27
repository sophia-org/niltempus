#!/usr/bin/env bash
# Provenance: moved from Sophia tools/start_sophia_hagia_policy_tty4.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${SOPHIA_INSTALL_PREFIX:-/opt/sophia}"

export SOPHIA_TTY_PROFILE=hagia-policy
export SOPHIA_TTY_NUMBER=4
export SOPHIA_HAGIA_PHYSICAL_ARM=1
export SOPHIA_HAGIA_PHYSICAL_SEAT="${SOPHIA_HAGIA_PHYSICAL_SEAT:-seat0}"
export SOPHIA_HAGIA_BIN="${SOPHIA_HAGIA_BIN:-$PREFIX/current/target/release/hagia}"

exec "$ROOT_DIR/tools/session/start_sophia_tty3.sh" "$@"
