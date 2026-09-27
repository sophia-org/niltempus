#!/usr/bin/env bash
# Provenance: moved from Sophia tools/run_current_critical_path_tty4.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
set -euo pipefail

# Changes: Sophia is the explicit pinned checkout SOPHIA_SOURCE, Hagia (and,
# for the policy gate, Narthex) are explicit checkouts with no sibling default,
# this repository is bound too, and the TTY comes from SOPHIA_SESSION_TTY or the
# controlling terminal (tools/lib/physical_runner.sh).
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=tools/lib/physical_runner.sh
source "$ROOT_DIR/tools/lib/physical_runner.sh"
HAGIA_ROOT="${SOPHIA_HAGIA_ROOT:-}"
TTY_REQUIRED="${SOPHIA_CRITICAL_PATH_TTY:-/dev/tty4}"
REMOVABLE_CONNECTOR="${SOPHIA_CRITICAL_PATH_REMOVABLE_CONNECTOR:-DP-3}"

if [[ ! -t 0 ]]; then
    echo "Switch to tty4 with Ctrl+Alt+F4, log in, and run:" >&2
    echo "  cd $ROOT_DIR && tools/run_current_critical_path_tty4.sh" >&2
    exit 1
fi
runner_tty "$TTY_REQUIRED"
runner_inputs
if [[ "$HAGIA_ROOT" != /* || ! -e "$HAGIA_ROOT/.git" ]]; then
    echo "Hagia checkout not found at ${HAGIA_ROOT:-<unset>}; set SOPHIA_HAGIA_ROOT." >&2
    exit 1
fi

sophia_commit="$(git -C "$SOPHIA_SOURCE" rev-parse HEAD)"
integration_commit="$(git -C "$ROOT_DIR" rev-parse HEAD)"
hagia_commit="$(git -C "$HAGIA_ROOT" rev-parse HEAD)"
mapfile -t three_head_topology < <(
    printf '%s\n' DP-1 DP-2 "$REMOVABLE_CONNECTOR" | sort -u
)
(( ${#three_head_topology[@]} == 3 )) || {
    echo "The removable connector must differ from DP-1 and DP-2." >&2
    exit 1
}

verify_identity() {
    local repo commit name
    for name in Sophia Integration Hagia; do
        case "$name" in
            Sophia) repo="$SOPHIA_SOURCE"; commit="$sophia_commit" ;;
            Integration) repo="$ROOT_DIR"; commit="$integration_commit" ;;
            Hagia) repo="$HAGIA_ROOT"; commit="$hagia_commit" ;;
        esac
        [[ -z "$(git -C "$repo" status --porcelain --untracked-files=all)" ]] || {
            echo "$name worktree changed; stopping before another physical gate." >&2
            exit 1
        }
        [[ "$(git -C "$repo" rev-parse HEAD)" == "$commit" ]] || {
            echo "$name HEAD changed; stopping before another physical gate." >&2
            exit 1
        }
        git -C "$repo" verify-commit "$commit" >/dev/null 2>&1 || {
            echo "$name HEAD lacks a valid cryptographic signature." >&2
            exit 1
        }
        # Every phase binds the same signed commit, independently of publication.
    done
}

connected_connectors() {
    local status connector
    for status in /sys/class/drm/card*-*/status; do
        [[ -r "$status" && "$(<"$status")" == connected ]] || continue
        connector="${status%/status}"
        basename "$connector" | sed -E 's/^card[0-9]+-//'
    done | sort
}

wait_for_topology() {
    local instruction="$1"
    shift
    local -a expected=("$@") observed=()
    local reply
    while true; do
        echo
        echo "$instruction"
        echo "Expected: ${expected[*]}"
        echo "Press Enter after the cable state is stable, or type q to stop."
        IFS= read -r reply </dev/tty
        [[ "$reply" != q ]] || exit 1
        mapfile -t observed < <(connected_connectors)
        if [[ "${observed[*]}" == "${expected[*]}" ]]; then
            echo "Topology accepted: ${observed[*]}"
            return 0
        fi
        echo "Topology is not ready; observed: ${observed[*]:-none}" >&2
    done
}

verify_identity
echo "Sophia/Hagia critical-path physical proof"
echo "Sophia: $sophia_commit"
echo "Hagia:  $hagia_commit"
echo "The script stops at the first refusal or failed visual check."

wait_for_topology \
    "Disconnect $REMOVABLE_CONNECTOR for the two-head mirror gate." \
    DP-1 DP-2
verify_identity
"$ROOT_DIR/tools/run_mirror_group_gate_tty4.sh"

wait_for_topology \
    "Reconnect $REMOVABLE_CONNECTOR for the centered three-head mixed gate." \
    "${three_head_topology[@]}"
verify_identity
SOPHIA_MIXED_MIRROR_PRIMARY=DP-1 \
SOPHIA_MIXED_MIRROR_MEMBER="$REMOVABLE_CONNECTOR" \
SOPHIA_MIXED_EXTENDED=DP-2 \
    "$ROOT_DIR/tools/run_mixed_output_gate_tty4.sh" \
    --optimize-for=center-unscaled

wait_for_topology \
    "Disconnect $REMOVABLE_CONNECTOR again for the two-head Hagia/broker gate." \
    DP-1 DP-2
verify_identity
SOPHIA_HAGIA_ROOT="$HAGIA_ROOT" \
    "$ROOT_DIR/tools/run_current_hagia_policy_gate_tty4.sh"

verify_identity
"$ROOT_DIR/tools/verify_mirror_group_physical_archive.sh"
"$ROOT_DIR/tools/verify_mixed_output_physical_archive.sh"
SOPHIA_HAGIA_ROOT="$HAGIA_ROOT" \
    "$ROOT_DIR/tools/verify_hagia_policy_physical_archive.sh"

echo
echo "All three current critical-path physical gates passed and their archives verify."
echo "Reconnect $REMOVABLE_CONNECTOR before returning to the graphical session."
