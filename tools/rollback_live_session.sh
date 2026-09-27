#!/usr/bin/env bash
# Provenance: moved from Sophia tools/rollback_live_session.sh at a6edbbcad02ad9e9bb4c790e7cfd714cf333d30b (original copy; source unchanged at the pin de776c68) (Sophia rule 13).
set -euo pipefail

PREFIX="${SOPHIA_INSTALL_PREFIX:-/opt/sophia}"
SESSION_DIR="${SOPHIA_SESSION_DIR:-/usr/share/wayland-sessions}"
COMMAND_DIR="${SOPHIA_COMMAND_DIR:-/usr/local/bin}"
self="$(readlink -f "$0")"
source_release="$(cd "$(dirname "$self")/.." && pwd -P)"
# shellcheck source=tools/lib/live_session_surface.sh
source "$source_release/tools/lib/live_session_surface.sh"
# shellcheck source=tools/lib/activation_ledger.sh
source "$source_release/tools/lib/activation_ledger.sh"
current="$(readlink "$PREFIX/current" 2>/dev/null || true)"
previous="$(readlink "$PREFIX/previous" 2>/dev/null || true)"
[[ -n "$current" && -n "$previous" ]] || {
    echo "Rollback requires both $PREFIX/current and $PREFIX/previous." >&2
    exit 1
}
[[ -d "$PREFIX/$previous" ]] || {
    echo "Previous release is missing: $PREFIX/$previous" >&2
    exit 1
}
target="$(readlink -f "$PREFIX/$previous")"
releases="$(readlink -f "$PREFIX/releases")"
[[ -n "$target" && "$target" == "$releases/"* && ! -L "$target" ]] || {
    echo "Previous release is outside the immutable release directory." >&2
    exit 1
}
# Only a recorded release with exactly its recorded contents is a rollback
# target (a prefix without a ledger records its current and previous once).
# It keeps its own packaged verifier. An unrecorded one is a new candidate:
# activate it with tools/activate_live_session_release.sh, which applies the
# current verifier.
activation_ledger_bootstrap
target_id="$(activation_release_id "$target")"
[[ "$target_id" =~ ^[0-9A-Za-z._-]+$ && "$target" == "$releases/$target_id" ]] || {
    echo "Previous release has an invalid release_id or path." >&2
    exit 1
}
activation_history="$(activation_ledger_status "$target_id" "$target")" || exit 1
[[ "$activation_history" == recorded ]] || {
    echo "Previous release $target_id was never activated here; activate it as a new candidate instead." >&2
    exit 1
}
(
    cd "$target"
    sha256sum -c SHA256SUMS
)
"$target/tools/verify_packaged_policy.sh" "$target"
sophia_surface_install "$target" "$PREFIX" "$SESSION_DIR" "$COMMAND_DIR"

ln -sfn "$previous" "$PREFIX/current"
ln -sfn "$current" "$PREFIX/previous"
echo "Rolled back Sophia: current=$previous previous=$current"
