#!/usr/bin/env bash
# Provenance: moved from Sophia tools/check_lom_gpu_content_proof_verifiers.sh at 9fcaec782ce4fe9978568c0466ee17a78b3d4571 (Sophia rule 13).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cat > "$work/gpu.log" <<'EOF'
lom_shell_transport schema=1 wire=9p2000.L revision=6 epoch=1
lom_gpu_admission schema=2 status=ready grant_epoch=1 render_node=/dev/dri/renderD128 device_major=226 device_minor=128 selection_method=drm_dev_t adapter_render_major=226 adapter_render_minor=128 adapter_has_render=true pci_bus_id=0000:01:00.0 pci_vendor_id=1002 pci_device_id=744c backend=Vulkan device_type=DiscreteGpu adapter_name="fixture" driver="fixture" visible_dri_entries=renderD128
sophia_shell_gpu_content_render schema=1 index=1 generation=3 bytes=24576 checksum=0123456789abcdef outcome=presented_synthetic
sophia_shell_gpu_content_render schema=1 index=2 generation=4 bytes=24576 checksum=fedcba9876543210 outcome=renderer_failed
sophia_shell_gpu_content_proof schema=1 status=complete protected=true wire=9p2000.L revision=6 capabilities=0x783 grant_epoch=1 render_node=/dev/dri/renderD128 device_major=226 device_minor=128 pci_bus_id=0000:01:00.0 output_width=256 output_height=64 edge=top width=256 height=24 renders=2 pixels=full_surface_raster discrete_input=true end=client_exits backing_bytes=0 native_presentation=false
EOF
"$ROOT_DIR/tools/verify_lom_gpu_content_hardware_proof.sh" "$work/gpu.log" >/dev/null
proof='/^sophia_shell_gpu_content_proof /'
render1='/^sophia_shell_gpu_content_render schema=1 index=1 /'
render2='/^sophia_shell_gpu_content_render schema=1 index=2 /'
gpu_mutations=(
    extra_drm cpu zero_checksum native identity adapter_identity method has_render
    missing_identity missing_backend missing_device_type duplicate_epoch malformed_major
    overflow_major zero_epoch non_vulkan missing_content_input_capability wrong_renderer_outcome
    one_render missing_second_checksum claimed_native_first
    contract_pixels stop_client input_denied bottom_edge other_output wide_surface
    thick_surface over_coverage wrong_bytes regressed_generation reordered_render
    extra_render backing_retained old_record
    missing_transport duplicate_transport wrong_wire wrong_host_wire wrong_transport_epoch late_transport natural_ipc)
