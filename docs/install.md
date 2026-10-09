# Building and installing the desktop

The desktop is one Nix build. Sophia, Hagia, narthex, Lom, Bemenu and kleis
are inputs of this repository's flake, and `flake.lock` records the commit
and content hash of each. Building the same lock gives the same release.

```sh
nix flake update           # optional: move every input to its branch head
nix build .#desktop        # the whole release, as one store path
tools/desktop install      # copy it into /opt and make it current
```

`nix build` rebuilds only what changed and takes everything else from the
Nix store. A change to one component rebuilds that component and the small
derivation that lays out the release; with nothing to compile it takes about
ten seconds.

## What a release contains

The release holds the Sophia session binaries, Hagia and narthex, Lom,
Bemenu, kleis, the session tools and the desktop profile. It is laid out by
`cargo xtask assemble-nix` and sealed with `SHA256SUMS`. Its `manifest`
records the Sophia, Hagia, narthex and integration commits and Hagia's
vendored C SDK revision, which the build checks file by file.

The release is named by a hash of every locked input, for example
`niltempus-ecd0f9a391d5a6fc6b30`. That name is also its directory under
`/opt/sophia-niltempus-desktop/releases`, and the profile names each
component by its path there.

## The profile

The desktop profile is `profiles/desktop.kdl`. It names the window manager
and the components as `@hagia@`, `@lom@`, `@bemenu@` and `@kleis@`; the build
replaces these with the installed paths. Lom and kleis read their own
configuration files from your home directory at login.

Before the release is laid out, the build checks the profile with Sophia
(`sophia config check-session-profile`) and checks Hagia's policy, as the
login does. The build cannot read your home directory, so in that check each
component's configuration file is an empty stand-in; only its presence is
checked there.

The daily layout prefers DP-1 at 2560x1440, 120 Hz, with workspaces 1 through 6.
Its adaptive output policy tolerates an absent preferred monitor. If no configured
output is available, one connected unnamed output receives the desktop at its
preferred mode and keeps workspace affinity 1. This permits moving the main
monitor to another port without preventing login. Other unnamed outputs remain
disabled ordinarily. HDMI-A-2 is explicitly disabled for development; its absence
also permits login. Settings on a present named output still require support.
Fallback uses the monitor's preferred refresh rate with VRR disabled; the saved
120 Hz and VRR settings remain specific to DP-1.

This controls the desktop layout, not DRM card ownership. Sophia can open
cards assigned to its seat and bring up heads before reconciling the profile.
The host's iGPU seat1 rule keeps that card out of the normal seat0 session.
Running another KMS session still needs separate card and seat/VT ownership.
Startup fallback does not establish live output loss/return recovery.

## Installing

`tools/desktop install` builds the release, copies it to
`/opt/sophia-niltempus-desktop/releases/<name>`, and makes it `current`; the
release that was current becomes `previous`. It asks for sudo for the copy.
Log out and log in again to use the new release.

The copy belongs to root and no one else can write it, because Sophia's PAM
helper refuses to start under a directory that others can write, and the Nix
store is one. Before switching, the script compares the copy with the store
path, file for file and executable for executable, and refuses a copy that
differs. A release directory that is already there must match its store path
the same way; a damaged one is refused until it is removed. Each link is
replaced by renaming a complete new link over it.

Install, rollback and prune hold one lock, `/opt/sophia-niltempus-desktop/.lock`,
for as long as they change anything, so a second command waits for the first.
A build runs before the lock is taken. `tools/check_desktop_tool.sh` checks
all of this against a private prefix, without sudo.

The script also keeps a garbage-collection root for each installed release in
`~/.local/state/sophia-niltempus-desktop/gcroots`, so the libraries its
binaries load stay in the store, and it installs the login entry
`Sophia niltempus Desktop` into `/usr/share/wayland-sessions` when that entry
changes.

`tools/desktop install STORE-PATH` installs a release that is already built.

## Rolling back, status and pruning

```sh
tools/desktop rollback     # swap current and previous
tools/desktop status       # current and previous, with their commits
tools/desktop prune [N]    # keep current, previous and the N newest others (default 2)
```

If a login fails, switch to a text console, run `tools/desktop rollback`, and
log in again. A second rollback swaps the same two releases back.

Pruning removes old release directories and their roots. The store paths
they held are freed at the next garbage collection (`nix-collect-garbage`, or
automatically with the `min-free` and `max-free` settings in
`/etc/nix/nix.conf`).

## Working on a component

To build the desktop with work in progress, override that input:

```sh
nix build .#desktop --override-input hagia 'git+file:///home/niltempus/dev/hagia?ref=my-branch'
tools/desktop install ./result
```

The override applies to that build only; `flake.lock` is unchanged. Commit the
work first, since an input is read from its git history. Use cargo or nimble in
the component's own repository to edit and recompile; use Nix to assemble and
install the desktop.

The login runs the release's own Hagia. A personal Hagia under
`~/.local/state` is not used by this login entry; build Hagia changes into a
release as above.

## Graphics

Sophia and Lom are built against the Mesa that the flake pins. Their
graphics libraries find that Mesa's drivers through paths compiled into the
binaries, with no environment variables, so the desktop does not depend on the
host's graphics packages, and programs that the desktop starts keep using the
host's own.
