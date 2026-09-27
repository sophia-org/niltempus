<!-- Generated from this repository's provenance headers and PROVENANCE files, checked against the pin de776c68afdf9a133818f86917893c3362dc9fb7 in the Sophia repository. -->
# Sophia deletion list

The Sophia-side half of the rule-13 move, for root to apply at (or after) the
pin `de776c68afdf9a133818f86917893c3362dc9fb7`, once this repository's gates
are green and these docs have landed. Every path was checked to exist at the
pin. "Moved to" is this repository's path; its header (or `PROVENANCE` file)
names the Sophia source and commit.

Nothing retained by ruling is listed for deletion, and no deletion may remove
retained Sophia coverage: every inbound reference in retained Sophia code is
listed under [Inbound hunks](#inbound-hunks) and changes in the same commit
(the live-record reader registry fails otherwise).

Counts: 297 whole-file deletions (290 from S5 plus 7 from the Quickshell closure, [quickshell](quickshell.md)), 20 moved-then-kept files that stay in Sophia by ruling, 34 files already deleted at the pin, plus the reductions and inbound hunks below. Lom and Bemenu
product IPC coverage is recorded as retired with product IPC in
[GATE-MAPPING](GATE-MAPPING.md), not moved.

## Kept in Sophia (by ruling; do not delete)

The product-neutral live-session verifier closure (director rulings on the S5
conflicts 1-4 and 6: the persistent, two-xterm, milestone 3, 4 and 5 GTK
evidence verifiers, the milestone 4 self-test, their fixtures and the decoy
configuration fixture read by `sophia_conformance::profile`). This repository
pin-reads them from the staged tree.

- `tools/check_live_session_milestone4_verifier.sh`
- `tools/fixtures/decoy-config-home/sophia/desktop.kdl`
- `tools/fixtures/live_session_milestone4_evidence_no_mixed_export.log`
- `tools/fixtures/live_session_milestone4_evidence_pass.log`
- `tools/fixtures/live_session_persistent_evidence_cleanup_debt.log`
- `tools/fixtures/live_session_persistent_evidence_pass.log`
- `tools/fixtures/live_session_persistent_evidence_physical_mismatch.log`
- `tools/fixtures/live_session_persistent_evidence_physical_missing.log`
- `tools/fixtures/live_session_persistent_evidence_physical_pass.log`
- `tools/fixtures/live_session_persistent_evidence_post_completion_error.log`
- `tools/fixtures/live_session_persistent_evidence_v8_pass.log`
- `tools/fixtures/live_session_persistent_evidence_wm_pass.log`
- `tools/fixtures/live_session_two_xterm_evidence_pass.log`
- `tools/fixtures/live_session_two_xterm_evidence_slow_compose.log`
- `tools/fixtures/live_session_two_xterm_evidence_slow_startup.log`
- `tools/verify_live_session_milestone3_evidence.sh`
- `tools/verify_live_session_milestone4_evidence.sh`
- `tools/verify_live_session_milestone5_gtk_evidence.sh`
- `tools/verify_live_session_persistent_evidence.sh`
- `tools/verify_live_session_two_xterm_evidence.sh`
- Never moved, retained: `tools/verify_live_session_three_class_baseline.sh`
  (the historical three-class archive aggregate), the QEMU harness
  (`tools/qemu_session_harness.sh`, `tools/qemu_milestone5_acceptance.sh`,
  `tools/verify_qemu_session_evidence.sh`), `tools/check_atomic_scanout_verifiers.sh`,
  and the schema-reader registrations of every kept verifier.
- Pin-read retained primitives: `tools/run_sophia_session.sh`,
  `tools/sophia_tty_mode.py`, `tools/stop_sophia_session.sh`,
  `tools/lib/session_preparation.sh`, `tools/lib/session_lifecycle.sh`,
  `tools/lib/session_profile.sh`, `tools/lib/drm_master_guard.sh`,
  `tools/config/sophia/core.kdl`, the probe profiles
  (`tools/fixtures/{mirror_group_probe,mixed_output_probe,frame_fed_output_proof,direct_scanout_core,direct_scanout_desktop,native_launcher_core}.kdl`),
  `tools/verify_atomic_scanout_preflight.sh`, `tools/atomic_scanout_preflight.sh`,
  `tools/verify_native_egl_mixed_evidence.sh`,
  `tools/verify_sophia_native_composition_pixels.sh`,
  `tools/report_sophia_input_latency.sh`, `tools/check_bounded_xterm_geometry.sh`,
  `tools/probes/run_bounded_xterm.sh`, `tools/collect_sophia_kernel_stall_log.sh`.
- Copied, not moved: `crates/xtask/src/git_tree.rs`,
  `crates/sophia-session/tests/support/component_processes/peer.rs`.

## Root follow-ups (do not delete)

- `tools/check_session_profile_preflight.sh` (registered at
  `crates/xtask/src/check.rs:119`) copies and runs the moved
  `tools/start_sophia_tty3.sh` (:51, :53, :72). Root retargets it (for example
  to `sophia config check-session-profile`); its coverage stays.
- `tools/check_session_lifecycle_diagnostics.sh:65` runs the moved
  `tools/status_live_session.sh`; it is run only by the orphan aggregator
  `tools/check_atomic_scanout_local.sh:38`. Retarget it, or rule on it.


## Whole-file deletions

### `crates/sophia-cli` (7)

| Sophia path | Moved to |
| --- | --- |
| `crates/sophia-cli/src/commands/session_prepare/discovery.rs` | `crates/xtask/src/session/discovery.rs` |
| `crates/sophia-cli/src/commands/session_prepare/proofs.rs` | `crates/xtask/src/session/proofs.rs` |
| `crates/sophia-cli/src/commands/session_prepare/standalone.rs` | `crates/xtask/src/session/standalone.rs` |
| `crates/sophia-cli/tests/fixtures/session_arguments_before_t027.sh` | `crates/xtask/tests/fixtures/session_arguments_before_t027.sh` |
| `crates/sophia-cli/tests/fixtures/session_environment_before_t027.sh` | `crates/xtask/tests/fixtures/session_environment_before_t027.sh` |
| `crates/sophia-cli/tests/fixtures/session_inputs_before_t027.sh` | `crates/xtask/tests/fixtures/session_inputs_before_t027.sh` |
| `crates/sophia-cli/tests/session_application_arguments.rs` | `crates/xtask/tests/session_recipe_application.rs` |

### `crates/sophia-conformance` (22)

| Sophia path | Moved to |
| --- | --- |
| `crates/sophia-conformance/src/desktop_comparison.rs` | `crates/desktop-comparison/src/lib.rs` |
| `crates/sophia-conformance/src/desktop_comparison/capture.rs` | `crates/desktop-comparison/src/capture.rs` |
| `crates/sophia-conformance/src/desktop_comparison/capture_owner.rs` | `crates/desktop-comparison/src/capture_owner.rs` |
| `crates/sophia-conformance/src/desktop_comparison/capture_owner/attestation.rs` | `crates/desktop-comparison/src/capture_owner/attestation.rs` |
| `crates/sophia-conformance/src/desktop_comparison/capture_owner/crtc.rs` | `crates/desktop-comparison/src/capture_owner/crtc.rs` |
| `crates/sophia-conformance/src/desktop_comparison/capture_owner/process_population.rs` | `crates/desktop-comparison/src/capture_owner/process_population.rs` |
| `crates/sophia-conformance/src/desktop_comparison/capture_owner/qualification.rs` | `crates/desktop-comparison/src/capture_owner/qualification.rs` |
| `crates/sophia-conformance/src/desktop_comparison/capture_owner/reference.rs` | `crates/desktop-comparison/src/capture_owner/reference.rs` |
| `crates/sophia-conformance/src/desktop_comparison/capture_owner/tests.rs` | `crates/desktop-comparison/src/capture_owner/tests.rs` |
| `crates/sophia-conformance/src/desktop_comparison/capture_owner/trace.rs` | `crates/desktop-comparison/src/capture_owner/trace.rs` |
| `crates/sophia-conformance/src/desktop_comparison/capture_owner/trace/tests.rs` | `crates/desktop-comparison/src/capture_owner/trace/tests.rs` |
| `crates/sophia-conformance/src/desktop_comparison/capture_owner/visibility.rs` | `crates/desktop-comparison/src/capture_owner/visibility.rs` |
| `crates/sophia-conformance/src/desktop_comparison/capture_owner/workload.rs` | `crates/desktop-comparison/src/capture_owner/workload.rs` |
| `crates/sophia-conformance/src/desktop_comparison/capture_owner/workload/tests.rs` | `crates/desktop-comparison/src/capture_owner/workload/tests.rs` |
| `crates/sophia-conformance/src/desktop_comparison/host.rs` | `crates/desktop-comparison/src/host.rs` |
| `crates/sophia-conformance/src/desktop_comparison/sample_record.rs` | `crates/desktop-comparison/src/sample_record.rs` |
| `crates/sophia-conformance/src/desktop_comparison/storage.rs` | `crates/desktop-comparison/src/storage.rs` |
| `crates/sophia-conformance/src/direct_scanout_gate.rs` | `crates/xtask/src/direct_scanout_gate.rs` |
| `crates/sophia-conformance/src/panel.rs` | `crates/xtask/src/panel.rs` (`verify`) |
| `crates/sophia-conformance/tests/direct_scanout.rs` | `crates/xtask/tests/direct_scanout_gate.rs` |
| `crates/sophia-conformance/tests/panel.rs` | `crates/xtask/tests/panel.rs` |
| `crates/sophia-conformance/tests/support/desktop_comparison.rs` | `crates/desktop-comparison/tests/support/desktop_comparison.rs` |

### `crates/xtask` (1)

| Sophia path | Moved to |
| --- | --- |
| `crates/xtask/src/panel.rs` | `crates/xtask/src/panel.rs` (`run`) |

### `docs` (2)

| Sophia path | Moved to |
| --- | --- |
| `docs/operations.md` | `docs/operations.md` |
| `docs/quickshell-x11-panel.md` | `docs/quickshell.md` ("Running") |

### `tools` (172)

| Sophia path | Moved to |
| --- | --- |
| `tools/activate_live_session_release.sh` | `tools/activate_live_session_release.sh` |
| `tools/archive_frame_fed_output_physical_run.sh` | `tools/archive_frame_fed_output_physical_run.sh` |
| `tools/archive_hagia_native_session_run.sh` | `tools/archive_hagia_native_session_run.sh` |
| `tools/archive_hagia_policy_physical_run.sh` | `tools/archive_hagia_policy_physical_run.sh` |
| `tools/archive_keyboard_independence_physical_run.sh` | `tools/archive_keyboard_independence_physical_run.sh` |
| `tools/archive_mirror_group_diagnostic_run.sh` | `tools/archive_mirror_group_diagnostic_run.sh` |
| `tools/archive_mirror_group_physical_run.sh` | `tools/archive_mirror_group_physical_run.sh` |
| `tools/archive_mixed_output_physical_run.sh` | `tools/archive_mixed_output_physical_run.sh` |
| `tools/benchmark_sophia_glxgears_shake_tty3.sh` | `tools/benchmark_sophia_glxgears_shake_tty3.sh` |
| `tools/benchmark_sophia_glxgears_tty3.sh` | `tools/benchmark_sophia_glxgears_tty3.sh` |
| `tools/benchmark_sophia_terminal_tty3.sh` | `tools/benchmark_sophia_terminal_tty3.sh` |
| `tools/benchmark_sophia_vkcube_tty3.sh` | `tools/benchmark_sophia_vkcube_tty3.sh` |
| `tools/benchmark_xserver_graphics.sh` | `tools/benchmark_xserver_graphics.sh` |
| `tools/check_firefox_m10_dialog_page.sh` | `tools/check_firefox_m10_dialog_page.sh` |
| `tools/check_firefox_m10_kitty_probe.sh` | `tools/check_firefox_m10_kitty_probe.sh` |
| `tools/check_firefox_m10_primary_kitty_probe.sh` | `tools/check_firefox_m10_primary_kitty_probe.sh` |
| `tools/check_firefox_m10_primary_page.sh` | `tools/check_firefox_m10_primary_page.sh` |
| `tools/check_firefox_m10_promotion_page.sh` | `tools/check_firefox_m10_promotion_page.sh` |
| `tools/check_firefox_m10_rendering_page.sh` | `tools/check_firefox_m10_rendering_page.sh` |
| `tools/check_firefox_m10_selection_kitty_probe.sh` | `tools/check_firefox_m10_selection_kitty_probe.sh` |
| `tools/check_firefox_m10_selection_page.sh` | `tools/check_firefox_m10_selection_page.sh` |
| `tools/check_frame_fed_output_verifier.sh` | `tools/check_frame_fed_output_verifier.sh` |
| `tools/check_hagia_native_matchers.sh` | `tools/check_hagia_native_matchers.sh` |
| `tools/check_hagia_physical_matchers.sh` | `tools/check_hagia_physical_matchers.sh` |
| `tools/check_hagia_profile_selection.sh` | `tools/check_hagia_profile_selection.sh` |
| `tools/check_installed_fallback_verifier.sh` | `tools/check_installed_fallback_verifier.sh` |
| `tools/check_installed_hagia_ledger.sh` | `tools/check_installed_hagia_ledger.sh` |
| `tools/check_installed_login_cycle_verifier.sh` | `tools/check_installed_login_cycle_verifier.sh` |
| `tools/check_installed_native_chrome_verifier.sh` | `tools/check_installed_native_chrome_verifier.sh` |
| `tools/check_installed_native_verifiers.sh` | `tools/check_installed_native_verifiers.sh` |
| `tools/check_installed_session_lifecycle_verifier.sh` | `tools/check_installed_session_lifecycle_verifier.sh` |
| `tools/check_installed_session_type.sh` | `tools/check_installed_session_type.sh` |
| `tools/check_installed_watchdog_recovery.sh` | `tools/check_installed_watchdog_recovery.sh` |
| `tools/check_installed_xterm_verifier.sh` | `tools/check_installed_xterm_verifier.sh` |
| `tools/check_keyboard_independence_session_verifier.sh` | `tools/check_keyboard_independence_session_verifier.sh` |
| `tools/check_keyboard_independence_verifier.sh` | `tools/check_keyboard_independence_verifier.sh` |
| `tools/check_live_session_install.sh` | `tools/check_live_session_install.sh` |
| `tools/check_live_session_milestone5_verifier.sh` | `tools/check_live_session_milestone5_verifier.sh` |
| `tools/check_mirror_group_physical_verifier.sh` | `tools/check_mirror_group_physical_verifier.sh` |
| `tools/check_proof_preconditions.sh` | `tools/check_proof_preconditions.sh` |
| `tools/check_rehearse_wm_9p.sh` | `tools/check_rehearse_wm_9p.sh` |
| `tools/check_retired_milestone_launchers.sh` | `tools/check_retired_milestone_launchers.sh` |
| `tools/check_session_terminal_arguments.sh` | `tools/check_session_terminal_arguments.sh` |
| `tools/check_sophia_firefox_dialog_verifier.sh` | `tools/check_sophia_firefox_dialog_verifier.sh` |
| `tools/check_sophia_firefox_lifecycle_verifier.sh` | `tools/check_sophia_firefox_lifecycle_verifier.sh` |
| `tools/check_sophia_firefox_physical_verifier.sh` | `tools/check_sophia_firefox_physical_verifier.sh` |
| `tools/check_sophia_firefox_primary_verifier.sh` | `tools/check_sophia_firefox_primary_verifier.sh` |
| `tools/check_sophia_firefox_rendering_verifier.sh` | `tools/check_sophia_firefox_rendering_verifier.sh` |
| `tools/check_sophia_firefox_selection_verifier.sh` | `tools/check_sophia_firefox_selection_verifier.sh` |
| `tools/check_sophia_glxgears_performance_reporter.sh` | `tools/check_sophia_glxgears_performance_reporter.sh` |
| `tools/check_sophia_native_chrome_verifier.sh` | `tools/check_sophia_native_chrome_verifier.sh` |
| `tools/check_sophia_rendering_performance_reporter.sh` | `tools/check_sophia_rendering_performance_reporter.sh` |
| `tools/check_sophia_standalone_vkcube_verifier.sh` | `tools/check_sophia_standalone_vkcube_verifier.sh` |
| `tools/check_sophia_terminal_performance_reporter.sh` | `tools/check_sophia_terminal_performance_reporter.sh` |
| `tools/check_truecolor_verifier.sh` | `tools/check_truecolor_verifier.sh` |
| `tools/check_xserver_rendering_performance_reporter.sh` | `tools/check_xserver_rendering_performance_reporter.sh` |
| `tools/collect_mirror_group_kernel_delta.sh` | `tools/collect_mirror_group_kernel_delta.sh` |
| `tools/compare_sophia_xserver_rendering.sh` | `tools/compare_sophia_xserver_rendering.sh` |
| `tools/desktop_comparison_tracefs.sh` | `tools/desktop_comparison_tracefs.sh` |
| `tools/desktop_comparison_tty3.sh` | `tools/desktop_comparison_tty3.sh` |
| `tools/diagnose_sophia_kitty_tty3.sh` | `tools/diagnose_sophia_kitty_tty3.sh` |
| `tools/direct_scanout_gate.sh` | `tools/direct_scanout_gate.sh` |
| `tools/hagia-native-proof` | `tools/hagia-native-proof` |
| `tools/hagia-proof` | `tools/hagia-proof` |
| `tools/hagia_client_lifecycle_fault_smoke.sh` | `tools/hagia_client_lifecycle_fault_smoke.sh` |
| `tools/hagia_live_session_smoke.sh` | `tools/hagia_live_session_smoke.sh` |
| `tools/hagia_native_session_gate.sh` | `tools/hagia_native_session_gate.sh` |
| `tools/hagia_owner_settlement_fault_smoke.sh` | `tools/hagia_owner_settlement_fault_smoke.sh` |
| `tools/hagia_policy_physical_gate.sh` | `tools/hagia_policy_physical_gate.sh` |
| `tools/install_live_session.sh` | `tools/install_live_session.sh` |
| `tools/keyboard_independence_physical_gate.sh` | `tools/keyboard_independence_physical_gate.sh` |
| `tools/keyboard_independence_walkthrough.py` | `tools/keyboard_independence_walkthrough.py` |
| `tools/lib/installed_attempt_ledger.sh` | `tools/lib/installed_attempt_ledger.sh` |
| `tools/lib/installed_hagia_evidence.sh` | `tools/lib/installed_hagia_evidence.sh` |
| `tools/lib/live_session_surface.sh` | `tools/lib/live_session_surface.sh` |
| `tools/lib/proof_checkout.sh` | `tools/lib/proof_checkout.sh` |
| `tools/lib/rendering_performance.sh` | `tools/lib/rendering_performance.sh` |
| `tools/lib/session_terminal.sh` | `tools/session/lib/session_terminal.sh` |
| `tools/lib/verify_firefox_rendering.awk` | `tools/lib/verify_firefox_rendering.awk` |
| `tools/live_session_milestone3_hardware_proof.sh` | `tools/live_session_milestone3_hardware_proof.sh` |
| `tools/live_session_milestone4_hardware_proof.sh` | `tools/live_session_milestone4_hardware_proof.sh` |
| `tools/live_session_milestone5_gtk_hardware_proof.sh` | `tools/live_session_milestone5_gtk_hardware_proof.sh` |
| `tools/live_session_persistent_hardware_proof.sh` | `tools/live_session_persistent_hardware_proof.sh` |
| `tools/live_session_two_xterm_hardware_proof.sh` | `tools/live_session_two_xterm_hardware_proof.sh` |
| `tools/native_egl_vkcube_mixed_smoke.sh` | `tools/native_egl_vkcube_mixed_smoke.sh` |
| `tools/operator_keyboard_hardware_proof.sh` | `tools/operator_keyboard_hardware_proof.sh` |
| `tools/output_topology_physical_gate.sh` | `tools/output_topology_physical_gate.sh` |
| `tools/package_live_session.sh` | `crates/xtask/src/package_desktop.rs` |
| `tools/prune_sophia_releases.sh` | `tools/prune_sophia_releases.sh` |
| `tools/record_installed_emergency_run.sh` | `tools/record_installed_emergency_run.sh` |
| `tools/record_installed_fallback_run.sh` | `tools/record_installed_fallback_run.sh` |
| `tools/record_installed_firefox_attempt.sh` | `tools/record_installed_firefox_attempt.sh` |
| `tools/record_installed_hagia_run.sh` | `tools/record_installed_hagia_run.sh` |
| `tools/record_installed_native_chrome_run.sh` | `tools/record_installed_native_chrome_run.sh` |
| `tools/record_installed_truecolor_run.sh` | `tools/record_installed_truecolor_run.sh` |
| `tools/record_installed_watchdog_run.sh` | `tools/record_installed_watchdog_run.sh` |
| `tools/record_installed_xterm_run.sh` | `tools/record_installed_xterm_run.sh` |
| `tools/record_sophia_firefox_physical_run.sh` | `tools/record_sophia_firefox_physical_run.sh` |
| `tools/rehearse_wm_9p.sh` | `tools/rehearse_wm_9p.sh` |
| `tools/reload_policy_client.sh` | `tools/reload_policy_client.sh` |
| `tools/report_sophia_glxgears_performance.sh` | `tools/report_sophia_glxgears_performance.sh` |
| `tools/report_sophia_rendering_performance.sh` | `tools/report_sophia_rendering_performance.sh` |
| `tools/report_sophia_terminal_performance.sh` | `tools/report_sophia_terminal_performance.sh` |
| `tools/report_xserver_rendering_performance.sh` | `tools/report_xserver_rendering_performance.sh` |
| `tools/rollback_live_session.sh` | `tools/rollback_live_session.sh` |
| `tools/run_current_critical_path_tty4.sh` | `tools/run_current_critical_path_tty4.sh` |
| `tools/run_current_hagia_native_gate_tty4.sh` | `tools/run_current_hagia_native_gate_tty4.sh` |
| `tools/run_current_hagia_policy_gate_tty4.sh` | `tools/run_current_hagia_policy_gate_tty4.sh` |
| `tools/run_frame_fed_output_gate_tty4.sh` | `tools/run_frame_fed_output_gate_tty4.sh` |
| `tools/run_gtk_redraw_probe.py` | `tools/run_gtk_redraw_probe.py` |
| `tools/run_keyboard_independence_gate_tty4.sh` | `tools/run_keyboard_independence_gate_tty4.sh` |
| `tools/run_mirror_group_gate_tty4.sh` | `tools/run_mirror_group_gate_tty4.sh` |
| `tools/run_mixed_output_gate_tty4.sh` | `tools/run_mixed_output_gate_tty4.sh` |
| `tools/run_output_topology_gate_tty4.sh` | `tools/run_output_topology_gate_tty4.sh` |
| `tools/run_sophia_input_latency_tty3.sh` | `tools/run_sophia_input_latency_tty3.sh` |
| `tools/run_sophia_kitty_session.sh` | `tools/session/run_sophia_kitty_session.sh` |
| `tools/run_sophia_terminal_gate_tty3.sh` | `tools/run_sophia_terminal_gate_tty3.sh` |
| `tools/run_x11_live_session_stability.sh` | `tools/run_x11_live_session_stability.sh` |
| `tools/setup_sophia_uinput.sh` | `tools/setup_sophia_uinput.sh` |
| `tools/start_sophia_hagia_policy_tty4.sh` | `tools/start_sophia_hagia_policy_tty4.sh` |
| `tools/start_sophia_kitty_tty3.sh` | `tools/session/start_sophia_kitty_tty3.sh` |
| `tools/start_sophia_native_hot_reload_tty3.sh` | `tools/start_sophia_native_hot_reload_tty3.sh` |
| `tools/start_sophia_tty3.sh` | `tools/session/start_sophia_tty3.sh` |
| `tools/start_sophia_vkcube_standalone_tty3.sh` | `tools/start_sophia_vkcube_standalone_tty3.sh` |
| `tools/status_live_session.sh` | `tools/status_live_session.sh` |
| `tools/stop_sophia_kitty_session.sh` | `tools/session/stop_sophia_kitty_session.sh` |
| `tools/stop_sophia_native_session.sh` | `tools/session/stop_sophia_native_session.sh` |
| `tools/stop_sophia_standalone_session.sh` | `tools/session/stop_sophia_standalone_session.sh` |
| `tools/test_verify_mixed_output_evidence.sh` | `tools/test_verify_mixed_output_evidence.sh` |
| `tools/verify_frame_fed_output_evidence.sh` | `tools/verify_frame_fed_output_evidence.sh` |
| `tools/verify_frame_fed_output_physical_archive.sh` | `tools/verify_frame_fed_output_physical_archive.sh` |
| `tools/verify_hagia_native_session.sh` | `tools/verify_hagia_native_session.sh` |
| `tools/verify_hagia_native_session_archive.sh` | `tools/verify_hagia_native_session_archive.sh` |
| `tools/verify_hagia_policy_physical.sh` | `tools/verify_hagia_policy_physical.sh` |
| `tools/verify_hagia_policy_physical_archive.sh` | `tools/verify_hagia_policy_physical_archive.sh` |
| `tools/verify_installed_emergency_archive.sh` | `tools/verify_installed_emergency_archive.sh` |
| `tools/verify_installed_fallback_run.sh` | `tools/verify_installed_fallback_run.sh` |
| `tools/verify_installed_fallback_session.sh` | `tools/verify_installed_fallback_session.sh` |
| `tools/verify_installed_hagia_archive.sh` | `tools/verify_installed_hagia_archive.sh` |
| `tools/verify_installed_hagia_recovery.sh` | `tools/verify_installed_hagia_recovery.sh` |
| `tools/verify_installed_hagia_session.sh` | `tools/verify_installed_hagia_session.sh` |
| `tools/verify_installed_login_cycle.sh` | `tools/verify_installed_login_cycle.sh` |
| `tools/verify_installed_native_chrome_archive.sh` | `tools/verify_installed_native_chrome_archive.sh` |
| `tools/verify_installed_native_chrome_session.sh` | `tools/verify_installed_native_chrome_session.sh` |
| `tools/verify_installed_runtime_identity.sh` | `tools/verify_installed_runtime_identity.sh` |
| `tools/verify_installed_session_lifecycle.sh` | `tools/verify_installed_session_lifecycle.sh` |
| `tools/verify_installed_truecolor_runs.sh` | `tools/verify_installed_truecolor_runs.sh` |
| `tools/verify_installed_truecolor_session.sh` | `tools/verify_installed_truecolor_session.sh` |
| `tools/verify_installed_watchdog_archive.sh` | `tools/verify_installed_watchdog_archive.sh` |
| `tools/verify_installed_watchdog_recovery.sh` | `tools/verify_installed_watchdog_recovery.sh` |
| `tools/verify_installed_xterm_runs.sh` | `tools/verify_installed_xterm_runs.sh` |
| `tools/verify_installed_xterm_session.sh` | `tools/verify_installed_xterm_session.sh` |
| `tools/verify_keyboard_independence_physical.sh` | `tools/verify_keyboard_independence_physical.sh` |
| `tools/verify_keyboard_independence_physical_archive.sh` | `tools/verify_keyboard_independence_physical_archive.sh` |
| `tools/verify_keyboard_independence_session.sh` | `tools/verify_keyboard_independence_session.sh` |
| `tools/verify_live_session_milestone5_tty_recovery.sh` | `tools/verify_live_session_milestone5_tty_recovery.sh` |
| `tools/verify_mirror_group_diagnostic.sh` | `tools/verify_mirror_group_diagnostic.sh` |
| `tools/verify_mirror_group_diagnostic_archive.sh` | `tools/verify_mirror_group_diagnostic_archive.sh` |
| `tools/verify_mirror_group_physical.sh` | `tools/verify_mirror_group_physical.sh` |
| `tools/verify_mirror_group_physical_archive.sh` | `tools/verify_mirror_group_physical_archive.sh` |
| `tools/verify_mixed_output_evidence.sh` | `tools/verify_mixed_output_evidence.sh` |
| `tools/verify_mixed_output_physical_archive.sh` | `tools/verify_mixed_output_physical_archive.sh` |
| `tools/verify_packaged_policy.sh` | `tools/verify_packaged_policy.sh` |
| `tools/verify_sophia_firefox_dialog_physical.sh` | `tools/verify_sophia_firefox_dialog_physical.sh` |
| `tools/verify_sophia_firefox_lifecycle_physical.sh` | `tools/verify_sophia_firefox_lifecycle_physical.sh` |
| `tools/verify_sophia_firefox_physical.sh` | `tools/verify_sophia_firefox_physical.sh` |
| `tools/verify_sophia_firefox_physical_runs.sh` | `tools/verify_sophia_firefox_physical_runs.sh` |
| `tools/verify_sophia_firefox_primary_physical.sh` | `tools/verify_sophia_firefox_primary_physical.sh` |
| `tools/verify_sophia_firefox_rendering_physical.sh` | `tools/verify_sophia_firefox_rendering_physical.sh` |
| `tools/verify_sophia_firefox_selection_physical.sh` | `tools/verify_sophia_firefox_selection_physical.sh` |
| `tools/verify_sophia_native_chrome.sh` | `tools/verify_sophia_native_chrome.sh` |
| `tools/verify_sophia_standalone_vkcube.sh` | `tools/verify_sophia_standalone_vkcube.sh` |

### `tools/config` (3)

| Sophia path | Moved to |
| --- | --- |
| `tools/config/99-sophia-uinput.rules` | `tools/config/99-sophia-uinput.rules` |
| `tools/config/proof_helpers.sh` | `tools/config/proof_helpers.sh` |
| `tools/config/sophia-uinput.conf` | `tools/config/sophia-uinput.conf` |

### `tools/fixtures` (63)

| Sophia path | Moved to |
| --- | --- |
| `tools/fixtures/firefox_m10_kitty_probe.sh` | `tools/fixtures/firefox_m10_kitty_probe.sh` |
| `tools/fixtures/firefox_m10_primary_kitty_probe.sh` | `tools/fixtures/firefox_m10_primary_kitty_probe.sh` |
| `tools/fixtures/firefox_m10_selection_kitty_probe.sh` | `tools/fixtures/firefox_m10_selection_kitty_probe.sh` |
| `tools/fixtures/firefox_m8_local_page.html` | `tools/fixtures/firefox_m8_local_page.html` |
| `tools/fixtures/hagia_native_session_guide.sh` | `tools/fixtures/hagia_native_session_guide.sh` |
| `tools/fixtures/hagia_physical_guide.sh` | `tools/fixtures/hagia_physical_guide.sh` |
| `tools/fixtures/hagia_restart_once.sh` | `tools/fixtures/hagia_restart_once.sh` |
| `tools/fixtures/installed_emergency_session_pass.log` | `tools/fixtures/installed_emergency_session_pass.log` |
| `tools/fixtures/installed_fallback_guard_pass.log` | `tools/fixtures/installed_fallback_guard_pass.log` |
| `tools/fixtures/installed_fallback_recovery_pass.log` | `tools/fixtures/installed_fallback_recovery_pass.log` |
| `tools/fixtures/installed_fallback_session_pass.log` | `tools/fixtures/installed_fallback_session_pass.log` |
| `tools/fixtures/installed_lifecycle_emergency_pass.log` | `tools/fixtures/installed_lifecycle_emergency_pass.log` |
| `tools/fixtures/installed_lifecycle_normal_pass.log` | `tools/fixtures/installed_lifecycle_normal_pass.log` |
| `tools/fixtures/installed_lifecycle_watchdog_pass.log` | `tools/fixtures/installed_lifecycle_watchdog_pass.log` |
| `tools/fixtures/installed_native_chrome_recovery_pass.log` | `tools/fixtures/installed_native_chrome_recovery_pass.log` |
| `tools/fixtures/installed_runtime_identity_pass.log` | `tools/fixtures/installed_runtime_identity_pass.log` |
| `tools/fixtures/installed_session_soak_pass.log` | `tools/fixtures/installed_session_soak_pass.log` |
| `tools/fixtures/installed_soak_archive_session_pass.log` | `tools/fixtures/installed_soak_archive_session_pass.log` |
| `tools/fixtures/installed_truecolor_input_guard_pass.log` | `tools/fixtures/installed_truecolor_input_guard_pass.log` |
| `tools/fixtures/installed_truecolor_recovery_pass.log` | `tools/fixtures/installed_truecolor_recovery_pass.log` |
| `tools/fixtures/installed_truecolor_session_pass.log` | `tools/fixtures/installed_truecolor_session_pass.log` |
| `tools/fixtures/installed_watchdog_guard_pass.log` | `tools/fixtures/installed_watchdog_guard_pass.log` |
| `tools/fixtures/installed_watchdog_recovery_pass.log` | `tools/fixtures/installed_watchdog_recovery_pass.log` |
| `tools/fixtures/installed_watchdog_session_pass.log` | `tools/fixtures/installed_watchdog_session_pass.log` |
| `tools/fixtures/installed_xterm_session_pass.log` | `tools/fixtures/installed_xterm_session_pass.log` |
| `tools/fixtures/keyboard_independence_guide.sh` | `tools/fixtures/keyboard_independence_guide.sh` |
| `tools/fixtures/keyboard_independence_physical_pass/guard_pinned.log` | `tools/fixtures/keyboard_independence_physical_pass/guard_pinned.log` |
| `tools/fixtures/keyboard_independence_physical_pass/guard_seat.log` | `tools/fixtures/keyboard_independence_physical_pass/guard_seat.log` |
| `tools/fixtures/keyboard_independence_physical_pass/session.log` | `tools/fixtures/keyboard_independence_physical_pass/session.log` |
| `tools/fixtures/keyboard_independence_session_pass/events.0.log` | `tools/fixtures/keyboard_independence_session_pass/events.0.log` |
| `tools/fixtures/keyboard_independence_session_pass/health` | `tools/fixtures/keyboard_independence_session_pass/health` |
| `tools/fixtures/keyboard_independence_session_pass/lifecycle.log` | `tools/fixtures/keyboard_independence_session_pass/lifecycle.log` |
| `tools/fixtures/keyboard_independence_session_pass/manifest` | `tools/fixtures/keyboard_independence_session_pass/manifest` |
| `tools/fixtures/keyboard_independence_session_pass/outcome` | `tools/fixtures/keyboard_independence_session_pass/outcome` |
| `tools/fixtures/live_session_milestone5_gtk_classic_pass.log` | `tools/fixtures/live_session_milestone5_gtk_classic_pass.log` |
| `tools/fixtures/live_session_milestone5_gtk_confined_pass.log` | `tools/fixtures/live_session_milestone5_gtk_confined_pass.log` |
| `tools/fixtures/live_session_milestone5_gtk_protocol_error.log` | `tools/fixtures/live_session_milestone5_gtk_protocol_error.log` |
| `tools/fixtures/live_session_milestone5_tty_recovery_bad_kd.log` | `tools/fixtures/live_session_milestone5_tty_recovery_bad_kd.log` |
| `tools/fixtures/live_session_milestone5_tty_recovery_emergency.log` | `tools/fixtures/live_session_milestone5_tty_recovery_emergency.log` |
| `tools/fixtures/live_session_milestone5_tty_recovery_pass.log` | `tools/fixtures/live_session_milestone5_tty_recovery_pass.log` |
| `tools/fixtures/mirror_group_physical_pass.log` | `tools/fixtures/mirror_group_physical_pass.log` |
| `tools/fixtures/physical_firefox_dialog_pass.log` | `tools/fixtures/physical_firefox_dialog_pass.log` |
| `tools/fixtures/physical_firefox_guard_pass.log` | `tools/fixtures/physical_firefox_guard_pass.log` |
| `tools/fixtures/physical_firefox_lifecycle_pass.log` | `tools/fixtures/physical_firefox_lifecycle_pass.log` |
| `tools/fixtures/physical_firefox_primary_pass.log` | `tools/fixtures/physical_firefox_primary_pass.log` |
| `tools/fixtures/physical_firefox_recovery_pass.log` | `tools/fixtures/physical_firefox_recovery_pass.log` |
| `tools/fixtures/physical_firefox_rendering_pass.log` | `tools/fixtures/physical_firefox_rendering_pass.log` |
| `tools/fixtures/physical_firefox_selection_pass.log` | `tools/fixtures/physical_firefox_selection_pass.log` |
| `tools/fixtures/physical_firefox_session_pass.log` | `tools/fixtures/physical_firefox_session_pass.log` |
| `tools/fixtures/physical_native_chrome_pass.log` | `tools/fixtures/physical_native_chrome_pass.log` |
| `tools/fixtures/physical_native_chrome_sequence_pass.log` | `tools/fixtures/physical_native_chrome_sequence_pass.log` |
| `tools/fixtures/quickshell_sophia/core.kdl` | `tools/fixtures/quickshell_sophia/core.kdl` |
| `tools/fixtures/quickshell_sophia/desktop.kdl` | `tools/fixtures/quickshell_sophia/desktop.kdl` |
| `tools/fixtures/quickshell_sophia/shell.qml` | `tools/fixtures/quickshell_sophia/shell.qml` |
| `tools/fixtures/rendering_performance_pass.log` | `tools/fixtures/rendering_performance_pass.log` |
| `tools/fixtures/sophia_glxgears_performance_pass.log` | `tools/fixtures/sophia_glxgears_performance_pass.log` |
| `tools/fixtures/sophia_terminal_performance_pass.log` | `tools/fixtures/sophia_terminal_performance_pass.log` |
| `tools/fixtures/sophia_terminal_performance_startup_only.log` | `tools/fixtures/sophia_terminal_performance_startup_only.log` |
| `tools/fixtures/standalone_vkcube_verifier_cpu_pass.log` | `tools/fixtures/standalone_vkcube_verifier_cpu_pass.log` |
| `tools/fixtures/standalone_vkcube_verifier_pass.log` | `tools/fixtures/standalone_vkcube_verifier_pass.log` |
| `tools/fixtures/t018_tab_reference.kdl` | `tools/fixtures/t018_tab_reference.kdl` |
| `tools/fixtures/truecolor_kitty_probe.sh` | `tools/fixtures/truecolor_kitty_probe.sh` |
| `tools/fixtures/xserver_rendering_performance_pass.log` | `tools/fixtures/xserver_rendering_performance_pass.log` |

### `tools/installed` (12)

| Sophia path | Moved to |
| --- | --- |
| `tools/installed/capture-runtime-identity.sh` | `tools/installed/capture-runtime-identity.sh` |
| `tools/installed/sophia-firefox-proof` | `tools/installed/sophia-firefox-proof` |
| `tools/installed/sophia-hagia-promotion-session` | `tools/installed/sophia-hagia-promotion-session` |
| `tools/installed/sophia-hagia-session` | `tools/installed/sophia-hagia-session` |
| `tools/installed/sophia-hagia-xtest-session` | `tools/installed/sophia-hagia-xtest-session` |
| `tools/installed/sophia-kitty-session` | `tools/installed/sophia-kitty-session` |
| `tools/installed/sophia-native-chrome-proof` | `tools/installed/sophia-native-chrome-proof` |
| `tools/installed/sophia-recovery-proof` | `tools/installed/sophia-recovery-proof` |
| `tools/installed/sophia-session` | `tools/installed/sophia-session` |
| `tools/installed/sophia-stop` | `tools/installed/sophia-stop` |
| `tools/installed/sophia-truecolor-proof` | `tools/installed/sophia-truecolor-proof` |
| `tools/installed/sophia-xterm-proof` | `tools/installed/sophia-xterm-proof` |

### `tools/probes` (4)

| Sophia path | Moved to |
| --- | --- |
| `tools/probes/gtk_redraw.c` | `tools/probes/gtk_redraw.c` |
| `tools/probes/run_bounded_glxgears.sh` | `tools/probes/run_bounded_glxgears.sh` |
| `tools/probes/uinput_text_injector.py` | `tools/probes/uinput_text_injector.py` |
| `tools/probes/xserver_present_probe.c` | `tools/probes/xserver_present_probe.c` |

### `tools/tests` (1)

| Sophia path | Moved to |
| --- | --- |
| `tools/tests/physical_gate_identity_test.py` | `tools/tests/physical_gate_identity_test.py` |

### `validation/desktop-comparison` (10)

| Sophia path | Moved to |
| --- | --- |
| `validation/desktop-comparison/README.md` | `validation/desktop-comparison/README.md` |
| `validation/desktop-comparison/config/niri.kdl` | `validation/desktop-comparison/config/niri.kdl` |
| `validation/desktop-comparison/config/sophia.kdl` | `validation/desktop-comparison/config/sophia.kdl` |
| `validation/desktop-comparison/config/xlibre-xmonad.kdl` | `validation/desktop-comparison/config/xlibre-xmonad.kdl` |
| `validation/desktop-comparison/firefox/index.html` | `validation/desktop-comparison/firefox/index.html` |
| `validation/desktop-comparison/firefox/user.js` | `validation/desktop-comparison/firefox/user.js` |
| `validation/desktop-comparison/profiles/core.kdl` | `validation/desktop-comparison/profiles/core.kdl` |
| `validation/desktop-comparison/profiles/hagia.kdl` | `validation/desktop-comparison/profiles/hagia.kdl` |
| `validation/desktop-comparison/profiles/niri.kdl` | `validation/desktop-comparison/profiles/niri.kdl` |
| `validation/desktop-comparison/profiles/xmonad.hs` | `validation/desktop-comparison/profiles/xmonad.hs` |

## Reductions (partial moves)

- `crates/sophia-cli/src/commands/session_prepare.rs`: remove
  `prepare-arguments` (the named-profile whitelist, the Hagia block, the Firefox
  recipe and the Kitty/xterm terminal arguments, including the probe and page
  names at :205-211 and :256) and the `prepare-inputs` and `stage-proofs`
  subcommands. Keep `prepare-controls` (with an opaque profile label),
  `prepare-environment` (TTY, bus mode, verbose trace and atomic-scanout smoke
  only; drop the Firefox probe), `check-launch`, `host`, `acceptance` and
  `bounded.rs`.
- `crates/sophia-cli/src/commands/session_prepare/{discovery,proofs,standalone}.rs`:
  delete (listed above); `environment.rs` and `controls.rs` are reduced as
  described; `bounded.rs` stays.
- `crates/sophia-cli/src/commands/help.rs:69` and `:72`: drop the
  `prepare-arguments` line and `--firefox-probe`.
- `crates/sophia-cli/tests/session_prepare_arguments.rs`: keep only the
  unknown-option refusal for the kept subcommands.
- `crates/sophia-cli/tests/session_prepare_inputs.rs`: keep the controls and
  `check-launch` parts.
- `crates/sophia-cli/tests/launcher_safety.rs`: remove the moved sections
  (installed 271-377, Lom 44-53, and the sections that read moved scripts:
  the references at :2-11, :255, :303, :319, :331, :339, :391); keep the
  sections on the reduced `run_sophia_session.sh`.
- `crates/sophia-cli/tests/session_launcher_recovery.rs`: remove
  `tty_adapter_refuses_controls_before_queries_or_privileged_handoff` (it runs
  the moved `start_sophia_tty3.sh`, :80-145); keep the generic recovery tests.
- `crates/sophia-conformance/src/lib.rs:16` (`pub mod desktop_comparison`) and
  `:21` (`pub mod direct_scanout_gate`); `Cargo.toml:59-60, 70` (`drm`,
  `niri-ipc`, `x11rb`) once nothing else uses them.
- `crates/xtask/src/main.rs`: the `desktop_comparison` and
  `direct_scanout_gate` imports (:27-28), dispatch (:98, :137-141),
  `run_desktop_comparison` (:154), `gate_desktop_comparison` (:240),
  `build_release_sophia` (:270), `run_direct_scanout` (:293),
  `gate_direct_scanout` (:323) and usage lines :505-506 and :519-532.
  Direct-scanout verify, bind, archive and archive-verify stay.
- Quickshell ([quickshell](quickshell.md), "Root deletion boundary"):
  `crates/sophia-cli/src/commands/x_authority.rs:76-88` (the two named arms);
  `crates/sophia-cli/src/commands/help.rs:57-58`;
  `run_x_authority_quickshell_smoke` and `run_x_authority_quickshell_software_smoke`
  in `crates/sophia-cli/src/commands/x_authority/basic_smokes.rs` (:522-652);
  the `"quickshell"` stage entry in `external_probe.rs:347` and
  `probe_tolerates_client_error` (:538-549), which has no other caller. Keep
  the shared observer's null-window pre-filter, the render-device helpers and
  every other probe. `crates/sophia-conformance/src/lib.rs:27`
  (`pub mod panel`). `crates/xtask/src/main.rs:22` (`mod panel`), :45 (the
  `panel` dispatch), :94-97 (`conformance verify panel`) and usage :447-449.

## Inbound hunks

Retained Sophia files that name a deleted path (line numbers at the pin):

| File | Lines | Change |
| --- | --- | --- |
| `crates/xtask/src/check.rs` | :80 | drop `physical_gate_identity_test.py` |
| `crates/xtask/src/check.rs` | :110 | drop the moved terminal gate `--self-test` |
| `crates/xtask/src/check.rs` | :120, :123, :125, :128-136 | drop the moved self-tests (keep :119, :121, :122, :124, :126, :127) |
| `crates/xtask/src/check.rs` | :158-165 | drop the hagia-native and mirror-group archive families (keep direct-scanout) |
| `tools/check_live_record_schema_readers.py` | :18-28, :31, :33-35, :39, :41-42, :61-62 | remove the moved readers from `PROOF_READERS`, `NORMAL_READERS` and `WM_READERS`; keep the kept verifiers and the QEMU readers |
| `tools/tests/check_live_record_schema_readers_test.py` | :56-57, :59, :62, :82, :88, :103 | update the fixtures that name moved readers |
| `tools/check_no_legacy_wm_bridge.sh` | :16-17, :24, :38-40, :42-43 | drop the moved scripts from its globs and exemptions |
| `tools/check_atomic_scanout_local.sh` | :11, :13, :32-35, :39, :41-64, :66, :84-87, :89 | drop the lines for moved checks (keep :28 `check_atomic_scanout_verifiers.sh`; :37 `check_session_terminal_arguments.sh` moved) |
| `justfile` | :32-67, :71-78, :114-115, :123-128, :136-137, :139-165 | drop the `direct-scanout-probe` and `direct-scanout-*-gate` recipes (the moved gate), `glxgears-*`, `install-session`, `xtest-session`, `reload-wm` and `desktop-comparison-*`; keep `direct-scanout-archive` and `direct-scanout-verify` |
| `tools/audit_no_xlibre_runtime.sh` | :18 | drop the moved `tools/install_live_session.sh` from the scan |
| `tools/remote_target.sh` | :101 | point the physical-proof hint at the integration repository |
| `crates/sophia-session/tests/support/live_session/session_config_tests.rs` | :1323 | comment: the cursor-path line is required by the external `verify_hagia_native_session.sh` |
| `tools/probes/run_bounded_xterm.sh` | :7 | comment: the report consumer is external |
| `docs/README.md` | :22-23 | drop the Quickshell X11 panel entry (moved to this repository's `docs/quickshell.md`) |
| `docs/x11-compatibility-matrix.md` | rows at :81, :101, :102 | stop citing `x-authority-quickshell-smoke` and `-software-smoke`; cite `present_msc_ordering.rs` (`present_selection_after_destroy_reports_bad_window_and_keeps_serving`) and the x11_wire SHAPE tests, and point to this repository for the client trace; do not carry over the unasserted GLX/DRI3/Present and "no server refusal" claims ([quickshell](quickshell.md)) |
| `crates/sophia-conformance/src/direct_scanout_gate.rs` | whole file | moved (listed above); `profile::check_every_profile` stays in Sophia |

Orphans to delete with the move (they only drive moved scripts):
`tools/install_current_live_session.sh`, `tools/install_current_sophia_release.sh`,
`tools/install_session_from_head.sh` (replaced by `cargo xtask
package-desktop` and `tools/install_live_session.sh` here).

Historical notes (`done*.md`, `docs/notes/**`, `research/**`,
`validation/specula/**`) are not edited.


## Already deleted at the pin

Moved before the pin (E2 and G2) and already removed from Sophia by root:

- `crates/sophia-conformance/src/dock.rs` (moved to `crates/xtask/src/dock.rs`)
- `crates/sophia-conformance/tests/dock.rs` (moved to `crates/xtask/tests/dock.rs`)
- `crates/sophia-runtime/tests/shell_bemenu_files.rs` (moved to `crates/live-tests/tests/bemenu_files.rs`)
- `crates/sophia-runtime/tests/support/shell_bemenu_files/artifact.rs` (moved to `crates/live-tests/tests/support/bemenu_files/artifact.rs`)
- `crates/sophia-runtime/tests/support/shell_bemenu_files/fixture.rs` (moved to `crates/live-tests/tests/support/bemenu_files/fixture.rs`)
- `crates/sophia-runtime/tests/support/shell_bemenu_files/sandbox.rs` (moved to `crates/live-tests/tests/support/bemenu_files/sandbox.rs`)
- `crates/sophia-session/tests/support/component_processes/bemenu.rs` (moved to `crates/live-tests/tests/support/component_processes/bemenu.rs`)
- `crates/xtask/src/bemenu_artifact.rs` (moved to `crates/xtask/src/bemenu_artifact.rs`)
- `crates/xtask/tests/bemenu_artifact.rs` (moved to `crates/xtask/tests/bemenu_artifact.rs`)
- `crates/xtask/tests/dock_launcher.rs` (moved to `crates/xtask/tests/dock_launcher.rs`)
- `tools/check_lom_gpu_content_proof_verifiers.sh` (moved to `tools/check_lom_gpu_content_proof_verifiers.sh`)
- `tools/fixtures/lom_panel_core.kdl` (moved to `tools/fixtures/lom_panel_core.kdl`)
- `tools/fixtures/lom_panel_desktop.kdl` (moved to `tools/fixtures/lom_panel_desktop.kdl`)
- `tools/fixtures/lom_workload_budgets.json` (moved to `tools/fixtures/lom_workload_budgets.json`)
- `tools/lom_gpu_content_hardware_proof.sh` (moved to `tools/lom_gpu_content_hardware_proof.sh`)
- `tools/probes/dock/README.md` (moved to `tools/probes/dock/README.md`)
- `tools/probes/lom_workload/.gitignore` (moved to `tools/probes/lom_workload/.gitignore`)
- `tools/probes/lom_workload/README.md` (moved to `tools/probes/lom_workload/README.md`)
- `tools/probes/lom_workload/memory.py` (moved to `tools/probes/lom_workload/memory.py`)
- `tools/probes/lom_workload/records.py` (moved to `tools/probes/lom_workload/records.py`)
- `tools/probes/lom_workload/tests/fixture.py` (moved to `tools/probes/lom_workload/tests/fixture.py`)
- `tools/probes/lom_workload/tests/test_launcher.py` (moved to `tools/probes/lom_workload/tests/test_launcher.py`)
- `tools/probes/lom_workload/tests/test_memory.py` (moved to `tools/probes/lom_workload/tests/test_memory.py`)
- `tools/probes/lom_workload/tests/test_shutdown.py` (moved to `tools/probes/lom_workload/tests/test_shutdown.py`)
- `tools/probes/lom_workload/tests/test_verify.py` (moved to `tools/probes/lom_workload/tests/test_verify.py`)
- `tools/probes/lom_workload/verify.py` (moved to `tools/probes/lom_workload/verify.py`)
- `tools/probes/native_launcher/README.md` (moved to `tools/probes/native_launcher/README.md`)
- `tools/probes/native_launcher/profile.py` (moved to `tools/probes/native_launcher/profile.py`)
- `tools/probes/native_launcher/tests/test_command.py` (moved to `tools/probes/native_launcher/tests/test_command.py`)
- `tools/probes/native_launcher/tests/test_native_verify.py` (moved to `tools/probes/native_launcher/tests/test_native_verify.py`)
- `tools/probes/native_launcher/verify.py` (moved to `tools/probes/native_launcher/verify.py`)
- `tools/run_current_lom_panel_gate_tty4.sh` (moved to `tools/run_current_lom_panel_gate_tty4.sh`)
- `tools/verify_lom_gpu_content_hardware_proof.sh` (moved to `tools/verify_lom_gpu_content_hardware_proof.sh`)
- `tools/verify_lom_panel_native_gate.sh` (moved to `tools/verify_lom_panel_native_gate.sh`)
