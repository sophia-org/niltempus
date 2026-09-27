#!/usr/bin/env bash
# Provenance: moved from Sophia tools/benchmark_sophia_terminal_tty3.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
set -euo pipefail

# Bounded Sophia terminal CPU-path throughput benchmark. Drives the
# SHM/software-Present composition path with an unmodified xterm scrollback
# workload (rather than the GPU DRI3 flip path exercised by the vkcube/glxgears
# benchmarks) and reports the CPU patch-batch, compose, and damage-driven
# repaint evidence.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
# Changes: Sophia is the caller's explicit pinned checkout SOPHIA_SOURCE and
# prebuilt absolute SOPHIA_BIN (no build here), whose bounded xterm probe stays in Sophia; Sophia's launcher also receives
# the absolute SOPHIA_SESSION_PREFLIGHT (tools/lib/physical_runner.sh).
# shellcheck source=tools/lib/physical_runner.sh
source "$ROOT_DIR/tools/lib/physical_runner.sh"
runner_inputs
runner_sophia_bin

export SOPHIA_TTY_PROFILE=standalone
export SOPHIA_STANDALONE_WORKLOAD=xterm
export SOPHIA_STANDALONE_APP_BIN="$SOPHIA_SOURCE/tools/probes/run_bounded_xterm.sh"
export SOPHIA_XTERM_DURATION_SECONDS="${SOPHIA_XTERM_DURATION_SECONDS:-20}"
export SOPHIA_XTERM_WIDTH="${SOPHIA_XTERM_WIDTH:-500}"
export SOPHIA_XTERM_HEIGHT="${SOPHIA_XTERM_HEIGHT:-500}"
export SOPHIA_XTERM_LINES="${SOPHIA_XTERM_LINES:-1}"
export SOPHIA_XTERM_INTERVAL_MSEC="${SOPHIA_XTERM_INTERVAL_MSEC:-16}"
[[ "$SOPHIA_XTERM_DURATION_SECONDS" =~ ^[1-9][0-9]*$ ]] || {
    echo "SOPHIA_XTERM_DURATION_SECONDS must be a positive integer." >&2
    exit 1
}
[[ "$SOPHIA_XTERM_WIDTH" =~ ^[1-9][0-9]*$
    && "$SOPHIA_XTERM_HEIGHT" =~ ^[1-9][0-9]*$ ]] || {
    echo "SOPHIA_XTERM_WIDTH and SOPHIA_XTERM_HEIGHT must be positive integers." >&2
    exit 1
}
[[ "$SOPHIA_XTERM_LINES" =~ ^[1-9][0-9]*$ ]] || {
    echo "SOPHIA_XTERM_LINES must be a positive integer." >&2
    exit 1
}
[[ "$SOPHIA_XTERM_INTERVAL_MSEC" =~ ^[1-9][0-9]*$
    && "$SOPHIA_XTERM_INTERVAL_MSEC" -le 1000 ]] || {
    echo "SOPHIA_XTERM_INTERVAL_MSEC must be an integer from 1 through 1000." >&2
    exit 1
}
# The inner workload self-bounds; the process-external deadline is only a safety
# net so a hung session still restores the TTY.
export SOPHIA_SESSION_WATCHDOG_SECONDS="${SOPHIA_SESSION_WATCHDOG_SECONDS:-$((SOPHIA_XTERM_DURATION_SECONDS + 10))}"
export SOPHIA_SESSION_VERBOSE_TRACE="${SOPHIA_SESSION_VERBOSE_TRACE:-false}"
unset SOPHIA_STANDALONE_FRAME_COUNT

printf '%s\n' \
    "Starting bounded Sophia xterm CPU-path benchmark (${SOPHIA_XTERM_DURATION_SECONDS} seconds, ${SOPHIA_XTERM_WIDTH}x${SOPHIA_XTERM_HEIGHT}, ${SOPHIA_XTERM_LINES} lines every ${SOPHIA_XTERM_INTERVAL_MSEC}ms)." \
    'First checking the software-Present path without taking over the TTY.' \
    'Confirm that a centered xterm shows continuous scrolling text.' \
    'The workload and Sophia exit automatically when the timer completes.' \
    "An independent ${SOPHIA_SESSION_WATCHDOG_SECONDS}-second deadline restores the TTY if Sophia locks." \
    'Ctrl+Alt+Backspace remains available for emergency recovery.'

"$SOPHIA_BIN" x-authority-xterm-input-smoke
"$ROOT_DIR/tools/session/start_sophia_tty3.sh" "$@"
"$ROOT_DIR/tools/report_sophia_terminal_performance.sh"
