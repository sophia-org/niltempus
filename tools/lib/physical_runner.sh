# shellcheck shell=bash
# Common explicit inputs of the moved physical runners (Sophia rule 13).
# Requires ROOT_DIR (this repository); sources tools/lib/artifacts.sh.
#
# SOPHIA_SOURCE            the absolute clean Sophia checkout whose signed HEAD
#                          is the pinned revision (pins/sophia.toml). Nothing is
#                          built there: runners that build take their binaries
#                          and the exact pinned tree from prepared physical
#                          inputs (tools/lib/physical_inputs.sh), whose staged
#                          tree becomes SOPHIA_ROOT. Never a sibling default.
# SOPHIA_GATE_BUILD_DIR    the absolute private build directory the prepared
#                          inputs are built in (runners that build).
# SOPHIA_SESSION_PREFLIGHT the absolute host checker, handed to the launcher.
# SOPHIA_INTEGRATION_XTASK the absolute recipe tool, for launches through
#                          tools/session.
# SOPHIA_SESSION_TTY       the target TTY; otherwise the controlling terminal.
#
# Every runner that starts a session hands Sophia's launcher an absolute
# SOPHIA_BIN (the binary it built and bound) and SOPHIA_SESSION_PREFLIGHT.

# shellcheck source=tools/lib/artifacts.sh
. "$ROOT_DIR/tools/lib/artifacts.sh"
# shellcheck source=tools/lib/physical_inputs.sh
. "$ROOT_DIR/tools/lib/physical_inputs.sh"

runner_inputs() {
    local name
    [[ "${SOPHIA_SOURCE:-}" == /* ]] || {
        echo "SOPHIA_SOURCE must name the absolute pinned Sophia checkout (no default)." >&2
        exit 2
    }
    for name in SOPHIA_SESSION_PREFLIGHT SOPHIA_INTEGRATION_XTASK; do
        [[ "${!name:-}" == /* && -f "${!name}" && -x "${!name}" ]] || {
            echo "$name must name an absolute executable (no default)." >&2
            exit 2
        }
    done
    check_sophia_source "$SOPHIA_SOURCE"
    SOPHIA_SOURCE="$(cd -- "$SOPHIA_SOURCE" && pwd -P)"
    SOPHIA_ROOT="$SOPHIA_SOURCE"
    export SOPHIA_SOURCE SOPHIA_ROOT SOPHIA_SESSION_PREFLIGHT SOPHIA_INTEGRATION_XTASK
}

# runner_tty [EXPECTED]: the target TTY from SOPHIA_SESSION_TTY or the
# controlling terminal; refused when there is none, or it is not EXPECTED.
runner_tty() {
    local expected="${1:-}"
    RUNNER_TTY="${SOPHIA_SESSION_TTY:-}"
    [[ -n "$RUNNER_TTY" ]] || RUNNER_TTY="$(tty 2>/dev/null || true)"
    [[ "$RUNNER_TTY" == /dev/* ]] || {
        echo "No target TTY: run from a local TTY or set SOPHIA_SESSION_TTY." >&2
        exit 2
    }
    [[ -z "$expected" || "$RUNNER_TTY" == "$expected" ]] || {
        echo "Run this from $expected (the target TTY is $RUNNER_TTY)." >&2
        exit 2
    }
    SOPHIA_SESSION_TTY="$RUNNER_TTY"
    export SOPHIA_SESSION_TTY
}

# runner_integration_commit: this repository is clean and its HEAD signed;
# prints the HEAD it binds.
runner_integration_commit() {
    local commit
    [[ -z "$(git -C "$ROOT_DIR" status --porcelain --untracked-files=all)" ]] || {
        echo "Integration worktree must be clean before the physical proof." >&2
        exit 1
    }
    commit="$(git -C "$ROOT_DIR" rev-parse HEAD)"
    git -C "$ROOT_DIR" verify-commit "$commit" >/dev/null 2>&1 || {
        echo "Integration HEAD lacks a valid signature." >&2
        exit 1
    }
    printf '%s\n' "$commit"
}

# runner_sophia_bin: runners that do not build take the caller's prebuilt
# Sophia, which must be an absolute executable (no default).
runner_sophia_bin() {
    [[ "${SOPHIA_BIN:-}" == /* && -f "${SOPHIA_BIN:-}" && -x "${SOPHIA_BIN:-}" ]] || {
        echo "SOPHIA_BIN must name an absolute prebuilt Sophia executable (no default)." >&2
        exit 2
    }
    export SOPHIA_BIN
}
