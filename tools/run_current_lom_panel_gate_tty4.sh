#!/usr/bin/env bash
# Provenance: moved from Sophia tools/run_current_lom_panel_gate_tty4.sh at 9fcaec782ce4fe9978568c0466ee17a78b3d4571 (Sophia rule 13).
# Changes: every input is explicit. SOPHIA_SOURCE is a clean checkout whose
# HEAD is exactly the signed revision in pins/sophia.toml; the gate stages that
# revision's exact tree (git archive, tree hash proven) into the private
# SOPHIA_GATE_BUILD_DIR and reads or executes Sophia files only from there.
# Lom, Hagia, Bemenu and Provlita are prepared artifacts (cargo xtask
# prepare-product-artifact / prepare-bemenu-artifact) bound to the operator's
# SOPHIA_<PRODUCT>_COMMIT and SOPHIA_<PRODUCT>_SHA256 (and _CONFIG_SHA256).
# Every build writes only below SOPHIA_GATE_BUILD_DIR; no source checkout,
# /home default or sibling path is built or read.
set -euo pipefail

# Generated profiles must satisfy the configuration reader's ownership policy,
# independently of the operator's inherited (possibly group-writable) umask.
umask 077
GATE_MODE=panel
if [[ "${1:-}" == launcher || "${1:-}" == dock ]]; then GATE_MODE="$1"; shift; fi
[[ $# -eq 0 ]] || { echo "usage: lom-test [launcher|dock]" >&2; exit 2; }

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=tools/lib/artifacts.sh
. "$ROOT_DIR/tools/lib/artifacts.sh"
required=(SOPHIA_SOURCE SOPHIA_GATE_BUILD_DIR
    SOPHIA_LOM_ARTIFACT SOPHIA_LOM_COMMIT SOPHIA_LOM_SHA256 SOPHIA_LOM_CONFIG_SHA256
    SOPHIA_HAGIA_ARTIFACT SOPHIA_HAGIA_COMMIT SOPHIA_HAGIA_SHA256)
[[ "$GATE_MODE" == panel ]] || required+=(SOPHIA_BEMENU_ARTIFACT SOPHIA_BEMENU_COMMIT SOPHIA_BEMENU_SHA256)
[[ "$GATE_MODE" != dock ]] || required+=(SOPHIA_PROVLITA_ARTIFACT SOPHIA_PROVLITA_COMMIT
    SOPHIA_PROVLITA_SHA256 SOPHIA_PROVLITA_CONFIG_SHA256)
WORKLOAD_BUDGETS="$ROOT_DIR/tools/fixtures/lom_workload_budgets.json"
EVIDENCE_DIR="${SOPHIA_LOM_NATIVE_EVIDENCE_DIR:-$ROOT_DIR/.artifacts/lom-panel-native/$(date -u +%Y%m%dT%H%M%SZ)}"

[[ "$(tty)" == /dev/tty4 ]] || { echo "Run this from /dev/tty4 after ending the graphical session." >&2; exit 2; }
[[ "${SOPHIA_LOM_NATIVE_GATE_ARM:-0}" == 1 ]] || { echo "Set SOPHIA_LOM_NATIVE_GATE_ARM=1 to run the native panel gate." >&2; exit 2; }
for name in "${required[@]}"; do
    [[ -n "${!name:-}" ]] || { echo "$name is required (no default)" >&2; exit 2; }
done
[[ -z "$(git -C "$ROOT_DIR" status --short)" ]] || { echo "Integration source must be clean" >&2; exit 2; }
git -C "$ROOT_DIR" verify-commit HEAD >/dev/null
check_sophia_source "$SOPHIA_SOURCE"
check_build_dir "$SOPHIA_GATE_BUILD_DIR"
INTEGRATION_COMMIT="$(git -C "$ROOT_DIR" rev-parse HEAD)"
SOPHIA_COMMIT="$(pinned_sophia_rev)"
[[ ! -e "$EVIDENCE_DIR" ]] || { echo "Evidence directory already exists; refusing to overwrite it" >&2; exit 2; }
mkdir -p "$(dirname "$EVIDENCE_DIR")"
mkdir -m 700 "$EVIDENCE_DIR"
# Every build output and every staged Sophia file lives below this directory.
BUILD_DIR="$(realpath -- "$SOPHIA_GATE_BUILD_DIR")"
SOPHIA_TREE="$BUILD_DIR/sophia-tree"
SOPHIA_TARGET="$BUILD_DIR/sophia-target"
INTEGRATION_TARGET="$BUILD_DIR/integration-target"
stage_sophia_tree "$SOPHIA_SOURCE" "$BUILD_DIR" "$SOPHIA_TREE"
if [[ "$GATE_MODE" == panel ]]; then
    default_core="$ROOT_DIR/tools/fixtures/lom_panel_core.kdl"
else
    default_core="$SOPHIA_TREE/tools/fixtures/native_launcher_core.kdl"
fi
LOM_CORE_CONFIG="${SOPHIA_LOM_CORE_CONFIG:-$default_core}"
LOM_BIN="$EVIDENCE_DIR/lom"
LOM_CONFIG="$EVIDENCE_DIR/lom-config.kdl"
HAGIA_BIN="$EVIDENCE_DIR/hagia"
BEMENU_BIN="$EVIDENCE_DIR/bemenu-sophia"
PROVLITA_BIN="$EVIDENCE_DIR/provlita"
PROVLITA_CONFIG="$EVIDENCE_DIR/provlita-config.kdl"
load_artifact lom "$SOPHIA_LOM_ARTIFACT" "$SOPHIA_LOM_COMMIT" "$SOPHIA_LOM_SHA256" "$LOM_BIN" \
    "$SOPHIA_LOM_CONFIG_SHA256" "$LOM_CONFIG"
load_artifact hagia "$SOPHIA_HAGIA_ARTIFACT" "$SOPHIA_HAGIA_COMMIT" "$SOPHIA_HAGIA_SHA256" "$HAGIA_BIN"
[[ "$GATE_MODE" == panel ]] || load_artifact bemenu "$SOPHIA_BEMENU_ARTIFACT" "$SOPHIA_BEMENU_COMMIT" \
    "$SOPHIA_BEMENU_SHA256" "$BEMENU_BIN"
[[ "$GATE_MODE" != dock ]] || load_artifact provlita "$SOPHIA_PROVLITA_ARTIFACT" "$SOPHIA_PROVLITA_COMMIT" \
    "$SOPHIA_PROVLITA_SHA256" "$PROVLITA_BIN" "$SOPHIA_PROVLITA_CONFIG_SHA256" "$PROVLITA_CONFIG"
cp "$WORKLOAD_BUDGETS" "$EVIDENCE_DIR/workload-budgets.json"
cp "$LOM_CORE_CONFIG" "$EVIDENCE_DIR/core.kdl"
cp "$ROOT_DIR/tools/fixtures/lom_panel_desktop.kdl" "$EVIDENCE_DIR/probe-overrides.kdl"
LOM_CORE_CONFIG="$EVIDENCE_DIR/core.kdl"
python3 - "$ROOT_DIR/tools/probes/lom_workload" "$EVIDENCE_DIR/workload-budgets.json" <<'PY'
import json, sys
sys.path.insert(0, sys.argv[1])
from verify import budgets, unique_json_object
with open(sys.argv[2], encoding="utf-8") as source:
    budgets(json.load(source, object_pairs_hook=unique_json_object))
PY
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR="$SOPHIA_TARGET" nice -n 19 cargo build --locked --offline --release \
    -p sophia-cli --features native-session --manifest-path "$SOPHIA_TREE/Cargo.toml"
SOPHIA_BIN="$SOPHIA_TARGET/release/sophia"
if [[ "$GATE_MODE" == launcher ]]; then
    python3 "$ROOT_DIR/tools/probes/native_launcher/profile.py" \
        --lom "$LOM_BIN" --config "$LOM_CONFIG" --bemenu "$BEMENU_BIN" \
        > "$EVIDENCE_DIR/probe-overrides.kdl"
fi
XTASK_BIN="$INTEGRATION_TARGET/release/xtask"
if [[ "$GATE_MODE" == dock ]]; then
    # This repository builds offline from its accepted private CARGO_HOME,
    # which the marker names (outside every source tree).
    [[ -f "$ROOT_DIR/.provision/accepted" ]] || { echo "Run tools/provision.sh first" >&2; exit 2; }
    provisioned_home=$(sed -n 's/^cargo_home=\(\/.*\)$/\1/p' "$ROOT_DIR/.provision/accepted")
    [[ -n "$provisioned_home" ]] || { echo "Re-run tools/provision.sh (marker has no cargo_home)" >&2; exit 2; }
    (export CARGO_HOME="$provisioned_home"
        CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR="$INTEGRATION_TARGET" nice -n 19 cargo build --locked --offline \
            --release -p xtask --manifest-path "$ROOT_DIR/Cargo.toml")
    "$XTASK_BIN" dock profile "$LOM_BIN" "$LOM_CONFIG" "$BEMENU_BIN" "$PROVLITA_BIN" "$PROVLITA_CONFIG" \
        > "$EVIDENCE_DIR/probe-overrides.kdl"
fi
wm_profile="${SOPHIA_DESKTOP_PROFILE:-}"
if [[ -z "$wm_profile" ]]; then
    config_home="${XDG_CONFIG_HOME:-$HOME/.config}"
    for candidate in "$config_home/sophia/desktop.kdl" "$config_home/hagia/config.kdl" \
        /etc/sophia/desktop.kdl /etc/hagia/config.kdl; do
        if [[ -e "$candidate" || -L "$candidate" ]]; then
            wm_profile="$candidate"
            break
        fi
    done
fi
[[ "$wm_profile" == /* && -f "$wm_profile" ]] || {
    echo "Select an existing absolute WM profile with SOPHIA_DESKTOP_PROFILE." >&2
    exit 2
}
# Expand includes through the real parser and preserve all configured bindings
# and application declarations. Only the recorded probe overrides differ.
"$SOPHIA_BIN" config print-effective --desktop-profile="$wm_profile" \
    > "$EVIDENCE_DIR/wm-profile.kdl"
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR="$SOPHIA_TARGET" nice -n 19 cargo build --locked --offline --release \
    -p sophia-config --example desktop_profile_probe --manifest-path "$SOPHIA_TREE/Cargo.toml"
probe_args=()
[[ "$GATE_MODE" == panel ]] || probe_args+=(--require-launcher-binding)
"$SOPHIA_TARGET/release/examples/desktop_profile_probe" \
    "$EVIDENCE_DIR/wm-profile.kdl" "$EVIDENCE_DIR/probe-overrides.kdl" "${probe_args[@]}" \
    > "$EVIDENCE_DIR/desktop.kdl"
"$SOPHIA_BIN" config check --desktop-profile="$EVIDENCE_DIR/desktop.kdl"
{
    printf 'gate_mode=%s\n' "$GATE_MODE"
    if [[ "$GATE_MODE" != panel ]]; then
        printf 'bemenu_commit=%s\n' "$SOPHIA_BEMENU_COMMIT"
        printf 'bemenu_binary_sha256=%s\n' "$(sha256sum "$BEMENU_BIN" | cut -d' ' -f1)"
    fi
    if [[ "$GATE_MODE" == dock ]]; then
        printf 'provlita_commit=%s\n' "$SOPHIA_PROVLITA_COMMIT"
        printf 'provlita_binary_sha256=%s\n' "$(sha256sum "$PROVLITA_BIN" | cut -d' ' -f1)"
        printf 'provlita_config_sha256=%s\n' "$(sha256sum "$PROVLITA_CONFIG" | cut -d' ' -f1)"
        printf 'scope=three-component-smoke\nlatency_acceptance=NOT_RUN\nrestart_acceptance=NOT_RUN\n'
    fi
    printf 'integration_commit=%s\n' "$INTEGRATION_COMMIT"
    printf 'sophia_commit=%s\n' "$SOPHIA_COMMIT"
    printf 'sophia_binary_sha256=%s\n' "$(sha256sum "$SOPHIA_BIN" | cut -d' ' -f1)"
    printf 'lom_commit=%s\n' "$SOPHIA_LOM_COMMIT"
    printf 'lom_binary_sha256=%s\n' "$(sha256sum "$LOM_BIN" | cut -d' ' -f1)"
    printf 'lom_config_sha256=%s\n' "$(sha256sum "$LOM_CONFIG" | cut -d' ' -f1)"
    printf 'hagia_commit=%s\n' "$SOPHIA_HAGIA_COMMIT"
    printf 'hagia_binary_sha256=%s\n' "$(sha256sum "$HAGIA_BIN" | cut -d' ' -f1)"
    printf 'workload_budgets_sha256=%s\n' "$(sha256sum "$EVIDENCE_DIR/workload-budgets.json" | cut -d' ' -f1)"
    printf 'core_config_sha256=%s\n' "$(sha256sum "$LOM_CORE_CONFIG" | cut -d' ' -f1)"
    printf 'desktop_profile_sha256=%s\n' "$(sha256sum "$EVIDENCE_DIR/desktop.kdl" | cut -d' ' -f1)"
    printf 'wm_profile_sha256=%s\n' "$(sha256sum "$EVIDENCE_DIR/wm-profile.kdl" | cut -d' ' -f1)"
    printf 'probe_overrides_sha256=%s\n' "$(sha256sum "$EVIDENCE_DIR/probe-overrides.kdl" | cut -d' ' -f1)"
    printf 'native_runtime_msec=90000\nwatchdog_seconds=110\n'
} > "$EVIDENCE_DIR/identity.manifest"
sha256sum "$SOPHIA_BIN" "$LOM_BIN" "$HAGIA_BIN" "$LOM_CONFIG" "$LOM_CORE_CONFIG" \
    "$EVIDENCE_DIR/desktop.kdl" "$EVIDENCE_DIR/wm-profile.kdl" \
    "$EVIDENCE_DIR/probe-overrides.kdl" "$EVIDENCE_DIR/workload-budgets.json" > "$EVIDENCE_DIR/inputs.sha256"
if [[ "$GATE_MODE" != panel ]]; then sha256sum "$BEMENU_BIN" >> "$EVIDENCE_DIR/inputs.sha256"; fi
if [[ "$GATE_MODE" == dock ]]; then sha256sum "$PROVLITA_BIN" "$PROVLITA_CONFIG" "$XTASK_BIN" >> "$EVIDENCE_DIR/inputs.sha256"; fi
verify_candidate_inputs() {
    sha256sum --check --status "$EVIDENCE_DIR/inputs.sha256"
    [[ "integration_commit=$(git -C "$ROOT_DIR" rev-parse HEAD)" == "$(sed -n '/^integration_commit=/p' "$EVIDENCE_DIR/identity.manifest")" ]]
    [[ "sophia_commit=$(git -C "$SOPHIA_SOURCE" rev-parse HEAD)" == "$(sed -n '/^sophia_commit=/p' "$EVIDENCE_DIR/identity.manifest")" ]]
    verify_staged_tree "$SOPHIA_SOURCE" "$SOPHIA_TREE"
    [[ -z "$(git -C "$ROOT_DIR" status --short)" && -z "$(git -C "$SOPHIA_SOURCE" status --short)" ]]
}

echo "Evidence: $EVIDENCE_DIR"
verify_candidate_inputs
echo "Checking Lom's protected GPU and content path before graphics takeover."
SOPHIA_LOM_GPU_PROOF_ARM=1 \
SOPHIA_LOM_GPU_EVIDENCE_DIR="$EVIDENCE_DIR/gpu-content" \
SOPHIA_SOURCE="$SOPHIA_SOURCE" \
SOPHIA_GATE_BUILD_DIR="$BUILD_DIR" \
SOPHIA_LOM_ARTIFACT="$SOPHIA_LOM_ARTIFACT" \
SOPHIA_LOM_COMMIT="$SOPHIA_LOM_COMMIT" \
SOPHIA_LOM_SHA256="$SOPHIA_LOM_SHA256" \
SOPHIA_LOM_CONFIG_SHA256="$SOPHIA_LOM_CONFIG_SHA256" \
SOPHIA_LOM_GPU_RENDER_NODE="${SOPHIA_LOM_GPU_RENDER_NODE:-/dev/dri/renderD128}" \
    "$ROOT_DIR/tools/lom_gpu_content_hardware_proof.sh"

verify_candidate_inputs
if [[ "$GATE_MODE" == panel ]]; then
cat <<'INSTRUCTIONS'
The session ends normally after 90 seconds; the 110-second watchdog is failure recovery only.
Confirm a bar and moving clock on BOTH outputs, then wait ten clock ticks.
Click an INACTIVE workspace number 20 times on EACH bar (40 clicks total).
Alternate the two outputs and two INACTIVE numbers from each bar; finish the clicks within 60 seconds.
Do not click during warmup or after the 40 clicks; let the clocks run until automatic exit.
ACK limits: p95 50ms / maximum 100ms. Native limits: p95 150ms / maximum 300ms.
Missing actions, stale/no-op clicks, restarts, timeouts and retained shutdown credits fail.
INSTRUCTIONS
elif [[ "$GATE_MODE" == launcher ]]; then
cat <<'INSTRUCTIONS'
Launcher smoke: the session ends after 90 seconds; the watchdog is failure recovery only.
Confirm Lom bars and clocks on both monitors. Use your WM application-launcher binding
(the selected operator profile binds Super+Space). Move the pointer to each monitor before opening its menu;
type a query, dismiss with Escape, reopen and confirm the query resets. Check that
the other bar keeps updating and workspace switching still works. Finally search
for terminal, select the entry named terminal, and press Enter once to launch it. Close the terminal, dismiss any menu,
and wait for automatic exit. Record placement, focus restoration, mouse dismissal,
query/reset and both-monitor observations separately. This is not the 40-action
panel latency workload or complete native acceptance.
INSTRUCTIONS
else
cat <<'INSTRUCTIONS'
Three-component smoke: automatic exit after 90 seconds; watchdog at 110 seconds.
On BOTH monitors confirm Lom above and Provlita below, without overlap. The Terminal
tile is enabled; Browser/Files are deliberately unavailable unless in your catalog.
Move to the OTHER monitor without switching desktops, then click Terminal on its dock.
Confirm the window opens on the clicked monitor's current workspace. Repeat on the other dock;
type 'exit' and Enter before the next launch.
On EACH monitor open Bemenu using your WM binding (Super+Space in the selected profile),
type terminal, launch that entry once, then type 'exit' and Enter. Also reopen and dismiss with Escape.
Check clocks and workspace switching continue on both bars, and dock clicks do not
take keyboard focus. Four terminal launches total. Let the session exit automatically.
This smoke checks logical launch-to-placement identity, not latency or component restarts.
Record physical monitor placement, focus and visual observations separately.
INSTRUCTIONS
fi
shell_args=()
[[ "$GATE_MODE" != panel ]] || shell_args+=("--shell-process=$LOM_BIN")
set +e
SOPHIA_BIN="$SOPHIA_BIN" \
SOPHIA_HAGIA_BIN="$HAGIA_BIN" \
SOPHIA_HAGIA_SHELL_BIN="$LOM_BIN" \
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
    "$SOPHIA_TREE/tools/run_sophia_session.sh" --max-runtime-ms=90000 "${shell_args[@]}" --wm-process="$HAGIA_BIN"
native_status=$?
set -e
printf 'native_exit_status=%s\n' "$native_status" > "$EVIDENCE_DIR/native-outcome.txt"
if [[ "$native_status" -ne 0 ]]; then
    echo "Native panel session ended unexpectedly with status $native_status" >&2
    exit 1
fi
verify_candidate_inputs
grep -q '^sophia_tty_recovery schema=3 .*termios_restored=true ' \
    "$EVIDENCE_DIR/session/recovery.log" \
    && grep -q '^sophia_tty_recovery_verification schema=1 .*keyd_restored=true$' \
        "$EVIDENCE_DIR/session/recovery.log" || {
    echo "Native panel session did not prove complete TTY and keyd recovery" >&2
    exit 1
}

if [[ "$GATE_MODE" == dock ]]; then
    "$XTASK_BIN" dock verify "$EVIDENCE_DIR/session/events.0.log" \
        | tee "$EVIDENCE_DIR/dock-verification.log"
    echo "Dock smoke transcript passed; visual acceptance remains operator evidence: $EVIDENCE_DIR"
    exit 0
fi
if [[ "$GATE_MODE" == launcher ]]; then
    python3 "$ROOT_DIR/tools/probes/native_launcher/verify.py" "$EVIDENCE_DIR/session/events.0.log" \
        | tee "$EVIDENCE_DIR/launcher-verification.json"
    echo "Launcher transcript passed; visual/focus/placement acceptance remains operator evidence: $EVIDENCE_DIR"
    exit 0
fi

"$ROOT_DIR/tools/verify_lom_panel_native_gate.sh" "$EVIDENCE_DIR/session/events.0.log" \
    | tee "$EVIDENCE_DIR/verification.log"
python3 "$ROOT_DIR/tools/probes/lom_workload/verify.py" \
    --host "$EVIDENCE_DIR/session/events.0.log" \
    --client "$EVIDENCE_DIR/session/untrusted-session-output.log" \
    --budgets "$EVIDENCE_DIR/workload-budgets.json" \
    | tee "$EVIDENCE_DIR/workload-verification.json"
echo "Workload evidence passed. Record your visual/placement observations separately; see $EVIDENCE_DIR"
