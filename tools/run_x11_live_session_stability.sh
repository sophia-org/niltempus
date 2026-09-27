#!/usr/bin/env bash
# Provenance: moved from Sophia tools/run_x11_live_session_stability.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Changes: Sophia is the explicit pinned checkout SOPHIA_SOURCE (never this
# repository). Nothing is built here or there: the release binary and the exact
# pinned tree come from prepared physical inputs built from the signed tree in
# the private SOPHIA_GATE_BUILD_DIR (tools/lib/physical_inputs.sh; this
# repository is bound, SOPHIA_INTEGRATION_XTASK prepares them), and the
# atomic-scanout preflight is read from that staged tree. --diagnostic runs the
# same prepared release binary under gdb (no separate debug-info build).
# shellcheck source=tools/lib/sophia_source.sh
source "$ROOT_DIR/tools/lib/sophia_source.sh"
# shellcheck source=tools/lib/physical_runner.sh
source "$ROOT_DIR/tools/lib/physical_runner.sh"
sophia_source="$(sophia_source_repo)" || exit 2
STATE_DIR="${XDG_STATE_HOME:-${HOME}/.local/state}/sophia"
EVIDENCE_DIR="$STATE_DIR/x11-live-session-stability"
MODE=normal
RUNS=1

usage() {
    echo "usage: $0 [--diagnostic|--trace|--core] [--runs COUNT]" >&2
}

while (( $# > 0 )); do
    case "$1" in
        --diagnostic) MODE=diagnostic ;;
        --trace) MODE=trace ;;
        --core) MODE=core ;;
        --runs)
            shift
            (( $# > 0 )) || { usage; exit 2; }
            RUNS="$1"
            ;;
        --runs=*) RUNS="${1#--runs=}" ;;
        -h|--help) usage; exit 0 ;;
        *) usage; exit 2 ;;
    esac
    shift
done

if [[ ! "$RUNS" =~ ^[0-9]+$ ]] || (( RUNS < 1 || RUNS > 20 )); then
    echo "--runs must be an integer from 1 through 20." >&2
    exit 2
fi
if [[ "$MODE" != normal && "$RUNS" != 1 ]]; then
    echo "Diagnostic modes cannot be combined with --runs." >&2
    exit 2
fi
if [[ ! -t 0 ]] || [[ -n "${DISPLAY:-}" || -n "${WAYLAND_DISPLAY:-}" ]]; then
    echo "Run this from a dedicated local text TTY." >&2
    exit 1
fi
for process in river niri sway Hyprland kwin_wayland Xorg; do
    if pgrep -x "$process" >/dev/null 2>&1; then
        echo "Refusing to take DRM ownership while $process is active." >&2
        exit 1
    fi
done
if [[ "$MODE" == diagnostic ]] && ! command -v gdb >/dev/null 2>&1; then
    echo "Diagnostic mode requires gdb." >&2
    exit 1
fi

mkdir -p "$EVIDENCE_DIR"
chmod 700 "$STATE_DIR" "$EVIDENCE_DIR"
integration_commit="$(runner_integration_commit)"
physical_inputs_prepare --sophia-features=native-session
physical_inputs_bound "$integration_commit"
[[ "${PI[SOPHIA_COMMIT]}" == "$(git -C "$sophia_source" rev-parse HEAD)" ]] || {
    echo "The prepared inputs are not the Sophia checkout's pinned commit." >&2
    exit 1
}
SOPHIA_ROOT="${PI[SOPHIA_ROOT]}"
export SOPHIA_ROOT
cd "$SOPHIA_ROOT"
physical_inputs_preflight "${PI[SOPHIA_BIN]}" "$SOPHIA_ROOT" "$EVIDENCE_DIR/preflight.log"

session=(
    "${PI[SOPHIA_BIN]}"
    session
    run
    --display=:181
    --native-scanout
    --max-runtime-ms=30000
    --inject-text=sophia
    --exit-after-input-proof
)

run_session() {
    local evidence="$1"
    local diagnostic="${2:-0}"
    local diagnostic_env=()
    if [[ "$diagnostic" == 1 ]]; then
        diagnostic_env=(SOPHIA_LIVE_SESSION_DIAGNOSTIC=1 MALLOC_CHECK_=3 MESA_DEBUG=1)
    fi
    env SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE=1 "${diagnostic_env[@]}" \
        timeout --foreground 45s "${session[@]}" >"$evidence" 2>&1
    "$SOPHIA_ROOT/tools/verify_live_session_persistent_evidence.sh" "$evidence"
}

case "$MODE" in
    diagnostic)
        evidence="$EVIDENCE_DIR/x11-live-session-diagnostic.log"
        gdb_log="$evidence.gdb.log"
        set +e
        env SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE=1 \
            SOPHIA_LIVE_SESSION_DIAGNOSTIC=1 MALLOC_CHECK_=3 MESA_DEBUG=1 \
            timeout --foreground 45s gdb --batch --return-child-result \
                -ex 'set pagination off' \
                -ex 'set confirm off' \
                -ex 'set follow-fork-mode parent' \
                -ex 'set detach-on-fork on' \
                -ex run \
                -ex 'thread apply all bt full' \
                --args "${session[@]}" >"$gdb_log" 2>&1
        status=$?
        set -e
        install -m 600 "$gdb_log" "$evidence"
        if (( status != 0 )); then
            echo "X11 live-session diagnostic failed; GDB evidence: $gdb_log" >&2
            exit "$status"
        fi
        "$SOPHIA_ROOT/tools/verify_live_session_persistent_evidence.sh" "$gdb_log"
        ;;
    trace)
        evidence="$EVIDENCE_DIR/x11-live-session-trace.log"
        run_session "$evidence" 1
        ;;
    core)
        evidence="$EVIDENCE_DIR/x11-live-session-core.log"
        core_file="$evidence.core"
        rm -f "$EVIDENCE_DIR"/core "$EVIDENCE_DIR"/core.* "$core_file"
        set +e
        (
            cd "$EVIDENCE_DIR"
            ulimit -c 262144
            env SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE=1 \
                timeout --foreground 45s "${session[@]}"
        ) >"$evidence" 2>&1
        status=$?
        set -e
        captured_core="$(find "$EVIDENCE_DIR" -maxdepth 1 -type f -name 'core*' -print -quit)"
        if [[ -n "$captured_core" ]]; then
            install -m 600 "$captured_core" "$core_file"
        fi
        if (( status != 0 )); then
            if [[ -f "$core_file" ]]; then
                echo "X11 live-session failed; core evidence: $core_file" >&2
            else
                echo "X11 live-session failed without a captured core." >&2
            fi
            exit "$status"
        fi
        "$SOPHIA_ROOT/tools/verify_live_session_persistent_evidence.sh" "$evidence"
        ;;
    normal)
        for (( run = 1; run <= RUNS; run += 1 )); do
            evidence="$EVIDENCE_DIR/x11-live-session-run-$run.log"
            echo "X11 live-session stability run $run/$RUNS"
            run_session "$evidence"
        done
        echo "X11 live-session stability proof passed: $RUNS/$RUNS runs"
        ;;
esac
