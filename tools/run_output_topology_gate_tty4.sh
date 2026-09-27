#!/usr/bin/env bash
# Provenance: moved from Sophia tools/run_output_topology_gate_tty4.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
set -euo pipefail

# One-command signed physical proof for output loss and return. This gate
# performs a real modeset and therefore runs only from a recovery-safe TTY.

SCRIPT_PATH="$(readlink -f "${BASH_SOURCE[0]}")"
ROOT_DIR="$(cd "$(dirname "$SCRIPT_PATH")/.." && pwd)"
# Changes: Sophia is the explicit pinned checkout SOPHIA_SOURCE (never this
# repository): built there, its DRM-master guard read from it; Hagia is an
# explicit checkout or binary (no sibling default); this repository is bound
# too; the TTY comes from SOPHIA_SESSION_TTY or the controlling terminal;
# absolute SOPHIA_BIN and SOPHIA_SESSION_PREFLIGHT are exported
# (tools/lib/physical_runner.sh).
# shellcheck source=tools/lib/physical_runner.sh
source "$ROOT_DIR/tools/lib/physical_runner.sh"
TTY_REQUIRED="${SOPHIA_OUTPUT_TOPOLOGY_TTY:-/dev/tty4}"
SEAT="${SOPHIA_OUTPUT_TOPOLOGY_SEAT:-seat0}"
HAGIA_ROOT="${SOPHIA_HAGIA_ROOT:-}"
EVIDENCE="${SOPHIA_OUTPUT_TOPOLOGY_EVIDENCE:-/tmp/sophia-output-topology-$(date +%Y%m%d-%H%M%S).log}"
EVIDENCE_LATEST="${SOPHIA_OUTPUT_TOPOLOGY_EVIDENCE_LATEST:-/tmp/sophia-output-topology-physical.log}"

usage() {
    cat <<USAGE
usage: tools/run_output_topology_gate_tty4.sh

From tty4, run the complete signed output disconnect/reconnect gate with
safe defaults. Hagia is built from the clean signed checkout at $HAGIA_ROOT so
its policy wire matches current Sophia. Environment overrides remain available
through SOPHIA_OUTPUT_TOPOLOGY_*, SOPHIA_HAGIA_ROOT, SOPHIA_HAGIA_BIN,
SOPHIA_TERMINAL_BIN, and SOPHIA_FIREFOX_BIN.
USAGE
}

case "${1:-}" in
    -h | --help)
        usage
        exit 0
        ;;
    "") ;;
    *)
        echo "Unknown argument: $1" >&2
        usage >&2
        exit 2
        ;;
