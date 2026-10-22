#!/usr/bin/env bash
# t339 one-run Session qualification: Lom presents through the DMA-BUF
# capture path (r10 bit 14, mask 6) on one fixed-refresh output.
# Consumes only already-built pinned binaries at their original paths
# (cargo-slot outputs); nothing is copied and nothing is built here.
# Correctness qualification only: no performance, matrix or anchor
# runs, and no default enablement claim.
set -euo pipefail

umask 077
[[ $# -eq 0 ]] || { echo "usage: run_t339_capture_gate_tty4.sh" >&2; exit 2; }

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

required=(SOPHIA_SOURCE SOPHIA_BIN SOPHIA_SHA256 SOPHIA_INTEGRATION_XTASK SOPHIA_SESSION_PREFLIGHT
SOPHIA_LOM_BIN SOPHIA_LOM_COMMIT SOPHIA_LOM_SHA256 SOPHIA_HAGIA_BIN
SOPHIA_FACTOTUM_AGENT SOPHIA_FACTOTUM_PAM_HELPER)
EVIDENCE_DIR="${SOPHIA_T339_EVIDENCE_DIR:-$ROOT_DIR/.artifacts/t339-capture-native/$(date -u +%Y%m%dT%H%M%SZ)}"
RENDER_NODE="/dev/dri/renderD128"

[[ "$(tty)" == /dev/tty4 ]] || { echo "Run this from /dev/tty4 after ending the graphical session." >&2; exit 2; }
[[ "${SOPHIA_T339_GATE_ARM:-0}" == 1 ]] || { echo "Set SOPHIA_T339_GATE_ARM=1 to run the t339 capture gate." >&2; exit 2; }
for name in "${required[@]}"; do
    [[ -n "${!name:-}" ]] || { echo "$name is required (no default)" >&2; exit 2; }
done
for name in SOPHIA_BIN SOPHIA_INTEGRATION_XTASK SOPHIA_SESSION_PREFLIGHT SOPHIA_LOM_BIN SOPHIA_HAGIA_BIN SOPHIA_FACTOTUM_AGENT SOPHIA_FACTOTUM_PAM_HELPER; do
    [[ "${!name}" == /* && -f "${!name}" && -x "${!name}" ]] || {
        echo "$name must name an absolute prebuilt executable" >&2; exit 2;
    }
done
[[ -z "$(git -C "$ROOT_DIR" status --short -- tools/run_t339_capture_gate_tty4.sh tools/t339-foreign-driver.sh)" ]] || {
    echo "Gate scripts must be committed before the run" >&2; exit 2; }
[[ "$SOPHIA_SHA256" =~ ^[0-9a-f]{64}$ && "$SOPHIA_LOM_SHA256" =~ ^[0-9a-f]{64}$ ]] || {
    echo "Binary sha256 pins must be 64 lowercase hex" >&2; exit 2; }
[[ "$SOPHIA_LOM_COMMIT" =~ ^[0-9a-f]{40}$ ]] || { echo "SOPHIA_LOM_COMMIT must be a full commit" >&2; exit 2; }
[[ "$(sha256sum "$SOPHIA_BIN" | cut -d' ' -f1)" == "$SOPHIA_SHA256" ]] || {
    echo "Sophia binary does not match SOPHIA_SHA256" >&2; exit 2; }
[[ "$(sha256sum "$SOPHIA_LOM_BIN" | cut -d' ' -f1)" == "$SOPHIA_LOM_SHA256" ]] || {
    echo "Lom binary does not match SOPHIA_LOM_SHA256" >&2; exit 2; }
SOPHIA_COMMIT="$(git -C "$SOPHIA_SOURCE" rev-parse HEAD)"
git -C "$SOPHIA_SOURCE" verify-commit HEAD >/dev/null
INTEGRATION_COMMIT="$(git -C "$ROOT_DIR" rev-parse HEAD)"

# The isolated profile disables HDMI-A-1, so exactly one output stays
# enabled however many connectors are plugged in. Both connectors the
# profile reasons about must exist on the renderD128 card (card0).
for connector in card0-DP-1 card0-HDMI-A-1; do
    [[ -e "/sys/class/drm/$connector/status" ]] || {
        echo "expected connector $connector not present on card0" >&2
        exit 2; }
done

[[ ! -e "$EVIDENCE_DIR" ]] || { echo "Evidence directory already exists; refusing to overwrite it" >&2; exit 2; }
mkdir -p "$(dirname "$EVIDENCE_DIR")"
mkdir -m 700 "$EVIDENCE_DIR"

LOM_CONFIG="${SOPHIA_LOM_CONFIG:?SOPHIA_LOM_CONFIG must name the Lom config file}"
[[ -f "$LOM_CONFIG" ]] || { echo "SOPHIA_LOM_CONFIG is not a file" >&2; exit 2; }
# DMA is selected by Lom's protected panel config (CLI override stays
# unused here): refuse a config that would silently run owned-ring.
grep -Eq 'image-source[[:space:]]+"dma-buf"' "$LOM_CONFIG" || {
    echo "SOPHIA_LOM_CONFIG does not select panel image-source \"dma-buf\"" >&2
    exit 2; }
LOM_CORE_CONFIG="$ROOT_DIR/tools/fixtures/lom_panel_core.kdl"
cp "$LOM_CORE_CONFIG" "$EVIDENCE_DIR/core.kdl"
LOM_CORE_CONFIG="$EVIDENCE_DIR/core.kdl"

# Isolated one-output profile generated from the frozen template:
# the real pinned Lom ELF as the panel component (DMA selected by its
# protected config), HDMI-A-1 disabled so one output remains enabled.
sed -e "s|@LOM_BIN@|$SOPHIA_LOM_BIN|g" -e "s|@LOM_CONFIG@|$LOM_CONFIG|g" \
    "$ROOT_DIR/tools/fixtures/t339_capture_desktop.kdl.tmpl" \
    > "$EVIDENCE_DIR/desktop.kdl"
"$SOPHIA_BIN" config check --desktop-profile="$EVIDENCE_DIR/desktop.kdl"

{
    printf 'gate=t339-capture-qualification\n'
    printf 'integration_commit=%s\n' "$INTEGRATION_COMMIT"
    printf 'sophia_commit=%s\n' "$SOPHIA_COMMIT"
    printf 'sophia_binary=%s\nsophia_binary_sha256=%s\n' "$SOPHIA_BIN" "$SOPHIA_SHA256"
    printf 'integration_xtask=%s\nintegration_xtask_sha256=%s\n' \
        "$SOPHIA_INTEGRATION_XTASK" "$(sha256sum "$SOPHIA_INTEGRATION_XTASK" | cut -d' ' -f1)"
    printf 'session_preflight=%s\nsession_preflight_sha256=%s\n' \
        "$SOPHIA_SESSION_PREFLIGHT" "$(sha256sum "$SOPHIA_SESSION_PREFLIGHT" | cut -d' ' -f1)"
    printf 'lom_commit=%s\nlom_binary=%s\nlom_binary_sha256=%s\n' \
        "$SOPHIA_LOM_COMMIT" "$SOPHIA_LOM_BIN" "$SOPHIA_LOM_SHA256"
    printf 'lom_config=%s\nlom_config_sha256=%s\n' \
        "$LOM_CONFIG" "$(sha256sum "$LOM_CONFIG" | cut -d' ' -f1)"
    printf 'profile_template_sha256=%s\n' "$(sha256sum "$ROOT_DIR/tools/fixtures/t339_capture_desktop.kdl.tmpl" | cut -d' ' -f1)"
    printf 'hagia_binary=%s\nhagia_binary_sha256=%s\n' \
        "$SOPHIA_HAGIA_BIN" "$(sha256sum "$SOPHIA_HAGIA_BIN" | cut -d' ' -f1)"
    printf 'factotum_agent=%s\nfactotum_agent_sha256=%s\n' "$SOPHIA_FACTOTUM_AGENT" "$(sha256sum "$SOPHIA_FACTOTUM_AGENT" | cut -d' ' -f1)"
    printf 'factotum_pam_helper=%s\nfactotum_pam_helper_sha256=%s\n' "$SOPHIA_FACTOTUM_PAM_HELPER" "$(sha256sum "$SOPHIA_FACTOTUM_PAM_HELPER" | cut -d' ' -f1)"
    printf 'core_config_sha256=%s\n' "$(sha256sum "$LOM_CORE_CONFIG" | cut -d' ' -f1)"
    printf 'desktop_profile_sha256=%s\n' "$(sha256sum "$EVIDENCE_DIR/desktop.kdl" | cut -d' ' -f1)"
    printf 'render_node=%s\nenabled_outputs=1 (HDMI-A-1 disabled in profile)\n' "$RENDER_NODE"
    printf 'native_runtime_msec=90000\nwatchdog_seconds=110\n'
} > "$EVIDENCE_DIR/identity.manifest"

verify_candidate_inputs() {
    [[ "$(sha256sum "$SOPHIA_BIN" | cut -d' ' -f1)" == "$SOPHIA_SHA256" ]]
    [[ "$(sha256sum "$SOPHIA_LOM_BIN" | cut -d' ' -f1)" == "$SOPHIA_LOM_SHA256" ]]
    [[ "$(git -C "$ROOT_DIR" rev-parse HEAD)" == "$INTEGRATION_COMMIT" ]]
    [[ "$(git -C "$SOPHIA_SOURCE" rev-parse HEAD)" == "$SOPHIA_COMMIT" ]]
}

echo "Evidence: $EVIDENCE_DIR"
verify_candidate_inputs
cat <<'INSTRUCTIONS'
t339 capture qualification: the session ends normally after 90 seconds;
the 110-second watchdog is failure recovery only. One output on
renderD128; HDMI-A-1 is disabled by the generated profile.
Confirm the Lom bar appears and its clock keeps ticking until
automatic exit. Do not click anything; no interaction is required.
INSTRUCTIONS

set +e
SOPHIA_ROOT="$SOPHIA_SOURCE" \
SOPHIA_BIN="$SOPHIA_BIN" \
SOPHIA_SESSION_PREFLIGHT="$SOPHIA_SESSION_PREFLIGHT" \
SOPHIA_INTEGRATION_XTASK="$SOPHIA_INTEGRATION_XTASK" \
SOPHIA_SESSION_TTY=/dev/tty4 \
SOPHIA_DESKTOP_ADAPTER_LOG="$EVIDENCE_DIR/adapter.log" \
SOPHIA_HAGIA_BIN="$SOPHIA_HAGIA_BIN" \
SOPHIA_HAGIA_SHELL_BIN="$SOPHIA_LOM_BIN" \
SOPHIA_SHELL_CONFIG="$LOM_CONFIG" \
SOPHIA_BUILD_SESSION=false \
SOPHIA_MANAGE_KEYD=true \
SOPHIA_REQUIRE_LOCAL_VT=true \
SOPHIA_TTY_PROFILE=hagia \
SOPHIA_CORE_CONFIG="$LOM_CORE_CONFIG" \
SOPHIA_DESKTOP_PROFILE="$EVIDENCE_DIR/desktop.kdl" \
SOPHIA_SESSION_STARTUP=none \
SOPHIA_SESSION_WATCHDOG_SECONDS=110 \
SOPHIA_DIAGNOSTIC_DIR="$EVIDENCE_DIR/session" \
SOPHIA_UNTRUSTED_SESSION_OUTPUT_LOG="$EVIDENCE_DIR/session/untrusted-session-output.log" \
    env -u SOPHIA_FACTOTUM_AGENT -u SOPHIA_FACTOTUM_PAM_HELPER \
    "$ROOT_DIR/tools/session/run_desktop_session.sh" --max-runtime-ms=90000 \
    --wm-process="$SOPHIA_HAGIA_BIN" \
    --factotum-agent="$SOPHIA_FACTOTUM_AGENT" \
    --factotum-pam-helper="$SOPHIA_FACTOTUM_PAM_HELPER"
native_status=$?
set -e
printf 'native_exit_status=%s\n' "$native_status" > "$EVIDENCE_DIR/native-outcome.txt"
if [[ "$native_status" -ne 0 ]]; then
    echo "t339 capture session ended unexpectedly with status $native_status" >&2
    exit 1
fi
verify_candidate_inputs
grep -q '^sophia_tty_recovery schema=3 .*termios_restored=true ' \
    "$EVIDENCE_DIR/session/recovery.log" \
    && grep -q '^sophia_tty_recovery_verification schema=1 .*keyd_restored=true$' \
        "$EVIDENCE_DIR/session/recovery.log" || {
    echo "Session did not prove complete TTY and keyd recovery" >&2
    exit 1
}

# Capture/presentation observability: exact identity join, never a
# substring count. Lom emits per-candidate
#   lom_panel_candidate schema=2 status=presented connection_epoch=
#   content_grant_epoch= output= candidate_generation= ...
#   checksum=unavailable (the pinned DMA route carries no CPU checksum)
# and the host emits per-candidate
#   sophia_live_shell_content schema=1 status=presented output=
#   candidate_generation= presentation_epoch= ... connection_epoch=
#   content_grant_epoch=
# A candidate counts only when both sides agree on
# (connection_epoch, content_grant_epoch, output, candidate_generation).
# Direct mask6/ImageBegin/SourceReleased records are not in these logs:
# the DMA route itself is an inference from the pinned Lom build (its
# code issues Demand only after Accepted+CaptureComplete), and the
# outcome file labels it as such instead of claiming direct evidence.
output_log="$EVIDENCE_DIR/session/untrusted-session-output.log"
host_join="$EVIDENCE_DIR/host-presented.join"
lom_join="$EVIDENCE_DIR/lom-dma-candidates.join"
sed -n 's/^sophia_live_shell_content schema=1 status=presented output=\([0-9]*\) candidate_generation=\([0-9]*\) presentation_epoch=[0-9]* staging_bytes=[0-9]* resident_bytes=[0-9]* retiring_bytes=[0-9]* backing_bytes=[0-9]* connection_epoch=\([0-9]*\) content_grant_epoch=\([0-9]*\)$/\3 \4 \1 \2/p' \
    "$output_log" | sort -u > "$host_join"
sed -n 's/^lom_panel_candidate schema=2 status=presented connection_epoch=\([0-9]*\) content_grant_epoch=\([0-9]*\) output=\([0-9]*\) candidate_generation=\([0-9]*\) .* checksum=unavailable .*$/\1 \2 \3 \4/p' \
    "$output_log" | sort -u > "$lom_join"
host_presented="$(wc -l < "$host_join")"
lom_dma_candidates="$(wc -l < "$lom_join")"
joined="$(comm -12 "$host_join" "$lom_join" | wc -l)"
{
    printf 'host_presented_candidates=%s\n' "$host_presented"
    printf 'lom_dma_candidates=%s\n' "$lom_dma_candidates"
    printf 'joined_dma_presented=%s\n' "$joined"
    printf 'dma_route_evidence=inferred-from-pinned-lom (Demand only after Accepted+CaptureComplete); mask6/ImageBegin/SourceReleased not directly logged\n'
} >> "$EVIDENCE_DIR/native-outcome.txt"
if [[ "$joined" -lt 1 ]]; then
    echo "No joined DMA candidate was presented by the host" >&2
    exit 1
fi
echo "t339 capture qualification: joined=$joined DMA candidates presented, TTY/keyd recovery proven"