for mutation in "${gpu_mutations[@]}"; do
    cp "$work/gpu.log" "$work/$mutation.log"
    case "$mutation" in
        missing_transport) sed -i '/^lom_shell_transport /d' "$work/$mutation.log" ;;
        duplicate_transport) sed -n '/^lom_shell_transport /p' "$work/gpu.log" >> "$work/$mutation.log" ;;
        wrong_wire) sed -i '/^lom_shell_transport /s/wire=9p2000.L/wire=current-ipc/' "$work/$mutation.log" ;;
        wrong_host_wire) sed -i "${proof}s/wire=9p2000.L/wire=current-ipc/" "$work/$mutation.log" ;;
        wrong_transport_epoch) sed -i '/^lom_shell_transport /s/epoch=1/epoch=2/' "$work/$mutation.log" ;;
        late_transport) sed -i '/^lom_shell_transport /{h;d;}; /^lom_gpu_admission /G' "$work/$mutation.log" ;;
        natural_ipc) sed -i 's/wire=9p2000.L/wire=current-ipc/g' "$work/$mutation.log" ;;
        extra_drm) sed -i 's/visible_dri_entries=renderD128/visible_dri_entries=card0,renderD128/' "$work/$mutation.log" ;;
        cpu) sed -i 's/device_type=DiscreteGpu/device_type=Cpu/' "$work/$mutation.log" ;;
        zero_checksum) sed -i 's/checksum=0123456789abcdef/checksum=0000000000000000/' "$work/$mutation.log" ;;
        native) sed -i 's/native_presentation=false/native_presentation=true/' "$work/$mutation.log" ;;
        identity) sed -i '/^lom_gpu_admission /s/device_minor=128/device_minor=129/' "$work/$mutation.log" ;;
        adapter_identity) sed -i '/^lom_gpu_admission /s/adapter_render_minor=128/adapter_render_minor=129/' "$work/$mutation.log" ;;
        method) sed -i '/^lom_gpu_admission /s/selection_method=drm_dev_t/selection_method=pci/' "$work/$mutation.log" ;;
        has_render) sed -i '/^lom_gpu_admission /s/adapter_has_render=true/adapter_has_render=false/' "$work/$mutation.log" ;;
        missing_identity)
            sed -i 's/ device_major=226//g' "$work/$mutation.log"
            sed -i '/^lom_gpu_admission /s/ adapter_render_major=226//' "$work/$mutation.log"
            ;;
        missing_backend) sed -i '/^lom_gpu_admission /s/ backend=Vulkan//' "$work/$mutation.log" ;;
        missing_device_type) sed -i '/^lom_gpu_admission /s/ device_type=DiscreteGpu//' "$work/$mutation.log" ;;
        duplicate_epoch) sed -i '/^lom_gpu_admission /s/grant_epoch=1/grant_epoch=1 grant_epoch=1/' "$work/$mutation.log" ;;
        malformed_major)
            sed -i 's/device_major=226/device_major=invalid/g' "$work/$mutation.log"
            sed -i '/^lom_gpu_admission /s/adapter_render_major=226/adapter_render_major=invalid/' "$work/$mutation.log"
            ;;
        overflow_major)
            sed -i 's/device_major=226/device_major=4294967296/g' "$work/$mutation.log"
            sed -i '/^lom_gpu_admission /s/adapter_render_major=226/adapter_render_major=4294967296/' "$work/$mutation.log"
            ;;
        zero_epoch) sed -i 's/grant_epoch=1/grant_epoch=0/g' "$work/$mutation.log" ;;
        non_vulkan) sed -i '/^lom_gpu_admission /s/backend=Vulkan/backend=Gl/' "$work/$mutation.log" ;;
        missing_content_input_capability) sed -i 's/capabilities=0x783/capabilities=0x683/' "$work/$mutation.log" ;;
        wrong_renderer_outcome) sed -i "${render2}s/outcome=renderer_failed/outcome=presented_synthetic/" "$work/$mutation.log" ;;
        one_render)
            sed -i "${render2}d" "$work/$mutation.log"
            sed -i "${proof}s/renders=2/renders=1/" "$work/$mutation.log"
            ;;
        missing_second_checksum) sed -i "${render2}s/ checksum=fedcba9876543210//" "$work/$mutation.log" ;;
        claimed_native_first) sed -i "${render1}s/outcome=presented_synthetic/outcome=presented_native/" "$work/$mutation.log" ;;
        contract_pixels) sed -i "${proof}s/pixels=full_surface_raster/pixels=contract/" "$work/$mutation.log" ;;
        stop_client) sed -i "${proof}s/end=client_exits/end=stop_client/" "$work/$mutation.log" ;;
        input_denied) sed -i "${proof}s/discrete_input=true/discrete_input=false/" "$work/$mutation.log" ;;
        bottom_edge) sed -i "${proof}s/edge=top/edge=bottom/" "$work/$mutation.log" ;;
        other_output) sed -i "${proof}s/output_width=256/output_width=512/" "$work/$mutation.log" ;;
        wide_surface) sed -i "${proof}s/ width=256 / width=8193 /" "$work/$mutation.log" ;;
        thick_surface) sed -i "${proof}s/ height=24 / height=513 /" "$work/$mutation.log" ;;
        over_coverage) sed -i "${proof}s/output_height=64/output_height=40/" "$work/$mutation.log" ;;
        wrong_bytes) sed -i "${render1}s/bytes=24576/bytes=24575/" "$work/$mutation.log" ;;
        regressed_generation) sed -i "${render2}s/generation=4/generation=3/" "$work/$mutation.log" ;;
        reordered_render) sed -i "${render1}s/index=1/index=3/" "$work/$mutation.log" ;;
        extra_render) sed -n "${render2}p" "$work/gpu.log" >> "$work/$mutation.log" ;;
        backing_retained) sed -i "${proof}s/backing_bytes=0/backing_bytes=24576/" "$work/$mutation.log" ;;
        old_record) sed -i "${proof}s/^sophia_shell_gpu_content_proof schema=1/sophia_shell_gpu_content_hardware_proof schema=2/" "$work/$mutation.log" ;;
    esac
    if cmp -s "$work/gpu.log" "$work/$mutation.log"; then
        echo "mutation $mutation did not change the fixture" >&2
        exit 1
    fi
    if "$ROOT_DIR/tools/verify_lom_gpu_content_hardware_proof.sh" "$work/$mutation.log" >/dev/null 2>&1; then
        echo "verifier accepted $mutation mutation" >&2
        exit 1
    fi
