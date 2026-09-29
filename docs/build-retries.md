# Build refusal and cache reuse, 2026-09-29

The morning attempt for `niltempus-f9fdf4d2f98353bf6265` refused
`prepare-wm-pair` because the reviewed manifests bound Bubblewrap 0.12.0,
while the host now has the packaged Bubblewrap 0.13.0. Its log is under
`~/.local/state/sophia-niltempus-desktop/builds/` in
`niltempus-f9fdf4d2f98353bf6265-4226758301/prepare-wm-pair.log`.
The bootstrap compile took 20.058 seconds and the refusal another 59.551
seconds. The preceding evening attempt's Sophia build log has no final result;
it does not establish a compiler failure or successful package.

Both dependency manifests were regenerated from their existing explicit pins,
source commits and package store using the bound `nim-deps draft` command.
Comparison with the prior manifests found exactly one changed tool record in
each: Bubblewrap. Package provenance, source identities, full file inventories,
compiler, standard library, configuration and other host package records match.
The installer tests' nested Bubblewrap isolation proof passes with 0.13.0.

The new reviewed manifest digests are:

- Hagia: `c7e220996e19086606b609e2441cb873462c3655307e54136815bb70810c7d92`.
- Narthex: `0ce86f1fe36e1e1f8e3e631ca72d03bfec4703e16f557038a0156ffed35cc0c5`.

The old manifests remain unchanged. A new plan binds the replacements.

The subsequent build passed WM-pair preparation (4m2.468s), desktop packaging
(4m48.937s) and package verification, then exposed a second missing prerequisite:
Lom's locked SDK revision `c1323401b7e336606408499b13097d1270a2319d` was absent
from the default Cargo cache. Explicit `cargo fetch --locked` provisioned it.
The installer now runs an offline locked dependency check for Lom before any
desktop compilation, so future missing-cache failures precede those stages.

Previously each attempt allocated fresh Cargo targets for the packaging-tool
bootstrap, Sophia release build and packaging-tool release build. These targets
now persist under the private cache's `package-targets`, under the existing
exclusive build lock. Attempt logs, signed source staging, output packages and
source/toolchain/release checks retain their original ownership and checks.
Cargo decides freshness on every invocation; completed packages are still
sealed and verified. Local crates staged at new paths may still rebuild, so
this removes cold dependency builds without promising a fully incremental
release build.

Toolchain verification now checks executable paths and hashes before its
slower probes, naming the changed tool and both identities on refusal. Full
toolchain probing still follows a successful early check.

Validation: all installer tests, the full xtask test suite, and strict xtask
all-target clippy pass. Added controls retain cached outputs across retries,
keep logs separate, reject redirected/shared caches, and detect changed tool
bytes or symlink targets without executing the changed tool. These checks do
not qualify a graphical session or authorize desktop activation.