esac
if (( $# > 1 )); then
    echo "This gate accepts no positional arguments." >&2
    usage >&2
    exit 2
fi

refuse() {
    printf 'Output-topology gate refused: %s\n' "$*" >&2
    exit 2
}

current_tty="${SOPHIA_SESSION_TTY:-$(tty 2>/dev/null || true)}"
if [[ ! -t 0 || "$current_tty" != "$TTY_REQUIRED" ]]; then
    refuse "switch to $TTY_REQUIRED with Ctrl+Alt+F4, log in, and run $SCRIPT_PATH"
fi
runner_tty "$TTY_REQUIRED"
runner_inputs

if [[ -n "$(git -C "$SOPHIA_SOURCE" status --porcelain --untracked-files=all)" ]]; then
    refuse "Sophia worktree must be clean before a signed physical gate."
fi
integration_commit="$(runner_integration_commit)"
source_commit="$(git -C "$SOPHIA_SOURCE" rev-parse HEAD)"
git -C "$SOPHIA_SOURCE" verify-commit "$source_commit" >/dev/null 2>&1 ||
    refuse "Sophia HEAD must have a valid cryptographic signature."

if [[ -n "${SOPHIA_HAGIA_BIN:-}" ]]; then
    HAGIA_BIN="$SOPHIA_HAGIA_BIN"
    BUILD_HAGIA=0
    HAGIA_SOURCE_COMMIT=external
else
    [[ "$HAGIA_ROOT" == /* && -e "$HAGIA_ROOT/.git" ]] ||
        refuse "Hagia checkout not found at $HAGIA_ROOT; set SOPHIA_HAGIA_ROOT or SOPHIA_HAGIA_BIN."
    if [[ -n "$(git -C "$HAGIA_ROOT" status --porcelain --untracked-files=all)" ]]; then
        refuse "Hagia worktree must be clean before a signed physical gate."
    fi
    HAGIA_SOURCE_COMMIT="$(git -C "$HAGIA_ROOT" rev-parse HEAD)"
    git -C "$HAGIA_ROOT" verify-commit "$HAGIA_SOURCE_COMMIT" >/dev/null 2>&1 ||
        refuse "Hagia HEAD must have a valid cryptographic signature."
    command -v nim >/dev/null 2>&1 ||
        refuse "Nim is required to build the current Hagia policy client."
    HAGIA_BIN="${TMPDIR:-/tmp}/hagia-output-topology-${HAGIA_SOURCE_COMMIT:0:12}"
    HAGIA_NIMCACHE="${TMPDIR:-/tmp}/hagia-output-topology-nimcache-${HAGIA_SOURCE_COMMIT:0:12}"
    BUILD_HAGIA=1
fi
if (( ! BUILD_HAGIA )) && [[ -z "$HAGIA_BIN" || ! -x "$HAGIA_BIN" ]]; then
    refuse "Hagia was not found; set SOPHIA_HAGIA_BIN to its executable path."
fi
if (( ! BUILD_HAGIA )); then
    HAGIA_BIN="$(readlink -f "$HAGIA_BIN")"
fi

mapfile -t connected_outputs < <(
    for status in /sys/class/drm/card*-*/status; do
        [[ -r "$status" && "$(<"$status")" == connected ]] || continue
        connector="${status%/status}"
        basename "$connector" | sed -E 's/^card[0-9]+-//'
    done | sort
)
if (( ${#connected_outputs[@]} < 2 )); then
    refuse "connect at least two physical outputs (observed ${#connected_outputs[@]}: ${connected_outputs[*]:-none})."
fi

# shellcheck source=/dev/null
. "$SOPHIA_SOURCE/tools/lib/drm_master_guard.sh"
if ! drm_master_refusal="$(sophia_require_drm_master_available SOPHIA_OUTPUT_TOPOLOGY_FORCE 2>&1)"; then
    refuse "$drm_master_refusal"
fi

if (( BUILD_HAGIA )); then
    echo "Building signed Hagia source $HAGIA_SOURCE_COMMIT before DRM takeover..."
    (
        cd "$HAGIA_ROOT"
        nim c -d:release --path:src --nimcache:"$HAGIA_NIMCACHE" \
            -o:"$HAGIA_BIN" src/hagia.nim
    )
    if [[ -n "$(git -C "$HAGIA_ROOT" status --porcelain --untracked-files=all)" \
        || "$(git -C "$HAGIA_ROOT" rev-parse HEAD)" != "$HAGIA_SOURCE_COMMIT" ]]; then
        refuse "Hagia source identity changed during the physical-gate build."
    fi
    git -C "$HAGIA_ROOT" verify-commit "$HAGIA_SOURCE_COMMIT" >/dev/null 2>&1 ||
        refuse "Hagia HEAD signature no longer verifies after the build."
fi

echo "Building signed Sophia source $source_commit before DRM takeover..."
(
    cd "$SOPHIA_SOURCE"
    cargo build --quiet --release --offline -p sophia-cli \
        --features native-session --target-dir "$SOPHIA_SOURCE/target"
)
if [[ -n "$(git -C "$SOPHIA_SOURCE" status --porcelain --untracked-files=all)" \
    || -n "$(git -C "$ROOT_DIR" status --porcelain --untracked-files=all)" \
    || "$(git -C "$ROOT_DIR" rev-parse HEAD)" != "$integration_commit" \
    || "$(git -C "$SOPHIA_SOURCE" rev-parse HEAD)" != "$source_commit" ]]; then
    refuse "Sophia or integration source identity changed during the physical-gate build."
fi
git -C "$SOPHIA_SOURCE" verify-commit "$source_commit" >/dev/null 2>&1 ||
    refuse "Sophia HEAD signature no longer verifies after the build."

mkdir -p "$(dirname "$EVIDENCE")"
if [[ -n "$EVIDENCE_LATEST" && "$EVIDENCE_LATEST" != "$EVIDENCE" ]]; then
    ln -sfn "$EVIDENCE" "$EVIDENCE_LATEST"
fi

echo "Hagia:   $HAGIA_BIN"
echo "Hagia source: $HAGIA_SOURCE_COMMIT"
echo "Evidence: $EVIDENCE"
export SOPHIA_OUTPUT_TOPOLOGY_ARM=1
export SOPHIA_OUTPUT_TOPOLOGY_SEAT="$SEAT"
export SOPHIA_OUTPUT_TOPOLOGY_EVIDENCE="$EVIDENCE"
export SOPHIA_HAGIA_BIN="$HAGIA_BIN"
export SOPHIA_LIVE_SESSION_SKIP_BUILD=1
# The persistent proof runs the binary this run built and bound, absolute.
export SOPHIA_BIN="$SOPHIA_SOURCE/target/release/sophia"

"$ROOT_DIR/tools/output_topology_physical_gate.sh"

if [[ -n "$(git -C "$SOPHIA_SOURCE" status --porcelain --untracked-files=all)" \
    || -n "$(git -C "$ROOT_DIR" status --porcelain --untracked-files=all)" \
    || "$(git -C "$ROOT_DIR" rev-parse HEAD)" != "$integration_commit" \
    || "$(git -C "$SOPHIA_SOURCE" rev-parse HEAD)" != "$source_commit" ]]; then
    refuse "Sophia or integration source identity changed during the physical gate."
fi
git -C "$SOPHIA_SOURCE" verify-commit "$source_commit" >/dev/null 2>&1 ||
    refuse "Sophia HEAD signature no longer verifies after the gate."
if (( BUILD_HAGIA )); then
    if [[ -n "$(git -C "$HAGIA_ROOT" status --porcelain --untracked-files=all)" \
        || "$(git -C "$HAGIA_ROOT" rev-parse HEAD)" != "$HAGIA_SOURCE_COMMIT" ]]; then
        refuse "Hagia source identity changed during the physical gate."
    fi
    git -C "$HAGIA_ROOT" verify-commit "$HAGIA_SOURCE_COMMIT" >/dev/null 2>&1 ||
        refuse "Hagia HEAD signature no longer verifies after the gate."
fi

echo "Signed output-topology gate passed for Sophia $source_commit and Hagia $HAGIA_SOURCE_COMMIT"
