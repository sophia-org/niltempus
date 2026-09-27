#!/usr/bin/env bash
# Provenance: moved from Sophia tools/check_installed_session_type.sh at a6edbbcad02ad9e9bb4c790e7cfd714cf333d30b (original copy; source unchanged at the pin de776c68) (Sophia rule 13).
# The installed session must tell clients what they will actually find.
#
# Sophia serves clients over X11 and offers no Wayland socket, but its session
# entries live under `wayland-sessions/` because a display manager starts those
# as self-contained servers, which is how Sophia runs. That placement makes the
# display manager declare the session Wayland. An Ozone client believes it,
# selects a Wayland backend, finds no socket, and refuses to start.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT

release="$work/release"
install -d -m 755 "$release/bin" "$release/tools/lib" "$release/target/release"
install -m 755 "$root/tools/installed/sophia-session" "$release/bin/sophia-session"
printf 'version=0.0.0\ncommit=%s\n' "$(printf '0%.0s' {1..40})" >"$release/manifest"
: >"$release/tools/lib/session_lifecycle.sh"
: >"$release/tools/run_sophia_session.sh"
: >"$release/tools/sophia_tty_mode.py"
for stub in hagia narthex sophia-wm-demo; do
    : >"$release/target/release/$stub"
    chmod 755 "$release/target/release/$stub"
done

# The Engine stub stands in for the session owner and records the environment
# it was handed. That environment is what every client in the session inherits.
cat >"$release/target/release/sophia" <<'STUB'
#!/usr/bin/env bash
{
    printf 'ARGV=%s\n' "$*"
    printf 'PREFLIGHT=%s\n' "${SOPHIA_SESSION_PREFLIGHT-<unset>}"
    printf 'XTASK=%s\n' "${SOPHIA_INTEGRATION_XTASK-<unset>}"
    printf 'ROOT=%s\n' "${SOPHIA_ROOT-<unset>}"
    printf 'XDG_SESSION_TYPE=%s\n' "${XDG_SESSION_TYPE-<unset>}"
    printf 'WAYLAND_DISPLAY=%s\n' "${WAYLAND_DISPLAY-<unset>}"
} >"$SOPHIA_SESSION_TYPE_CAPTURE"
STUB
chmod 755 "$release/target/release/sophia"

capture="$work/environment"
export SOPHIA_SESSION_TYPE_CAPTURE="$capture"
# A display manager launching a `wayland-sessions/` entry declares the session
# Wayland and may advertise a socket. Both are wrong here, and the launcher has
# to correct them rather than pass them on.
# This fixture models ordinary login, even when invoked from a proof session.
XDG_SESSION_TYPE=wayland \
    WAYLAND_DISPLAY=wayland-0 \
    XDG_STATE_HOME="$work/state" \
    SOPHIA_TTY_PROFILE=hagia \
    SOPHIA_INSTALLED_ATTEMPT_MODE=hagia \
    SOPHIA_DESKTOP_PROFILE_MODE=user \
    SOPHIA_TRUECOLOR_PROOF=false \
    SOPHIA_SESSION_WATCHDOG_SECONDS= \
    "$release/bin/sophia-session"

observed_type="$(sed -n 's/^XDG_SESSION_TYPE=//p' "$capture")"
observed_display="$(sed -n 's/^WAYLAND_DISPLAY=//p' "$capture")"

if [[ "$observed_type" != x11 ]]; then
    echo "installed session declared XDG_SESSION_TYPE=$observed_type; clients reach Sophia over X11" >&2
    exit 1
fi
if [[ "$observed_display" != '<unset>' ]]; then
    echo "installed session left WAYLAND_DISPLAY=$observed_display; Sophia offers no Wayland socket" >&2
    exit 1
fi

# Sophia's supervisor sees only the opaque label and this repository's
# adapter, with every path absolute inside the sealed release.
expect_line() {
    grep -Fqx "$1" "$capture" || {
        echo "installed session handed Sophia an unexpected launch: wanted $1" >&2
        cat "$capture" >&2
        exit 1
    }
}
expect_line "ARGV=session _supervise --profile=managed -- $release/tools/session/run_desktop_session.sh"
expect_line "PREFLIGHT=$release/target/release/active-session-preflight"
expect_line "XTASK=$release/target/release/sophia-integration-xtask"
expect_line "ROOT=$release"

printf 'sophia_installed_session_type schema=1 status=x11 wayland_display=unset\n'
