<!-- Provenance: the Lom-specific parts of Sophia docs/lom-content-implementation.md (the Lom content-lifecycle diagnostic, Lom's conformance client, Lom's readback conversion and renderer deadline, and the Lom status paragraphs of "Production admission remains closed") at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13). Sophia keeps the generic content records, admission, pools, reducers and device-hidden offline gates. -->
# Lom content

Lom is a native Sophia shell component (bar, and in later work popouts) that
renders its content with Vello and hands complete candidates to Sophia over
the shell content path. Sophia owns the generic content contract; this page is
Lom's side and the external gates that exercise it.

## Status: pending the Lom 9P lockstep

Lom `0d1ff046edd41de02fdf0948cdb0ff74d35a97e7` is 9P-only: `lom --serve`
requires a nonempty `SOPHIA_SHELL_9P_SOCKET` and refuses any presence of
`SOPHIA_SHELL_SOCKET` (empty, or both set). It writes
`lom_shell_transport schema=1 wire=9p2000.L revision=6 epoch=N` after
negotiation and before GPU validation; revision 6 and capabilities `0x783`,
`lom_gpu_admission` and the presentation and action records are unchanged.
`lom content-proof --socket` is 9P-only too (revision 6, capabilities `0x81`,
renderer failure and resource release required).

The pinned Sophia (`de776c68`) serves both of Lom's proof paths and the legacy
`--shell-process` shell over IPC only:

- `sophia shell-gpu-content-proof`
  (`crates/sophia-session/src/live_session/metadata_shell/gpu_content_proof.rs`);
- the content-proof conformance host
  (`crates/sophia-runtime/examples/shell_content_conformance_host.rs`);
- the legacy metadata shell that `--shell-process` starts
  (`metadata_shell.rs`, `metadata_shell/component_launch.rs`).

Independent shell components already select 9P through
`transport "9p2000.L"`. Root is adding a generic 9P option to the two proof
hosts and deciding how the panel runner converges. Until those land and the pin
moves, these stay pending and on the bounds pending list, with no IPC path
kept for them:

- `tools/run_current_lom_panel_gate_tty4.sh [launcher|dock]`;
- `tools/lom_gpu_content_hardware_proof.sh` and
  `tools/verify_lom_gpu_content_hardware_proof.sh`;
- `tools/verify_lom_panel_native_gate.sh` and `tools/probes/lom_workload/`;
- their self-test `tools/check_lom_gpu_content_proof_verifiers.sh`;
- the Lom recipe in `prepare-product-artifact` (still at its earlier signed
  revision).

When they land, the lockstep requires `lom_shell_transport` (wire
`9p2000.L`, revision 6, the negotiated epoch, ordered after negotiation and
before GPU admission), passes only `SOPHIA_SHELL_9P_SOCKET` with negatives for
the IPC variable present, empty or both set, updates the content-proof
invocation, moves the Lom artifact to `0d1ff046` with provenance, and converts
the three Lom scripts to prepared inputs.

## Lom's side of the content path

Lom converts Vello's straight-alpha RGBA readback into premultiplied B,G,R,A in
place, using rounded integer arithmetic in sRGB channel space and transparent
black at alpha zero. Its iterator produces maximal whole-row chunks without
duplicating the image. Content dimensions are checked against 4 MiB before GPU
rendering; the diagnostic preview has its own separately named bound.

The renderer uses the texture path and its own map/copy operation instead of
`imaging_vello`'s indefinite readback wait. One two-second deadline covers the
poll and callback. A failed job is retained and the renderer refuses new work.
This recovery budget is not presentation pacing, proof of GPU completion, a
bound on arbitrary driver calls or an aggregate GPU allocation budget.

Lom's conformance client derives the output identity from
`ContentOutputFacts`, raises a frame demand and uses only the returned permit;
it requests its panel allocation rather than having a fixture injected behind
it. Lom's `content-lifecycle` diagnostic, together with an independent C
client, runs in Sophia's deterministic lifecycle host (cross-language
acceptance, not native display acceptance).

## Evidence limits

No render-node grant, installed-profile change or shell replacement follows
from content acceptance alone. GPU permission is a separate explicit launch
grant, independent of content negotiation; the accepted policy permits direct
rendering on supported stock Linux without promising a hard aggregate GPU
memory limit. Native acceptance still requires retirement, activation,
anchoring, consumed outside dismissal, restart and output changes, observed on
the physical gates above once they are unblocked.
