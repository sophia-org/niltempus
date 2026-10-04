#!/usr/bin/env bash
# Check tools/desktop against a private prefix. The script runs as uid 0 in a
# user namespace, so it never asks for sudo, with its own
# /opt/sophia-niltempus-desktop, login-entry directory and state directory.
# Fixture releases are small directories added to the Nix store. Needs
# bwrap, flock and a running Nix daemon. Prints
# "desktop tool checks passed" on success.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tool=$repo/tools/desktop
work=$(mktemp -d "${XDG_CACHE_HOME:-$HOME/.cache}/desktop-tool-check.XXXXXX")
trap 'chmod -R u+w "$work" 2>/dev/null; rm -rf "$work"' EXIT
opt=$work/opt
state=$work/state
gcroots=$state/sophia-niltempus-desktop/gcroots
mkdir -p "$opt" "$work/sessions" "$work/bin" "$state"

fail() { echo "check_desktop_tool: $*" >&2; exit 1; }

# A fixture release: a manifest naming ID, an executable, a data file and
# the login entry install reads.
declare -A store
fixture() {
    local dir=$work/src/niltempus-$1
    mkdir -p "$dir/bin" "$dir/target/release" "$dir/share/wayland-sessions"
    printf 'release_id=niltempus-%s\n' "$1" >"$dir/manifest"
    printf '#!/bin/sh\necho %s\n' "$1" >"$dir/bin/run"
    chmod 0755 "$dir/bin/run"
    printf 'data %s\n' "$1" >"$dir/target/release/data"
    printf '[Desktop Entry]\nName=Fixture\nExec=@SOPHIA_INSTALL_PREFIX@/current/bin/run\n' \
        >"$dir/share/wayland-sessions/sophia-niltempus-desktop.desktop"
    store[$1]=$(nix-store --add "$dir")
}

# tools/desktop as root in a user namespace, with the private directories
# mounted where it expects them.
desktop() {
    bwrap --unshare-user --uid 0 --gid 0 --die-with-parent --dev-bind / / \
        --tmpfs /opt --bind "$opt" /opt/sophia-niltempus-desktop \
        --bind "$work/sessions" /usr/share/wayland-sessions \
        --setenv HOME "$work" --setenv XDG_STATE_HOME "$state" \
        "$tool" "$@"
}

link() { readlink "$opt/$1" 2>/dev/null | sed 's|^releases/niltempus-||'; }
expect_links() {
    [ "$(link current)" = "$1" ] || fail "current is $(link current), expected $1"
    [ "$(link previous)" = "$2" ] || fail "previous is $(link previous), expected $2"
}
executables() { (cd "$1" && find . -type f -perm /111 | sort); }
equals_store() {
    diff -r --no-dereference "${store[$1]}" "$opt/releases/niltempus-$1" >/dev/null &&
        diff <(executables "${store[$1]}") <(executables "$opt/releases/niltempus-$1") >/dev/null
}
# Every state the lock allows: no stage left, both links name complete
# releases equal to their store paths, each with its GC root.
invariants() {
    if compgen -G "$opt/releases/.tmp-*" >/dev/null; then fail "a stage was left behind"; fi
    local name id
    for name in current previous; do
        id=$(link "$name")
        [ -n "$id" ] || continue
        [ -d "$opt/releases/niltempus-$id" ] || fail "$name names a missing release $id"
        equals_store "$id" || fail "$name release $id differs from its store path"
        [ -L "$gcroots/niltempus-$id" ] || fail "$name release $id has no GC root"
    done
}

for name in a b c d e; do fixture "$name"; done

# Install, a second install, the login entry and modes.
desktop install "${store[a]}" >/dev/null
expect_links a ""
desktop install "${store[b]}" >/dev/null
expect_links b a
grep -qx 'Exec=/opt/sophia-niltempus-desktop/current/bin/run' \
    "$work/sessions/sophia-niltempus-desktop.desktop" || fail "login entry prefix"
