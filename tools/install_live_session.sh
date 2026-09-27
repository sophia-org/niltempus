#!/usr/bin/env bash
# Provenance: moved from Sophia tools/install_live_session.sh at a6edbbcad02ad9e9bb4c790e7cfd714cf333d30b (original copy; source unchanged at the pin de776c68) (Sophia rule 13).
set -euo pipefail
umask 022

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# The artifact is always explicit (cargo xtask package-desktop); nothing is
# packaged implicitly from a checkout.
(( $# == 1 )) || {
    echo "usage: tools/install_live_session.sh ARTIFACT_DIR" >&2
    exit 1
}
artifact="$1"
[[ -d "$artifact" ]] || {
    echo "Artifact directory does not exist: $artifact" >&2
    exit 1
}
artifact="$(cd "$artifact" && pwd)"
PREFIX="${SOPHIA_INSTALL_PREFIX:-/opt/sophia}"

if [[ "$(id -u)" != 0 ]]; then
    writable_ancestor="$PREFIX"
    while [[ ! -e "$writable_ancestor" ]]; do
        parent="$(dirname "$writable_ancestor")"
        [[ "$parent" != "$writable_ancestor" ]] || break
        writable_ancestor="$parent"
    done
    [[ -d "$writable_ancestor" && -w "$writable_ancestor" ]] || {
        echo "Installation requires root for $PREFIX; run with sudo." >&2
        exit 1
    }
fi
release_id="$(sed -n 's/^release_id=//p' "$artifact/manifest" | head -n 1)"
[[ "$release_id" =~ ^[0-9A-Za-z._-]+$ ]] || {
    echo "Artifact has an invalid release_id." >&2
    exit 1
}
(
    cd "$artifact"
    sha256sum -c SHA256SUMS
)

releases="$PREFIX/releases"
target="$releases/$release_id"
install -d -m 755 "$releases"
[[ ! -e "$target" ]] || {
    echo "Release is already installed: $target" >&2
    exit 1
}
staging="$releases/.install-$release_id-$$"
trap '[[ ! -d "$staging" ]] || mv "$staging" "$staging.failed"' EXIT
cp -a "$artifact" "$staging"
# Older artifacts may have inherited a private umask from a caller. These
# public release files are read by the session user after root installs them.
chmod 644 "$staging/manifest" "$staging/SHA256SUMS" \
    "$staging"/share/wayland-sessions/*.desktop
if [[ "$(id -u)" == 0 ]]; then
    chown -R 0:0 -- "$staging"
fi
(
    cd "$staging"
    sha256sum -c SHA256SUMS
)
"$staging/tools/verify_packaged_policy.sh" "$staging"
# The installer's own verifier too: a candidate's packaged verifier may be an
# older one (for example one that accepts schema 6), and a candidate must
# satisfy this repository's current release schema.
"$ROOT_DIR/tools/verify_packaged_policy.sh" "$staging"
mv "$staging" "$target"
"$ROOT_DIR/tools/activate_live_session_release.sh" "$target"
trap - EXIT

echo "Installed Sophia release: $release_id"
echo "Operator guide: $PREFIX/current/share/doc/sophia/operations.md"
