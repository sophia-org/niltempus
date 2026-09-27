<!-- Provenance: the Hagia operator gates moved from Sophia docs/project-hagia.md (the tools/hagia-proof and physical policy gate section of "1. Geometry Proof") and docs/validation.md (the Hagia policy smokes and the native session gate) at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13). Sophia keeps the generic policy-client semantics; this file is the Hagia-specific operator procedure. -->
# Hagia gates

Hagia is the native policy client (`sophia_wm_v1`) and Narthex its shell
partner (`sophia_shell_v1`). Sophia specifies and tests the policy and shell
contracts generically; the gates here prove this particular client pair on
real hardware. Every gate takes prepared physical inputs and never builds
([physical runners](physical-runners.md#what-every-runner-requires)); the
Hagia and Narthex binaries come from their signed trees with reviewed Nim
dependency manifests.

The live Sophia session supplies the per-session policy checkpoint to the
policy client as `SOPHIA_WM_POLICY_CHECKPOINT` inside its owner-only policy
endpoint directory. It survives supervised Hagia child replacement and is
removed at session teardown. It is never transferred to another policy process
or treated as portable configuration.

## Native session gate

```sh
tools/hagia-native-proof        # = tools/run_current_hagia_native_gate_tty4.sh
```

From a logged-in `/dev/tty4`. It requires clean signed Sophia, Hagia and
Narthex checkouts and this repository's clean signed HEAD. The desktop profile
is Hagia's canonical `examples/config/default.kdl`, taken from Hagia's staged
signed tree (`--profile=hagia:examples/config/default.kdl`), unless a reference
run names another tracked file with `SOPHIA_HAGIA_NATIVE_PROFILE`. The profile
is checked by both `hagia config check` and `sophia config check` before
takeover. The session runs through `tools/hagia_native_session_gate.sh` and the
external launcher; its evidence binds all three commits, the three binary
digests and the profile digest, and is archived by
`tools/archive_hagia_native_session_run.sh` under
`$XDG_STATE_HOME/sophia/promotion/hagia-native-runs/` (integration schema 1:
the archive binds this repository's signed commit too). Promoted runs are
re-verified offline by `cargo xtask verify-archives`; runs made before the
integration binding verify only with `--legacy`, which reports "integration
identity unavailable".

## Physical policy gate

```sh
tools/hagia-proof               # = tools/run_current_hagia_policy_gate_tty4.sh
```

From a logged-in `/dev/tty4`. The runner prepares Sophia, Hagia and Narthex,
checks Hagia's canonical default profile from its staged tree, and enters
`tools/hagia_policy_physical_gate.sh` through
`tools/start_sophia_hagia_policy_tty4.sh`. The gate requires an explicit arm
variable, real Hagia and Kitty binaries, a named input seat and two connected
outputs. The operator exercises fullscreen, native layout cycling, maximize,
minimize/restore, output movement, active-output actions and the `Super+P`
switcher around one policy restart and one shell restart. Kitty shows one
instruction at a time and advances only on the corresponding committed-action
evidence. The guide names the compiled public shortcut chords
(`Super+Shift+F` fullscreen, `Super+Shift+B` minimize, `Super+Alt+B` restore);
`tools/check_hagia_physical_matchers.sh` ties them to the compiled profile.

The verifier (`tools/verify_hagia_policy_physical.sh`) requires ordered
commits, a nonempty checkpoint load and reconciliation after the restart,
output-change evidence, per-head nonzero presentation, the exact physical text
and clean session health. The archive
(`tools/archive_hagia_policy_physical_run.sh`) rechecks every signature and
cross-record identity. The installed guide has a ten-minute physical-sequence
safety deadline inside an eleven-minute global ceiling; these are not soak
criteria.

## Critical path

```sh
tools/run_current_critical_path_tty4.sh
```

Runs the policy, mirror-group and mixed-output gates in sequence from one
console, each with its own prepared inputs and identity checks.

## Offline Hagia smokes (no hardware)

```sh
SOPHIA_HAGIA_BIN=/abs/hagia tools/hagia_live_session_smoke.sh
tools/hagia_client_lifecycle_fault_smoke.sh
tools/hagia_owner_settlement_fault_smoke.sh
```

These run the prepared release Sophia (never a debug build in a checkout)
against a supervised Hagia, terminate it at armed points
(`SOPHIA_HAGIA_FAULT_AFTER`, handled by the restart fixture
`tools/fixtures/hagia_restart_once.sh` and Hagia's own fault hooks) and
require epoch advancement, readiness of the replacement, retained layout and
clean session and layout health.

## Hagia's own conformance

Hagia's independently written decoder and proof client are Hagia's gate, run
in its own repository against a pinned Sophia tree (for example
`SOPHIA_ROOT=/abs/staged/sophia tools/check_sophia_policy.sh` in the Hagia
checkout). Sophia's authenticated black-box policy host covers the Rust, C and
Hagia peers generically.

## Retained evidence

Promotion archives keep their original identities; see the
[operations runbook](operations.md#native-evidence-workflows). Archive
`0004` (Sophia `9ca384a9`, Hagia `074e374c`) promoted the protected
metadata-broker row and Tier-0 archive `0005` (Sophia `a954745e`, Hagia
`8adf2655`) the indicator row; both predate the integration binding and verify
with `--legacy`.

## Workspace policy

Hagia's globally numbered, monitor-owned workspaces are Hagia semantics; see
[Hagia workspaces](hagia-workspaces.md).
