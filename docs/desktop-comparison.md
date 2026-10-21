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


## October 21: shell-path reference integration (t331), separate from wmbench

Niltempus's t331 direction replaces the former IPC +10% gates with external
Wayland/XLibre performance references. IPC remains diagnostic. This does not
replace the older whole-desktop comparison's correctness scope or promote its
`verdict=none` rows into shell-path evidence. Numerical t331 ceilings await the
operator's choice; the proposed R=2.0 is not approved.

The standalone reference sources are under `tools/external-bar`, imported from
Claude wB:pH and Muse wB:pJ's October 21 handbacks. Twenty rev-4 source hashes
were checked against their receipts and preserved before local fixes in
`t331-d4-owned-copy-20261021/external-bar-handback-02`. Earlier rev-3 sources and
failures remain separately retained. They add no Sophia production dependency.

Build and offline checks:

```sh
make -C tools/external-bar
python3 tools/external-bar/test/test_account.py
tools/external-bar/build/fill-parity
```

`headless_smoke.py` creates a new private output directory, runs bubblewrap with
private namespaces and no DRM/input nodes, and launches only a headless pixman
River and the reference client. It always terminates that owned compositor.
It is a protocol smoke, not a native performance run. Pass explicit compositor,
client, lifecycle and output paths; binaries are recorded by path/hash and never
copied into evidence. The wrapper's first attempt could not create `/runtime`
under its read-only root; the corrected private path is `/tmp/runtime`.

The unmodified rev-3 client failed with `no presentation clock_id`. Registry
bind requests need another roundtrip before checking initial clock events.
The local fix also destroys presentation-feedback proxies, timestamps before
the surface commit, and calls fallocate in the fresh path (the delivered code
said fallocate but only called ftruncate). Both revised fresh and reuse smokes
completed 8/8 feedback rows after two warmups, with two buffers. This qualifies
neither physical scanout nor the native workload. Sources are not yet frozen
as a seated campaign.

Use an exact River binary. The delivered `~/.local/bin/river` is
0.5.0-dev.75+d4fef52, whose empty-init run explicitly closed the layer surface
because no WM bound river_layer_shell_v1. `/usr/bin/river` 0.3.7 with `-c true`
passed the isolated smoke. The original 0.5 launch recipe and binary-string
protocol table are not qualification. A 0.5 native arm needs its separate WM
and complete process accounting; a 0.3.7 arm needs its own frozen protocol and
GPU evidence. Do not silently switch versions within a pair.

The accepted comparison axes are fresh lifecycle on all gated arms, matched
buffer count (two for the current peer), heads/modes, visuals and GPU-backed
composition; fill-start to presented is the proposed latency quantity. Reuse
is a separate currently ungated matrix for ordinary external behavior and the
B experiment. Never join fresh and reuse. Whole-desktop wmbench gets a separate
Sophia/XLibre/River campaign under t289, including backend and unsupported-row
reporting; its X compatibility clients do not exercise shell image admission.

The following issues still prevent a native performance verdict:

* The Wayland bulk surface is visible; X's bulk pixmap and Sophia's existing
  accepted/released bulk resource are offscreen. Matching byte counts does not
  match their rendering work. Define and implement a common bulk workload.
* Pin the actual output and geometry. The received Wayland client passes NULL
  for its output and ignores configured dimensions; this is not proof of (0,0)
  on a specified head. Match counted-frame shade generations too: the historical
  Sophia converter drops warmup but references currently restart their shade
  counter. The fill routine itself passes its byte/guard parity check.
* Synchronize CPU windows after warmup and before teardown on every arm, with
  server and client counters collected the same way. The reporter currently
  sees caller-selected snapshots and whole-process client rusage, so it cannot
  establish that interval. Its output is explicitly diagnostic-only. Missing
  client CPU remains unavailable, never zero; the Sophia converter no longer
  disguises a CPU upper bound as getrusage or invents zero major faults.
* Establish the composed X scanout endpoint. XLibre source at 79dee7378 rejects
  direct flip of a Composite-redirected window (`present_scmd.c`); its copy path
  copies into the window drawable and emits PresentCompleteModeCopy
  (`present_execute.c`). A client Present timestamp alone therefore supplies
  no association to xcompmgr's subsequent composed frame. Also distinguish Skip,
  client-receipt fallback and SHM copy completion from physical presentation.
* The Sophia converter consumes a schema-2 xtask join, checking exact offer
  identities and duplicate samples; its input must be re-derived by the native
  join validator from captured evidence. It does not independently certify
  kernel sources or complete physical heads. The historical 880-row conversion
  is a parser check, not a new native capture.

The reporter now handles spaced clock notes, rejects duplicate header fields,
malformed/noncontiguous rows, mixed lifecycles, reversed timestamps, changed or
empty/duplicate PID sets, changed tick rates and decreasing counters. Quantile
sample counts stay counts rather than being divided by 1000. These controls
prevent several misleading reports but are not the unfinished gate analyzer.
No X server smoke, seated reference pair, x13 run or GPU parity is claimed.
