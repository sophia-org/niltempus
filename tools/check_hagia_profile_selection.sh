#!/usr/bin/env bash
# Provenance: moved from Sophia tools/check_hagia_profile_selection.sh at a6edbbcad02ad9e9bb4c790e7cfd714cf333d30b (original copy; source unchanged at the pin de776c68) (Sophia rule 13).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d)"
trap 'rm -rf -- "$fixture"' EXIT
release="$fixture/release"
config_home="$fixture/config"
capture="$fixture/capture"
retired_dir="$config_home/hagia"
install -d -m 755 "$release/bin" "$release/share/sophia-policy/hagia" "$retired_dir"
install -m 755 "$ROOT_DIR/tools/installed/sophia-hagia-session" \
    "$release/bin/sophia-hagia-session"
install -m 755 "$ROOT_DIR/tools/installed/sophia-hagia-promotion-session" \
    "$release/bin/sophia-hagia-promotion-session"
printf 'schema 1\n' >"$release/share/sophia-policy/hagia/default.kdl"
printf '#!/usr/bin/env bash\nprintf "%%s|%%s|%%s\\n" "$SOPHIA_DESKTOP_PROFILE_MODE" "$SOPHIA_DESKTOP_PROFILE" "$SOPHIA_INSTALLED_ATTEMPT_MODE" >"$SOPHIA_PROFILE_CAPTURE"\n' \
    >"$release/bin/sophia-session"
chmod 755 "$release/bin/sophia-session"

run_session() {
    env XDG_CONFIG_HOME="$config_home" \
        SOPHIA_PROFILE_CAPTURE="$capture" \
        SOPHIA_DESKTOP_PROFILE="${SOPHIA_DESKTOP_PROFILE:-}" "$@"
    cat "$capture"
}

observed="$(run_session "$release/bin/sophia-hagia-session")"
[[ "$observed" == "packaged-fallback|$release/share/sophia-policy/hagia/default.kdl|hagia" ]]

# The retired per-client profile location is no longer consulted: with only
# that file present the session still falls back to the packaged default.
printf 'schema 1\n' >"$retired_dir/config.kdl"
observed="$(run_session "$release/bin/sophia-hagia-session")"
[[ "$observed" == "packaged-fallback|$release/share/sophia-policy/hagia/default.kdl|hagia" ]]

install -d -m 700 "$config_home/sophia"
printf 'schema 1\n' >"$config_home/sophia/desktop.kdl"
observed="$(run_session "$release/bin/sophia-hagia-session")"
[[ "$observed" == "user|$config_home/sophia/desktop.kdl|hagia" ]]

explicit="$fixture/explicit.kdl"
printf 'schema 1\n' >"$explicit"
observed="$(SOPHIA_DESKTOP_PROFILE="$explicit" \
    run_session "$release/bin/sophia-hagia-session")"
[[ "$observed" == "explicit|$explicit|hagia" ]]

observed="$(SOPHIA_DESKTOP_PROFILE="$explicit" \
    run_session "$release/bin/sophia-hagia-promotion-session")"
[[ "$observed" == "packaged-promotion|$release/share/sophia-policy/hagia/default.kdl|hagia-promotion" ]]

echo "Hagia daily and promotion profile selection checks passed."