done

domain_record='sophia_shell_gpu_domain schema=1 status=observed observation_id=0123456789abcdef0123456789abcdef grant_epoch=1 device_major=226 device_minor=128 dri_entries=1 device_inventory=bounded input_absent=true x11_socket_dir_absent=true user_runtime_dir_absent=true display_environment_absent=true inherited_devices=none inherited_sockets=none'
parent_record='sophia_shell_gpu_domain_parent schema=1 status=bound protected=true observation_id=0123456789abcdef0123456789abcdef grant_epoch=1 device_major=226 device_minor=128 peer_pid=42 supervisor_pid=41'
{ printf '%s\n' "$domain_record" "$parent_record"; cat "$work/gpu.log"; } > "$work/domain.log"
"$ROOT_DIR/tools/verify_lom_gpu_content_hardware_proof.sh" --require-domain "$work/domain.log" >/dev/null
if "$ROOT_DIR/tools/verify_lom_gpu_content_hardware_proof.sh" --require-domain "$work/gpu.log" >/dev/null 2>&1; then
    echo 'domain proof accepted a missing observation' >&2
    exit 1
fi
for mutation in epoch device input socket runtime environment fd duplicate missing parent nonce pid; do
    cp "$work/domain.log" "$work/domain-$mutation.log"
    case "$mutation" in
        epoch) sed -i '/^sophia_shell_gpu_domain /s/grant_epoch=1/grant_epoch=2/' "$work/domain-$mutation.log" ;;
        device) sed -i '/^sophia_shell_gpu_domain /s/device_minor=128/device_minor=129/' "$work/domain-$mutation.log" ;;
        input) sed -i 's/input_absent=true/input_absent=false/' "$work/domain-$mutation.log" ;;
        socket) sed -i 's/x11_socket_dir_absent=true/x11_socket_dir_absent=false/' "$work/domain-$mutation.log" ;;
        runtime) sed -i 's/user_runtime_dir_absent=true/user_runtime_dir_absent=false/' "$work/domain-$mutation.log" ;;
        environment) sed -i 's/display_environment_absent=true/display_environment_absent=false/' "$work/domain-$mutation.log" ;;
        fd) sed -i 's/inherited_sockets=none/inherited_sockets=present/' "$work/domain-$mutation.log" ;;
        duplicate) printf '%s\n' "$domain_record" >> "$work/domain-$mutation.log" ;;
        missing) sed -i 's/ inherited_devices=none//' "$work/domain-$mutation.log" ;;
        parent) sed -i '/^sophia_shell_gpu_domain_parent /d' "$work/domain-$mutation.log" ;;
        nonce) sed -i '/^sophia_shell_gpu_domain_parent /s/observation_id=[^ ]*/observation_id=1123456789abcdef0123456789abcdef/' "$work/domain-$mutation.log" ;;
        pid) sed -i 's/peer_pid=42/peer_pid=0/' "$work/domain-$mutation.log" ;;
    esac
    if "$ROOT_DIR/tools/verify_lom_gpu_content_hardware_proof.sh" --require-domain "$work/domain-$mutation.log" >/dev/null 2>&1; then
        echo "domain verifier accepted $mutation mutation" >&2
        exit 1
    fi
done

cat > "$work/native.log" <<'EOF'
sophia_live_shell_gpu schema=1 status=granted mode=direct peer_pid=42 grant_epoch=1 device_major=226 device_minor=128 pci_bus_id=0000:01:00.0
sophia_live_wm_configuration schema=2 status=committed catalog_generation=1 session_operation_count=7
sophia_live_shell_content schema=1 status=outputs facts_generation=1 outputs=1
sophia_live_shell_content schema=1 status=presented output=1 candidate_generation=1 presentation_epoch=11 staging_bytes=0 resident_bytes=24576 retiring_bytes=0 backing_bytes=24576
sophia_live_shell_content schema=1 status=presented output=1 candidate_generation=2 presentation_epoch=12 staging_bytes=0 resident_bytes=49152 retiring_bytes=24576 backing_bytes=49152
EOF
"$ROOT_DIR/tools/verify_lom_panel_native_gate.sh" "$work/native.log" >/dev/null
awk '{ printf "%d\t%d\t%d\t%s\n", NR, 1000 + NR, 2000 + NR, $0 }' \
    "$work/native.log" > "$work/native-events.log"
