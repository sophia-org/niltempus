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
(
    cd "$target"
    sha256sum -c SHA256SUMS
)
"$target/tools/verify_packaged_policy.sh" "$target"
sophia_surface_install "$target" "$PREFIX" "$SESSION_DIR" "$COMMAND_DIR"

ln -sfn "$previous" "$PREFIX/current"
ln -sfn "$current" "$PREFIX/previous"
echo "Rolled back Sophia: current=$previous previous=$current"
