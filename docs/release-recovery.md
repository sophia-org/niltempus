# Recover with a complete retained release

Source retirement in Sophia does not remove already installed releases. A
recovery target must pass verification and match its activation history before
selection. This procedure changes no running session; use it after logging out.

## Verified target for WM/shell IPC retirement

The following target was verified read-only on 2026-09-28 with niltempus
`b21eb20`. Its manifest and checksum list match the installed activation ledger:

- Release: `niltempus-4f498ff25c3a6d89c16f`.
- Sophia: `2d69924a9cac3ed1164089c7d1fae23b46d19d71`.
- Hagia: `69f427abb0565d10c04dab302915396048252dcf`, C SDK `8decca1d73699d6750c9228ecbf6e27f589d965d`.
- Lom: `53d3a921d929c92213feeff1564f436d65759d74`.
- Bemenu: `8e0d56d3ca292edf056822eedcbf2985a1cd0083`.
- Manifest SHA-256: `6c0de096e50eac78739d64c44aada29a832fcab45271c99cd7803419f2f7091d`.
- SHA256SUMS SHA-256: `1c7740f1cdd30e45848e85f8172c8d147c77270fba922f53773e0439b0c6494b`.

This is a compatible 9P WM/bar/menu release. It also contains the older Narthex
`50b9014d`, which the named desktop profile does not select. No fallback to an
IPC component is implied.

In Bash, verify the exact retained identity before selecting it:

```bash
release=/opt/sophia-niltempus-desktop/releases/niltempus-4f498ff25c3a6d89c16f
test "$(sha256sum "$release/manifest" | cut -d ' ' -f 1)" = 6c0de096e50eac78739d64c44aada29a832fcab45271c99cd7803419f2f7091d &&
test "$(sha256sum "$release/SHA256SUMS" | cut -d ' ' -f 1)" = 1c7740f1cdd30e45848e85f8172c8d147c77270fba922f53773e0439b0c6494b &&
niltempus verify "$release" &&
niltempus install "$release"
```

Ordinary `niltempus rollback` instead selects the verified `previous` link.
That link can move, so it is not a permanent name for the target above. A
recorded release whose manifest or checksum-list identity changed is refused.
An unrecorded older release cannot acquire rollback authority from a link.

## Pin the components as well as Sophia

Normal installation and rollback preserve personal component selections. The
normal login entry therefore does **not** guarantee use of the old release's
component binaries. If a component update is part of the problem, log into a
local text console after ending the graphical session, and use the verified
release's lower-level launcher with its sealed profile and WM:

```bash
release=/opt/sophia-niltempus-desktop/releases/niltempus-4f498ff25c3a6d89c16f
niltempus verify "$release" &&
env -u SOPHIA_DESKTOP_PROFILE_MODE -u SOPHIA_HAGIA_PROFILE_MODE \
    SOPHIA_DESKTOP_PROFILE="$release/share/sophia-niltempus-desktop/desktop.kdl" \
    SOPHIA_HAGIA_BIN="$release/target/release/hagia" \
    SOPHIA_WM_BIN="$release/target/release/hagia" \
    "$release/bin/sophia-hagia-session" \
    "--wm-process=$release/target/release/hagia" --wm-transport=9p2000.L
```

The sealed profile names this same release's Lom and Bemenu executables. This
entry bypasses personal component-profile rendering without changing personal
files. It retains the release's host preflight, TTY and input guards; do not use
it alongside another graphical session. For another release, verify its own
profile, component identities and launcher contract first.

## Evidence and limits

The real installer passed initial installation, repeated installation, a second
selection, rollback and status using this artifact in private `/opt` and
display-manager mounts. The test also preserved an independently updated user
WM's content, mode, inode and mtime. Selection damage, missing artifacts,
activation-history tampering and release tampering were tested separately.

Evidence is under `development-evidence/ipc-retirement/t269-rollback-*`. The
verification binary SHA-256 was
`57a8675a9f389de113147ebc57a64a45b114570007395d2d8225ab785c159ece`.
No host installation, activation or graphical recovery run was performed.
The sealed-component command above is based on the verified launch scripts and
profile; it is not a new physical acceptance or latency result.