[ "$(stat -c %a "$opt/releases/niltempus-b/bin/run")" = 755 ] || fail "executable mode"
[ "$(stat -c %a "$opt/releases/niltempus-b/manifest")" = 644 ] || fail "data mode"
invariants

# A copy that differs from the store path, by content or by mode, is refused
# and leaves nothing behind.
cat >"$work/bin/cp" <<'COPY'
#!/bin/sh
/usr/bin/cp "$@" || exit
for d; do :; done
chmod u+w "$d/target/release/data"
echo x >>"$d/target/release/data"
COPY
chmod 0755 "$work/bin/cp"
if PATH=$work/bin:$PATH desktop install "${store[c]}" 2>/dev/null; then fail "a changed copy was accepted"; fi
cat >"$work/bin/cp" <<'COPY'
#!/bin/sh
/usr/bin/cp "$@" || exit
for d; do :; done
chmod a-x "$d/bin/run"
COPY
if PATH=$work/bin:$PATH desktop install "${store[c]}" 2>/dev/null; then fail "a copy without its executable bit was accepted"; fi
rm "$work/bin/cp"
[ ! -e "$opt/releases/niltempus-c" ] || fail "a refused copy was kept"
expect_links b a
invariants

# A damaged release already under the prefix is refused, not activated:
# changed content, then a lost executable bit.
echo damage >>"$opt/releases/niltempus-a/target/release/data"
if desktop install "${store[a]}" 2>/dev/null; then fail "a damaged existing release was activated"; fi
expect_links b a
rm -rf "$opt/releases/niltempus-a"
desktop install "${store[a]}" >/dev/null
expect_links a b
chmod a-x "$opt/releases/niltempus-b/bin/run"
if desktop install "${store[b]}" 2>/dev/null; then fail "a release without its executable bit was activated"; fi
expect_links a b
rm -rf "$opt/releases/niltempus-b"
desktop install "${store[b]}" >/dev/null
expect_links b a
invariants

# Rollback swaps, and a second rollback swaps back.
desktop rollback >/dev/null
expect_links a b
desktop rollback >/dev/null
expect_links b a

# The lock serializes: while another holder has it, install, rollback and
# prune change nothing; when it is released, each proceeds.
for command in "install ${store[c]}" rollback "prune 0"; do
    before_current=$(link current) before_previous=$(link previous)
    flock "$opt/.lock" sleep 3 &
    holder=$!
    sleep 1
    # shellcheck disable=SC2086 # the command and its argument split here
    desktop $command >/dev/null &
    waiting=$!
    sleep 1
    expect_links "$before_current" "$before_previous"
    [ ! -e "$opt/releases/niltempus-c" ] || [ "$command" != "install ${store[c]}" ] ||
        fail "install copied while the lock was held"
    wait "$holder"
    wait "$waiting" || fail "$command failed after the lock was released"
done
invariants

# Concurrent installs, rollbacks and prunes, in rounds; after each round the
# prefix is in a state the lock allows.
for round in 1 2 3 4 5; do
    desktop install "${store[d]}" >/dev/null &
    desktop install "${store[e]}" >/dev/null &
    desktop prune 0 >/dev/null &
    desktop rollback >/dev/null 2>&1 &
    desktop install "${store[a]}" >/dev/null &
    wait
    invariants || fail "round $round"
done

# Prune keeps current, previous and the N newest others, with their roots.
desktop install "${store[c]}" >/dev/null
desktop install "${store[d]}" >/dev/null
desktop prune 0 >/dev/null
[ "$(find "$opt/releases" -mindepth 1 -maxdepth 1 | wc -l)" = 2 ] || fail "prune 0 kept others"
[ "$(find "$gcroots" -mindepth 1 -maxdepth 1 -name 'niltempus-*' | wc -l)" = 2 ] ||
    fail "prune left GC roots of removed releases"
invariants

echo "desktop tool checks passed"
