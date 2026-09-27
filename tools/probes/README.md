<!-- Provenance: the Lom sections of Sophia tools/probes/README.md (lines 3-41 and 245-252) at 9fcaec782ce4fe9978568c0466ee17a78b3d4571 (Sophia rule 13), adapted to explicit inputs. -->
# Attended Lom, Bemenu and Provlita gates

## Lom protected GPU/content proof

`tools/lom_gpu_content_hardware_proof.sh` launches the exact prepared Lom
artifact's `--serve` client through Sophia's production metadata-shell protection policy.
The private domain receives one render node under its real kernel basename and
a generated read-only sysfs discovery view for that same device, with no card
node, host sysfs tree, input device, display socket or network. Lom validates
the enumerated Vulkan adapter's DRM render major/minor before rendering a 256x24 Vello
panel and sends its real complete content candidate. The host verifies the
nonempty immutable pixels, returns the real `RendererFailed` outcome, and
requires resource and backing cleanup. This is a hardware render and protocol
proof; it deliberately records `native_presentation=false` and does not acquire
DRM master.

```sh
cargo xtask prepare-product-artifact lom LOM-REPO SIGNED-COMMIT LOM-ARTIFACT-DIR
SOPHIA_LOM_GPU_PROOF_ARM=1 SOPHIA_SOURCE=/abs/sophia SOPHIA_GATE_BUILD_DIR=/abs/private-build \
SOPHIA_LOM_ARTIFACT=LOM-ARTIFACT-DIR SOPHIA_LOM_COMMIT=SIGNED-COMMIT \
SOPHIA_LOM_SHA256=BINARY-SHA256 SOPHIA_LOM_CONFIG_SHA256=CONFIG-SHA256 \
    tools/lom_gpu_content_hardware_proof.sh
```

Waiting on seam D: at the pinned Sophia revision the proof command is
Lom-named. Until Sophia's generic `shell-gpu-content-proof` command is approved
the script refuses (exit 3) before building anything.

The separate `tools/run_current_lom_panel_gate_tty4.sh` is the native acceptance
candidate. Run it only from tty4 after ending the graphical session, with
`SOPHIA_LOM_NATIVE_GATE_ARM=1`. Source authorization: `SOPHIA_SOURCE` must be a
clean checkout whose HEAD is exactly the signed revision in `pins/sophia.toml`;
the gate stages that revision's exact tree into the private
`SOPHIA_GATE_BUILD_DIR` (tree hash proven) and reads, builds and executes
Sophia only from there. Artifact binding: it runs only prepared artifacts
(`SOPHIA_{LOM,HAGIA,BEMENU,PROVLITA}_ARTIFACT`) whose commit object and
binary (and configuration) match the operator's `_COMMIT`, `_SHA256` (and
`_CONFIG_SHA256`); the artifact manifest is unsigned and binds nothing alone.
It
first runs the protected GPU/content proof while Sophia does not own the display,
then runs the 90-second normal-exit workload (110-second failure watchdog). A failed prerequisite therefore
stops before graphics takeover and retains the client's boundary error. The native
stage restores the TTY through the existing session harness and retains exact
identities and structured session diagnostics. A machine-independent
gate or isolated proof cannot establish that the bar was visible; the attended
run still requires the operator to confirm its placement and appearance.

Wait ten clock ticks after both bars appear, then make 20 state-changing workspace
clicks per output within 60 seconds (40 total). No clicks during warmup or after
the 40; wait for automatic exit. The checked-in candidate budget is ACK p95/max
50/100 ms and exact native retirement p95/max 150/300 ms. These are predeclared
workload acceptance values, not claims of historical approval or driver guarantees.
The launcher copies/hashes budgets and profiles before GPU use, checks them again
after proof and session, and refuses any reused evidence directory. It requires
normal exit 0, existing strict health/recovery proof, complete causal outcomes,
bounded in-run inventory and exact zero final content accounting. See
[workload evidence](lom_workload/README.md) for scope and interpretation.

### Independent native launcher

The [three-component dock smoke](dock/README.md) adds `lom-test dock` for independent
Lom, Bemenu and Provlita processes. Its profile generator and host transcript
reader are Rust `xtask dock` commands; visual/native acceptance remains attended.

The [native launcher smoke](native_launcher/README.md) adds `lom-test launcher`
without changing plain `lom-test`. It retains separate component identities,
per-role GPU policy and host presentation/accounting evidence. Its offline
controls substitute builds/VT/native effects; no automated physical run is part
of those controls. See the linked readiness qualification before an attended run.
