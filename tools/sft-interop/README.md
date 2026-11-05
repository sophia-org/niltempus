# Real Lom / Sophia Frame Transactions integration

This isolated workspace tests the pinned, published Lom CPU renderer and service
against Sophia's actual Session component connection, authenticated file server,
owned-ring copy worker and candidate/resource stores. It does not change xtask's
or the other desktop tools' dependency pins.

Run from the integration checkout:

```sh
cargo-slot niltempus cargo test --manifest-path tools/sft-interop/Cargo.toml --locked
cargo-slot niltempus cargo clippy --manifest-path tools/sft-interop/Cargo.toml --all-targets --locked -- -D warnings
```

The same deterministic 1920x24 panel runs at r10 and explicitly enabled r13.
Four workspace-state changes establish one full image, then three regions, then
two one-region updates. Every region's CPU bytes must be identical between the
two paths. Unchanged regions must keep their exact resource identities and
bytes, while the changed workspace region must change both. The legacy run
requests four demand/permit exchanges; SFT requests only the two establishment
exchanges. Lom must consume all four Presented records before the test ends.
Both connections and workers then tear down and the Session epoch must become
quiescent with its temporary socket directory removed.

Labels stay constant: changing a label deliberately chooses Lom's full repaint
path. This fixture changes the workspace active bit, which exercises its existing
retained-region policy without changing the product to suit the test.

Scope: authentication verdicts, allocation policy and display completion are
supplied by the fixture. No Factotum password check, protected process launch,
GL, device access, native scanout, CLI executable or CPU/frame measurement is
claimed. The real public service and CPU worker run in the producer thread; the
host's image/candidate joins use their normal code. Native tear-free completion,
matched performance and the reference producer remain separate gates.
