#!/usr/bin/env bash
# Provenance: moved from Sophia tools/run_keyboard_independence_gate_tty4.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
# Binds a clean, signed Sophia commit and the exact release binary, then hands
# the keyboard-independence gate to the tty launcher, which takes the display.
set -euo pipefail

# Changes: Sophia is the explicit pinned checkout SOPHIA_SOURCE (never this
# repository); nothing is built here or there: the binary and the pinned tree
# come from prepared physical inputs built from the signed tree in the private
# SOPHIA_GATE_BUILD_DIR (tools/lib/physical_inputs.sh); this repository is
# bound too; the console is
# SOPHIA_SESSION_TTY or the controlling terminal; Sophia's launcher receives
# absolute SOPHIA_BIN and SOPHIA_SESSION_PREFLIGHT (tools/lib/physical_runner.sh).
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=tools/lib/physical_runner.sh
source "$ROOT_DIR/tools/lib/physical_runner.sh"

# Any text console will do, as long as it is not the display manager's: the
# gate takes the GPU and the seat's input, and the launcher stops greetd and
# restores it, which it cannot do from greetd's own VT.
console="${SOPHIA_SESSION_TTY:-$(tty 2>/dev/null || true)}"
manager_vt="$(awk '
    /^\[[^]]+\][[:space:]]*$/ { terminal = ($0 ~ /^\[terminal\][[:space:]]*$/) }
    terminal && /^[[:space:]]*vt[[:space:]]*=/ { sub(/^[^=]*=[[:space:]]*/, ""); gsub(/[[:space:]]/, ""); print; exit }
' /etc/greetd/config.toml 2>/dev/null || true)"
if [[ ! -t 0 || ! "$console" =~ ^/dev/tty([1-9][0-9]*)$ ]]; then
    echo "Switch to a text console (Ctrl+Alt+F3, for example), log in, and run:" >&2
    echo "  SOPHIA_KEYBOARD_A=/dev/input/by-id/...-event-kbd SOPHIA_KEYBOARD_B=/dev/input/by-id/...-event-kbd \\" >&2
    echo "  $ROOT_DIR/tools/run_keyboard_independence_gate_tty4.sh" >&2
    exit 1
fi
console_vt="${BASH_REMATCH[1]}"
if [[ -n "$manager_vt" && "$console_vt" == "$manager_vt" ]]; then
    echo "tty$console_vt belongs to the display manager; log out of its session and use another console." >&2
    exit 1
fi
if [[ -z "${SOPHIA_KEYBOARD_A:-}" || -z "${SOPHIA_KEYBOARD_B:-}" ]]; then
    echo "set SOPHIA_KEYBOARD_A (the keyboard you will unplug) and SOPHIA_KEYBOARD_B" >&2
    echo "available keyboard paths:" >&2
    find /dev/input/by-id /dev/input/by-path -maxdepth 1 -type l -name '*-event-kbd' -print 2>/dev/null >&2 || true
    exit 1
fi
runner_tty "$console"
runner_inputs
if [[ -n "$(git -C "$SOPHIA_SOURCE" status --short)" ]]; then
    echo "Sophia worktree must be clean before the physical proof." >&2
    exit 1
fi
integration_commit="$(runner_integration_commit)"
# Archives bind this signed integration commit of this checkout.
export SOPHIA_INTEGRATION_COMMIT="$integration_commit" SOPHIA_INTEGRATION_SOURCE="$ROOT_DIR"

sophia_commit="$(git -C "$SOPHIA_SOURCE" rev-parse HEAD)"
git -C "$SOPHIA_SOURCE" verify-commit "$sophia_commit" >/dev/null 2>&1 || {
    echo "Physical-proof HEAD lacks a valid signature: $SOPHIA_SOURCE" >&2
    exit 1
}

echo "Preparing the exact physical-proof binary before DRM takeover..."
echo "Sophia: $sophia_commit"
physical_inputs_prepare --sophia-features=native-session
physical_inputs_bound "$integration_commit"
[[ "${PI[SOPHIA_COMMIT]}" == "$sophia_commit" ]] || {
    echo "The prepared inputs are not the bound Sophia commit." >&2
    exit 1
}
sophia_bin="${PI[SOPHIA_BIN]}"
# The launcher reads Sophia's retained files from the staged pinned tree.
SOPHIA_ROOT="${PI[SOPHIA_ROOT]}"
export SOPHIA_ROOT
if [[ -n "$(git -C "$SOPHIA_SOURCE" status --short)" \
    || "$(git -C "$SOPHIA_SOURCE" rev-parse HEAD)" != "$sophia_commit" ]]; then
    echo "Sophia source identity changed while the physical inputs were prepared." >&2
    exit 1
fi
git -C "$SOPHIA_SOURCE" verify-commit "$sophia_commit" >/dev/null 2>&1 || {
    echo "Sophia signature no longer verifies after the inputs were prepared." >&2
    exit 1
}

sophia_sha256="$(sha256sum "$sophia_bin" | awk '{ print $1 }')"
echo "Sophia binary: $sophia_sha256"
# The launcher never builds, so the atomic-scanout preflight runs here, by the
# prepared binary (its log stays in the private build directory).
physical_inputs_preflight "$sophia_bin" "$SOPHIA_ROOT" \
    "$SOPHIA_GATE_BUILD_DIR/preflight-$(date -u +%Y%m%dT%H%M%SZ)-$$.log"

export SOPHIA_TTY_PROFILE=keyboard-independence
export SOPHIA_TTY_NUMBER="$console_vt"
export SOPHIA_KEYBOARD_INDEPENDENCE_ARM=1
export SOPHIA_KEYBOARD_INDEPENDENCE_SEAT="${SOPHIA_KEYBOARD_INDEPENDENCE_SEAT:-seat0}"
export SOPHIA_KEYBOARD_INDEPENDENCE_SOURCE_COMMIT="$sophia_commit"
export SOPHIA_KEYBOARD_INDEPENDENCE_SOPHIA_SHA256="$sophia_sha256"
# Sophia's launcher receives the prepared binary this run bound, absolute.
export SOPHIA_BIN="$sophia_bin"
exec "$ROOT_DIR/tools/session/start_sophia_tty3.sh"
