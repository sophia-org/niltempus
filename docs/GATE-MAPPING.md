# Gate mapping

Every Sophia gate, test and recipe that the rule-13 move touched, at the pin
`de776c68afdf9a133818f86917893c3362dc9fb7`, and where its coverage lives now.
"E" is this repository's offline gate (`cargo test --workspace --offline
--locked`, clippy, fmt, the pinned self-tests with `SOPHIA_TEST_SOURCE` and
`SOPHIA_TEST_TREE`, and `cargo xtask verify-archives`). "S" is retained Sophia
coverage. "Retired with product IPC" means the coverage exercised a product
over Sophia's current IPC shell wire, which is being removed from Lom, Bemenu
and Hagia; it is retired together with that IPC, not moved.

## Sophia `cargo xtask check` (crates/xtask/src/check.rs)

| Sophia entry (check.rs line) | Now |
| --- | --- |
| `python3 -m unittest ... physical_gate_identity_test.py` (:80) | E `physical_selftests::physical_gate_identity` (dry runs now stub the prepared-inputs helper and require nice 19, two jobs, no build and untouched checkouts) |
| `archives()`: hagia-native-runs, mirror-group-runs (:160, :164) | E `cargo xtask verify-archives` (integration schema 1; `--legacy` for older runs) |
| `archives()`: direct-scanout-runs | S (Rust `direct_scanout_archive`) |
| `run_sophia_terminal_gate_tty3.sh --self-test` (:110) | E `physical_selftests::run_sophia_terminal_gate_tty3_self_test` |
| `check_session_profile_preflight.sh` (:119) | S; root follow-up: retarget it off the moved `start_sophia_tty3.sh` (do not delete) |
| `check_installed_session_type.sh` (:120) | E `installed_selftests::installed_session_type` |
| `check_bounded_xterm_geometry.sh` (:121) | S (retained; pin-read by the terminal gate) |
| `check_live_record_schema_readers.sh --self-test` and plain (:115, :122) | S, with the moved readers' registrations removed by root (see the deletion list) |
| `check_retired_milestone_launchers.sh` (:123) | E `physical_selftests::check_retired_milestone_launchers` (pinned: the historical readers stay in Sophia) |
| `check_live_session_milestone4_verifier.sh` (:124) | S (kept by ruling with the m4 verifier and fixtures) |
| `check_sophia_firefox_physical_verifier.sh` (:125) | E `physical_selftests` |
| `check_direct_scanout_verifier.sh`, `check_direct_scanout_archive_verifier.sh` (:126-127) | S |
| `check_sophia_standalone_vkcube_verifier.sh` (:128) | E `physical_selftests` |
| `check_hagia_native_matchers.sh` (:129) | E `physical_selftests` (pinned) |
| `check_firefox_m10_rendering_page.sh` (:130) | E `physical_selftests` |
| `check_sophia_firefox_rendering_verifier.sh` (:131) | E `physical_selftests` |
| `check_mirror_group_physical_verifier.sh` (:132) | E `physical_selftests` (pinned) |
| `check_keyboard_independence_verifier.sh`, `..._session_verifier.sh` (:133-134) | E `physical_selftests` |
| `check_sophia_terminal_performance_reporter.sh` (:135) | E `physical_selftests` |
| `check_installed_native_verifiers.sh` and its sub-checks (:136) | E `physical_selftests` (each sub-check its own test) |
| `check_lom_gpu_content_proof_verifiers.sh` (moved at 9fcaec782) | E `verifier_self_tests`; the Lom gate itself is pending the Lom 9P seams |

Newly automated here (manual only in Sophia): `check_hagia_profile_selection.sh`,
`check_live_session_install.sh`, `check_rehearse_wm_9p.sh`
(`installed_selftests`), the Firefox page and probe checks, the installed
verifier sub-checks, the performance reporters, `check_hagia_physical_matchers.sh`,
`check_frame_fed_output_verifier.sh`, `test_verify_mixed_output_evidence.sh`
and `check_live_session_milestone5_verifier.sh` (`physical_selftests`).

## Rust tests

| Sophia test | Now |
| --- | --- |
| `sophia-conformance` `desktop_comparison*` (26 unit tests) | E crate `desktop-comparison` |
| `sophia-conformance/tests/direct_scanout.rs` | E `xtask/tests/direct_scanout_gate.rs` |
| `sophia-conformance` dock tests (`tests/dock.rs`) | E `xtask/tests/dock.rs`, `dock_launcher.rs` |
| `sophia-cli/tests/session_prepare_arguments.rs` (except the unknown-option refusal) | E `session_recipe_arguments.rs` (against the frozen `before_t027` oracles); S keeps the refusal |
| `sophia-cli/tests/session_prepare_inputs.rs` (recipe part) | E `session_recipe_inputs.rs`; S keeps the controls and `check-launch` parts |
| `sophia-cli/tests/session_application_arguments.rs` | E `session_recipe_application.rs` |
| `sophia-cli/tests/launcher_safety.rs` installed sections (271-377) | E `installed_launcher_safety.rs` |
| `sophia-cli/tests/launcher_safety.rs` Lom sections (44-53) | E `check_lom_gpu_content_proof_verifiers.sh` (pending Lom) |
| `sophia-cli/tests/launcher_safety.rs` remaining sections | S (on the reduced `run_sophia_session.sh`) |
| `session_launcher_recovery.rs` `tty_adapter_refuses_controls_before_queries_or_privileged_handoff` | E `session_tty3_launcher.rs`; S keeps the generic recovery tests |
| `xtask` `bemenu_artifact` tests | E `xtask/tests/bemenu_artifact.rs` |
| `sophia-runtime/tests/shell_bemenu_files.rs` | E `live-tests/tests/bemenu_files.rs` (9P only; asserts no `SOPHIA_SHELL_SOCKET` in Bemenu's environment and `wire=9p` on the negotiated line) |
| `sophia-conformance/tests/panel.rs` (3) | E `xtask/tests/panel.rs` (the same 3, on `xtask::panel::verify`) |
| `sophia x-authority-quickshell-smoke`, `x-authority-quickshell-software-smoke` (no test target; manual commands) | E `quickshell-probe` (`crates/quickshell-probe`; offline: `tests/evaluate.rs`, `tests/runner.rs`; the run itself needs an execution grant) and S `present_msc_ordering.rs` `present_selection_after_destroy_reports_bad_window_and_keeps_serving` (`6fda6f3b`) for the generic teardown answer ([quickshell](quickshell.md)) |
| `shell_descriptor_conformance_host` `--proof`, `--serve`, `--bar-proof` | S `shell_descriptor_modes.rs` (root `9bc6bb7`, 4 tests); Narthex product coverage waits on a 9P descriptor host (seam); GAP: no independent-client 9P test proves the work area changes only at commit (`--bar-proof`); likewise `--serve` has no independent peer. These are not coverage inherited from the IPC runs. They close only with root's descriptor-file contract and host plus an independent C SDK peer ([E1](E1-descriptor-hosts.md)) |
| `sophia-session/tests/shell_component_processes.rs` Bemenu tests (3) | Retired with product IPC (Bemenu 536d6b2), together with the interim external twins `bemenu_ipc`, `bemenu_session_ipc` and the IPC launch diagnostic `diag_bemenu_session_launch`; generic shell-component coverage stays S |

The live tests' Session-layer pins (`sophia-session`, `sophia-backend-live`)
and the shell client's `ipc-compat` feature went with the IPC twins; the
remaining pinned Sophia crates were `sophia-config`, `sophia-conformance`,
`sophia-engine`, `sophia-protocol`, `sophia-runtime` and `sophia-shell-client`.
The Quickshell probe adds `sophia-x-authority` and `sophia-backend-live`
(feature `gbm-probe`, the GPU variant's measurement and allocator), used only
by `crates/quickshell-probe`.

New external tests with no Sophia predecessor: `session_preflight`,
`session_host_paths`, `session_reap_last`, `session_wrapper_contract`,
`profile_mode`, `wm_default`, `wm_pair`, `package_desktop`, `installed_xtask`,
`product_artifact`, `nim_deps`, `nim_install`, `physical_inputs`,
`physical_runner_bounds`, `verify_archives`, `pins`, `provision_script`,
`xtask_alias`, and `quickshell-probe`'s `evaluate` and `runner`.

## Recipes (justfile) and xtask commands

| Sophia | Now |
| --- | --- |
| `just install-session` (`install_session_from_head.sh`) | `cargo xtask prepare-wm-pair`, `cargo xtask package-desktop`, `tools/install_live_session.sh` ([installed](installed.md)) |
| `just xtest-session` | the installed `sophia-hagia-xtest-session` entry |
| `just reload-wm` (`reload_policy_client.sh`) | this repository's `tools/reload_policy_client.sh`, an exempt operator tool |
| `just desktop-comparison-*`, `cargo xtask conformance desktop-comparison ...` | `cargo xtask desktop-comparison ...` ([desktop comparison](desktop-comparison.md)) |
| `just direct-scanout-*-gate`, `cargo xtask conformance gate direct-scanout` | `tools/direct_scanout_gate.sh` (prepared inputs; `xtask direct-scanout-gate`) |
| `just direct-scanout-probe`, `cargo xtask conformance run direct-scanout` (an unarchived probe run) | the gate runs the same probe (`direct_scanout_gate::run_probe`) and archives it; the unarchived probe entry is not kept |
| `just direct-scanout-archive`, `direct-scanout-verify` | S |
| `cargo xtask panel` | `cargo xtask panel` (the probe takes an explicit prepared `--sophia`; live mode is operator-only) |
| `cargo xtask conformance verify panel LOG` | `cargo xtask panel verify LOG` |
| `sophia x-authority-quickshell-smoke`, `x-authority-quickshell-software-smoke` | `quickshell-probe --renderer=gpu --render-node=...`, `--renderer=software` (device-hidden) |
| `just glxgears-benchmark`, `glxgears-shake` | `tools/benchmark_sophia_glxgears_tty3.sh`, `..._shake_tty3.sh` |

## Physical and operator gates

Every physical runner moved here and takes prepared inputs
([physical runners](physical-runners.md)). None runs in an automated gate in
either repository; their verifiers, archivers and preflights are covered as
above.

## Retained in Sophia (not moved)

The QEMU session harness and its scenarios; `verify_qemu_session_evidence.sh`;
`check_atomic_scanout_verifiers.sh`; the product-neutral live-session
verifiers (persistent, two-xterm, milestone 3, 4 and 5 GTK evidence) with their
fixtures and self-tests (this repository pin-reads them); the three-class
archive aggregate; the decoy configuration fixture read by
`sophia_conformance::profile`; the direct-scanout verifiers and archive
family; generic shell-component, policy-host and content-lifecycle tests.

## Pending

- Lom: the panel, launcher and dock gates, the GPU content proof and the
  content-proof step, pending the Sophia 9P seams ([Lom content](lom-content.md)).
- Bemenu: the new signed artifact preparation at Bemenu `536d6b25` (a slot and
  the operator's confirmation of the binary SHA before any live run).
