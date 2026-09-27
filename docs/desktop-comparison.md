<!-- Provenance: moved from Sophia docs/validation.md, section "Deferred Same-Hardware Comparison", and the comparison guidance in docs/development-tooling.md at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13). Rewritten for this repository's `cargo xtask desktop-comparison` and prepared inputs. The row procedure is validation/desktop-comparison/README.md. -->
# Same-hardware desktop comparison

The comparison measures Sophia (with the Hagia/Narthex pair), XLibre+xmonad
and niri on the same two-output rig. It is deferred and incomplete; resume it
only for an explicitly selected stable candidate or a named performance
investigation. Its 36-row requirement applies to comparison verification, not
to using Hagia or closing any Sophia milestone. Reports always end in
`verdict=none`: reference performance is never a Sophia correctness
threshold, and the XLibre+xmonad entry is a direct reference desktop that never
receives a Sophia policy socket.

The typed owner is the `desktop-comparison` crate
(`crates/desktop-comparison`, 26 unit tests), driven by:

```sh
cargo xtask desktop-comparison install-reference XLIBRE_SOURCE PREFIX
cargo xtask desktop-comparison prepare RUN
cargo xtask desktop-comparison prepare-soak SOAK_RUN
cargo xtask desktop-comparison gate RUN
cargo xtask desktop-comparison status RUN
cargo xtask desktop-comparison attest RUN SUPERVISOR_PID [CRTC]
cargo xtask desktop-comparison preflight RUN
cargo xtask desktop-comparison qualify RUN
cargo xtask desktop-comparison capture RUN
cargo xtask desktop-comparison finalize RUN
cargo xtask desktop-comparison replay RUN ATTEMPT
cargo xtask desktop-comparison verify RUN
cargo xtask desktop-comparison report RUN
```

## Inputs

Nothing is built by the comparison. Preparation requires:

- `SOPHIA_SOURCE`: the clean signed Sophia candidate; `SOPHIA_ROOT`: its
  pinned tree (Sophia's hashed `run_sophia_session.sh`, `sophia_tty_mode.py`
  and `lib/session_preparation.sh` are read from there, named `sophia:PATH`);
- `SOPHIA_DESKTOP_COMPARISON_{SOPHIA,HAGIA,NARTHEX}_BIN` and
  `SOPHIA_DESKTOP_COMPARISON_XLIBRE_PREFIX`: absolute prepared paths, taken
  from `cargo xtask prepare-physical-inputs` (with the Hagia/Narthex halves
  and their reviewed Nim dependency manifests) and the installed XLibre
  reference; there is no HOME, sibling-checkout or target default;
- this repository clean with a signed HEAD.

The schema-4 manifest binds both the Sophia candidate commit and this
repository's commit. Repository-owned inputs (the stack configurations,
isolated profiles, capture adapter `tools/desktop_comparison_tty3.sh`, the
tracefs helper, the session launcher and the local Firefox fixture) are hashed
from this repository; `validation/desktop-comparison/**` is byte for byte the
pinned copy.

## Procedure

Preparation verifies the exact pinned Kitty, Firefox and niri versions, pins
the candidate and reference-stack identities and records the common two-output
topology and the kernel, Mesa and GPU identities. The schedule rotates the
three stacks across three repetitions of four workloads (Kitty 60 s, the
loopback-only animated Firefox fixture, 120 resize requests, a 16-Kitty launch
burst): 36 required rows. `prepare-soak` creates a separate one-row Sophia run
for an optional two-hour durability check that never blocks the matrix.

`gate` owns one complete TTY3 row: it checks the clean prepared commits and
the host tool versions, verifies the prepared binaries, selects the next typed
stack, launches it without an operator application, attests its supervisor,
resolves DP-1's active CRTC, captures and tears down. `qualify` requires
physical cursor motion and a click on four targets before the first Sophia
row. `capture` rejects a controller or workload launcher inside the measured
supervisor tree, samples stack, workload and aggregate resource populations
separately and uses a private tracefs instance for authoritative kernel DRM
completion timestamps. `finalize` proves the attested supervisor is gone and
seals the row. Passive replay derives the single schema-4 sample only when the
visibility series, duration, resource cadence, frame monotonicity, resize
population, crash, sample-loss and teardown checks pass.

`verify` requires the exact complete matrix; `report` keeps memory,
allocation, CPU/fault, process/thread/fd, launch/settle/resize and kernel-frame
distributions with `verdict=none`.

See `validation/desktop-comparison/README.md` for the row-by-row operator
workflow and profile admission.