"$ROOT_DIR/tools/verify_lom_panel_native_gate.sh" "$work/native-events.log" >/dev/null
cp "$work/native.log" "$work/native-full.log"
sed -i '/candidate_generation=2/d' "$work/native.log"
if "$ROOT_DIR/tools/verify_lom_panel_native_gate.sh" "$work/native.log" >/dev/null 2>&1; then
    echo "native verifier accepted one generation" >&2
    exit 1
fi

sed 's/outputs=1/outputs=2/' "$work/native-full.log" > "$work/native-missing-output.log"
if "$ROOT_DIR/tools/verify_lom_panel_native_gate.sh" "$work/native-missing-output.log" >/dev/null 2>&1; then
    echo "native verifier accepted a missing output" >&2
    exit 1
fi

cp "$work/native-events.log" "$work/native-events-fatal.log"
printf '99\t9999\t9999\truntime_fatal phase=owner_loop\n' >> "$work/native-events-fatal.log"
if "$ROOT_DIR/tools/verify_lom_panel_native_gate.sh" "$work/native-events-fatal.log" >/dev/null 2>&1; then
    echo "native verifier accepted a fatal structured event" >&2
    exit 1
fi

cat "$work/native-events.log" "$work/native-events.log" > "$work/native-events-restarted.log"
if "$ROOT_DIR/tools/verify_lom_panel_native_gate.sh" "$work/native-events-restarted.log" >/dev/null 2>&1; then
    echo "native verifier accepted a restarted shell grant" >&2
    exit 1
fi

for failure in \
    'sophia_live_shell_gpu schema=1 status=revoked grant_epoch=1' \
    'sophia_live_shell_content schema=1 status=transport_failed' \
    'sophia_live_wm_configuration schema=2 status=rejected reason=unavailable_session_slot catalog_generation=1 missing_slot_count=1 missing_slots=7'; do
    cp "$work/native-events.log" "$work/native-events-lifecycle-failure.log"
    printf '100\t10000\t10000\t%s\n' "$failure" >> "$work/native-events-lifecycle-failure.log"
    if "$ROOT_DIR/tools/verify_lom_panel_native_gate.sh" "$work/native-events-lifecycle-failure.log" >/dev/null 2>&1; then
        echo "native verifier accepted a shell lifecycle failure" >&2
        exit 1
    fi
done

runner="$ROOT_DIR/tools/run_current_lom_panel_gate_tty4.sh"
preflight_line=$(grep -n 'lom_gpu_content_hardware_proof.sh' "$runner" | cut -d: -f1)
takeover_line=$(grep -nF '"$ROOT_DIR/tools/session/run_desktop_session.sh" --max-runtime-ms' "$runner" | cut -d: -f1)
[[ -n "$preflight_line" && -n "$takeover_line" && "$preflight_line" -lt "$takeover_line" ]] || {
    echo "native runner does not prove GPU/content before graphics takeover" >&2
    exit 1
}
grep -q '^SOPHIA_SESSION_STARTUP=none ' "$runner" || {
    echo "native runner unexpectedly starts an application" >&2
    exit 1
}
# Moved from Sophia crates/sophia-cli/tests/launcher_safety.rs:44-53 (the
# runner half; the generic session-launcher half stays in Sophia).
grep -qF 'SOPHIA_UNTRUSTED_SESSION_OUTPUT_LOG="$EVIDENCE_DIR/session/untrusted-session-output.log"' "$runner" || {
    echo "native runner does not keep unsanitized child output private and explicit" >&2
    exit 1
}
# Every source is explicit: no home default, no sibling checkout.
for script in "$runner" "$ROOT_DIR/tools/lom_gpu_content_hardware_proof.sh"; do
    if grep -nE '/home/|\$HOME/src|\.\./(lom|hagia|provlita|bemenu|sophia)|SOPHIA_[A-Z]+_SOURCE:-|SOPHIA_[A-Z]+_TARGET_DIR' "$script"; then
        echo "$script names an implicit source checkout" >&2
        exit 1
    fi
