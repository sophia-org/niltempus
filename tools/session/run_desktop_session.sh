#!/usr/bin/env bash
# The named-recipe half of Sophia's former tools/run_sophia_session.sh at
# a6edbbcad02ad9e9bb4c790e7cfd714cf333d30b (pin pending: root's gated boundary
# head) (Sophia rule 13): application discovery (prepare-inputs), recipe proof
# staging (stage-proofs), the profile's `session run` arguments
# (prepare-arguments), the recipe-only environment, and the profile banners.
# Sophia's retained generic wrapper then owns everything product-neutral:
# controls, host preflight (check-host), input guard, keyd, TTY takeover and
# restore, watchdog, bus, launch acceptance and recovery.
#
# Sophia's generic launcher contract (root, pin pending):
#   SOPHIA_TTY_PROFILE=<opaque label> run_sophia_session.sh -- session run <args...>
# with a prebuilt absolute SOPHIA_BIN, exactly one --input-seat=VALUE or
# --input-devices=VALUE among the session arguments (the wrapper hands it to
# the session and the input guard unchanged), and the product environment
# supplied by the caller (the wrapper's own prepare-environment receives only
# the TTY). Product names never reach Sophia: the profile maps to a label.
#
# Required, absolute, no defaults:
#   SOPHIA_ROOT                   the Sophia tree (staged pinned tree or release)
#   SOPHIA_BIN                    the prebuilt `sophia` executable
#   SOPHIA_SESSION_PREFLIGHT      the host checker (active-session-preflight)
#   SOPHIA_INTEGRATION_XTASK      this repository's prebuilt xtask (recipes)
#   SOPHIA_TTY_PROFILE            hagia | native | kitty | standalone
set -euo pipefail
umask 077

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
for name in SOPHIA_ROOT SOPHIA_BIN SOPHIA_SESSION_PREFLIGHT SOPHIA_INTEGRATION_XTASK; do
    value="${!name:-}"
    [[ "$value" == /* ]] || { echo "$name must be an absolute path (no default)." >&2; exit 2; }
done
for name in SOPHIA_BIN SOPHIA_SESSION_PREFLIGHT SOPHIA_INTEGRATION_XTASK; do
    [[ -f "${!name}" && -x "${!name}" ]] || { echo "$name must name an executable file." >&2; exit 2; }
done
[[ -d "$SOPHIA_ROOT" && -x "$SOPHIA_ROOT/tools/run_sophia_session.sh" ]] || {
    echo "SOPHIA_ROOT does not hold Sophia's retained session wrapper." >&2
    exit 2
}
SESSION_PROFILE="${SOPHIA_TTY_PROFILE:-}"
case "$SESSION_PROFILE" in
    hagia|native|kitty|standalone) ;;
    *) echo "SOPHIA_TTY_PROFILE must be hagia, native, kitty or standalone." >&2; exit 2 ;;
esac
export SOPHIA_BIN SOPHIA_SESSION_PREFLIGHT

# The opaque label Sophia's wrapper and stop primitive see for this profile.
session_label() {
    case "$1" in
        hagia) echo managed ;;
        native) echo native ;;
        kitty) echo terminal ;;
        standalone) echo standalone ;;
    esac
}
SESSION_LABEL="$(session_label "$SESSION_PROFILE")"

# Load one NUL vector from the recipe tool without evaluating its text.
recipe_vector=()
load_recipe() {
    local expected="$1" verb="$2" file
    shift 2
    file="$(mktemp "${TMPDIR:-/tmp}/sophia-desktop-recipe.XXXXXX")" || return 1
    if ! timeout --kill-after=2s 15s "$SOPHIA_INTEGRATION_XTASK" session-recipe "$verb" "$@" >"$file"; then
        rm -f -- "$file"
        echo "The recipe tool refused $verb." >&2
        return 1
    fi
    mapfile -d '' -t recipe_vector <"$file"
    rm -f -- "$file"
    [[ "${recipe_vector[0]:-}" == "$expected" ]] || {
        echo "The recipe tool does not support $verb." >&2
        return 1
    }
}

load_recipe 'sophia_session_inputs schema=1 status=prepared' prepare-inputs \
    "--profile=$SESSION_PROFILE" "--root=$ROOT_DIR" -- "$@"
[[ "${#recipe_vector[@]}" == 7 ]] || { echo "Incomplete session inputs." >&2; exit 1; }
terminal_bin="${recipe_vector[1]}"
terminal_kind="${recipe_vector[2]}"
hagia_browser_bin="${recipe_vector[3]}"
standalone_bin="${recipe_vector[4]}"
SOPHIA_HAGIA_BIN="${recipe_vector[5]}"
session_benchmark="${recipe_vector[6]}"

# Recipe state is this adapter's own private directory; Sophia's wrapper keeps
# its generic state directory separately.
runtime_root="${XDG_RUNTIME_DIR:-/tmp}"
STATE_DIR="$runtime_root/sophia-desktop-${SESSION_PROFILE}-${UID}"
mkdir -p "$STATE_DIR"
chmod 700 "$STATE_DIR"
firefox_m10_probe_dir=""
cleanup_recipe_state() {
    [[ -z "$firefox_m10_probe_dir" ]] || rm -rf -- "$firefox_m10_probe_dir"
}
trap cleanup_recipe_state EXIT

load_recipe 'sophia_session_proofs schema=1 status=prepared' stage-proofs \
    "--profile=$SESSION_PROFILE" "--root=$ROOT_DIR" "--sophia-root=$SOPHIA_ROOT" \
    "--state-dir=$STATE_DIR" "--standalone=$standalone_bin" -- "$@"
[[ "${#recipe_vector[@]}" == 3 ]] || { echo "Incomplete proof staging." >&2; exit 1; }
firefox_m10_probe_dir="${recipe_vector[1]}"
firefox_m10_profile_dir="${recipe_vector[2]}"

prepared_arguments="$STATE_DIR/session-arguments.bin"
if ! timeout --kill-after=2s 15s "$SOPHIA_INTEGRATION_XTASK" session-recipe prepare-arguments \
    "--profile=$SESSION_PROFILE" "--root=$ROOT_DIR" "--state-dir=$STATE_DIR" \
    "--binary=$SOPHIA_BIN" "--terminal=$terminal_bin" \
    "--terminal-kind=${terminal_kind:-}" "--browser=$hagia_browser_bin" \
    "--standalone=$standalone_bin" "--wm=$SOPHIA_HAGIA_BIN" \
    "--firefox-profile=$firefox_m10_profile_dir" -- "$@" >"$prepared_arguments"; then
    echo "The recipe tool refused session argument preparation." >&2
    exit 1
fi
chmod 600 "$prepared_arguments"
mapfile -d '' -t prepared_vector <"$prepared_arguments"
rm -f "$prepared_arguments"
if [[ "${prepared_vector[0]:-}" != 'sophia_session_arguments schema=1 status=prepared' ]]; then
    echo "The recipe tool does not support session argument preparation." >&2
    exit 1
fi
session_args=("${prepared_vector[@]:1}")

load_recipe 'sophia_desktop_recipe_environment schema=1 status=prepared' prepare-environment \
    "--firefox-probe=$firefox_m10_probe_dir" -- "$@"
recipe_environment=("${recipe_vector[@]:1}")

# Exactly one input selector reaches Sophia's wrapper, inside the arguments.
input_selectors=0
for argument in "${session_args[@]}"; do
    case "$argument" in
        --input-seat=*|--input-devices=*) input_selectors=$((input_selectors + 1)) ;;
    esac
done
[[ "$input_selectors" == 1 ]] || {
    echo "The session arguments must carry exactly one --input-seat or --input-devices (found $input_selectors)." >&2
    exit 1
}
# Workload benchmark identities are this adapter's records (Sophia keeps no
# product record channel). Readers look for them in the adapter log, which is
# explicit (SOPHIA_DESKTOP_ADAPTER_LOG) or the per-profile default below.
ADAPTER_LOG="${SOPHIA_DESKTOP_ADAPTER_LOG:-${XDG_STATE_HOME:-$HOME/.local/state}/sophia/desktop-session/${SESSION_PROFILE}-adapter.log}"
[[ "$ADAPTER_LOG" == /* ]] || { echo "SOPHIA_DESKTOP_ADAPTER_LOG must be absolute." >&2; exit 2; }
mkdir -p "$(dirname "$ADAPTER_LOG")"
chmod 700 "$(dirname "$ADAPTER_LOG")"
printf 'sophia_desktop_adapter schema=1 status=starting profile=%s label=%s started_at_utc=%s\n' \
    "$SESSION_PROFILE" "$SESSION_LABEL" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"$ADAPTER_LOG"
[[ -z "$session_benchmark" ]] || printf '%s\n' "$session_benchmark" >>"$ADAPTER_LOG"
chmod 600 "$ADAPTER_LOG"

if [[ "$SESSION_PROFILE" == standalone ]]; then
    echo "Starting Sophia's standalone single-application proof."
    echo "No terminal, window manager, or status bar will run."
    echo "Quit the application to end the session; Ctrl+Alt+Backspace is the"
    echo "emergency path and is recorded as one."
elif [[ "$SESSION_PROFILE" == native ]]; then
    echo "Starting Sophia's session-lifecycle proof without a window manager."
    echo "Exit the terminal to end the session; Ctrl+Alt+Backspace is the"
    echo "emergency path and is recorded as one."
elif [[ "$SESSION_PROFILE" == hagia ]]; then
    echo "Starting Sophia with Hagia's native policy."
    echo "Use Super+Enter for Kitty or Ctrl+Alt+Delete to log out."
else
    echo "Starting the supported Kitty-only Sophia input session."
    echo "A policy client and Super+Enter are intentionally disabled for this input gate."
    echo "Exit Kitty normally to return to the text console."
fi
echo "The outside control plane may run tools/session/stop_sophia_${SESSION_PROFILE}_session.sh."

status=0
env ${recipe_environment[@]+"${recipe_environment[@]}"} SOPHIA_TTY_PROFILE="$SESSION_LABEL" \
    "$SOPHIA_ROOT/tools/run_sophia_session.sh" -- "${session_args[@]}" || status=$?
exit "$status"