done
# Sophia is read only from the staged pinned tree: no path below the
# operator's checkout, and every build writes only to the private build dir.
for script in "$runner" "$ROOT_DIR/tools/lom_gpu_content_hardware_proof.sh" "$ROOT_DIR/tools/lib/artifacts.sh"; do
    if grep -nE '\$\{?SOPHIA_SOURCE\}?/' "$script"; then
        echo "$script reads Sophia outside the staged pinned tree" >&2
        exit 1
    fi
    if grep -E '^[^#]*cargo (build|run)' "$script"; then
        echo "$script builds outside the bounded preparation helper" >&2
        exit 1
    fi
done
# The standalone GPU wrapper keeps the staged-tree invariant on its own:
# stage, build, verify before exec, run the proof, verify after it, and only
# then trust the verifier. Each mutant drops one tree check and must fail.
wrapper="$ROOT_DIR/tools/lom_gpu_content_hardware_proof.sh"
wrapper_invariant() {
    local file=$1 stage before proof after verdict
    stage=$(grep -n 'physical_inputs_prepare --sophia-features' "$file" | cut -d: -f1)
    proof=$(grep -n '"\$SOPHIA_BIN" shell-gpu-content-proof' "$file" | cut -d: -f1)
    verdict=$(grep -n 'tools/verify_lom_gpu_content_hardware_proof.sh" "\$LOG"' "$file" | cut -d: -f1)
    mapfile -t checks < <(grep -n '^physical_inputs_verify_exported' "$file" | cut -d: -f1)
    [[ ${#checks[@]} -eq 2 && -n "$stage" && -n "$proof" && -n "$verdict" ]] || return 1
    before=${checks[0]} after=${checks[1]}
    (( stage < before && before < proof && proof < after && after < verdict ))
}
wrapper_invariant "$wrapper" || { echo "GPU wrapper does not re-verify the staged tree around exec" >&2; exit 1; }
for drop in 1 2; do
    awk -v drop="$drop" '/^physical_inputs_verify_exported/ { seen++; if (seen == drop) next } { print }' \
        "$wrapper" > "$work/wrapper-mutant-$drop.sh"
    if wrapper_invariant "$work/wrapper-mutant-$drop.sh"; then
        echo "wrapper invariant accepted a mutant without tree check $drop" >&2
        exit 1
    fi
done
grep -q 'SOPHIA_CORE_CONFIG="$LOM_CORE_CONFIG"' "$runner" || {
    echo "native runner does not pass its bounded application catalog to Sophia" >&2
    exit 1
}
if grep -Eq '^[[:space:]]*(bind|pointer-bind)[[:space:]]' \
    "$ROOT_DIR/tools/fixtures/lom_panel_desktop.kdl"; then
    echo "native panel profile carries an unrelated session action" >&2
    exit 1
fi
grep -q '^[[:space:]]*application-catalog "lom-panel-gate"$' \
    "$ROOT_DIR/tools/fixtures/lom_panel_desktop.kdl" || {
    echo "native panel profile does not select its application catalog" >&2
    exit 1
}
grep -q '^[[:space:]]*startup$' \
    "$ROOT_DIR/tools/fixtures/lom_panel_desktop.kdl" || {
    echo "native panel profile does not explicitly select an empty startup set" >&2
    exit 1
}
grep -q '^[[:space:]]*content-input #true$' \
    "$ROOT_DIR/tools/fixtures/lom_panel_desktop.kdl" || {
    echo "native panel profile does not admit discrete content input" >&2
    exit 1
}
grep -q 'application-catalog "lom-panel-gate" launch-policy="trusted-host"' \
    "$ROOT_DIR/tools/fixtures/lom_panel_core.kdl" || {
    echo "native panel core fixture does not define its admitted catalog" >&2
    exit 1
}

# These transcript controls launch only Python, never a native/GPU fixture.
python3 -B -m unittest discover -s "$ROOT_DIR/tools/probes/lom_workload/tests"


python3 -B -m unittest discover -s "$ROOT_DIR/tools/probes/native_launcher/tests"

echo "lom_gpu_content_verifiers schema=1 status=pass mutations=$(( ${#gpu_mutations[@]} + 12 )) structured_events=true pre_takeover_proof=true sequential_renders=2 workload_verifier=true launcher_smoke_verifier=true"
