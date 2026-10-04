---
id: rm8sjc2m
date: 2026-10-03
kind: investigation
status: investigating
tags: [investigation]
---
# Desktop product builds are not byte-reproducible across build directories

## Question

Two release builds of identical product sources produce different product
binaries. Is that only build-path dependence, and what does it limit?

## Evidence

The trigger was the independent audit of the successor release
`niltempus-0d029ccf6b7dac392a0f` against its reviewed predecessor
`niltempus-6a6440cabe2b23a63de1`. Their plans differ only in the niltempus
commit and reference, the installer hash and the release id. Sophia, Hagia,
Bemenu, Lom, Narthex, the profile, the C SDK revision and the Nim dependency
manifests are identical. Audit records:
`development-evidence/t295-sdk-release-01/combined-release-0{1,2}/AUDIT-pX-0{1,2}.txt`.

Packaged binaries compared with `cmp -l` (differing bytes):

| Binary | Source changed | Differing bytes | Size |
| --- | --- | --- | --- |
| `sophia` | no | 0 | equal |
| `lom`, `sophia-factotum`, `sophia-factotum-pam`, `active-session-preflight` | no | 0 | equal |
| `bemenu-sophia` (C) | no | 184058 | equal |
| `hagia` (Nim) | no | 49282 | equal |
| `narthex` (Nim) | no | 20088 | 604320 to 604328 |
| `sophia-integration-xtask` (Rust, from niltempus) | yes | 48 | equal |
| `niltempus` | yes | (expected) | |

The embedded strings show what differs:

- **`bemenu-sophia`:** its absolute build directory,
  `.../state-home/sophia-niltempus-desktop/builds/<release-id>-<random>/bemenu`.
- **`sophia-integration-xtask`:** `/tmp/sophia-integration-artifact-<random>/source/crates/xtask`.
- **`hagia` and `narthex`:** Nim type-info names (`NTIv2__<hash>`), whose hashes
  change with the build path. Narthex's size also changes.

These are observations from one pair of builds. The cause in each product
(debug info, `__FILE__`, `CARGO_MANIFEST_DIR`, Nim's path-derived hashes) is
inferred from the strings, not yet confirmed by a controlled rebuild.

## Finding and resolution

The C and Nim product builds, and the xtask helper, embed their build
directory. Every release build uses a fresh, randomly named directory, so
identical sources do not give identical bytes. The Rust desktop binaries above
did not change across these two builds.

**What this limits.** A release is audited, tested and installed as one exact
artifact, identified by its file hashes. That remains sound. What is lost is
comparing two builds of the same sources byte for byte: a rebuild cannot
confirm an earlier artifact, and a differing product binary does not by itself
show a source change. Audits must compare plans and sources, not product bytes.

Not yet resolved. Candidate fixes per product: build in a fixed path, or remap
the path (`-ffile-prefix-map`/`-fdebug-prefix-map` for C, `--remap-path-prefix`
for Rust, and a fixed project or nimcache path for Nim), then verify with two
builds in different directories. A sandboxed build in a fixed path (for
example a Nix derivation under `/build`) removes one cause. It does not on its
own guarantee reproducibility: timestamps, build ids, link order and toolchain
identity still need checking.

**Controlled confirmation for Nim (2026-10-04).** In the kleis Nix
derivation (`nix/devshell` `d8430385`), two sandboxed builds at the same `/build`
path still differed, only in `NTIv2__<hash>` symbols, while the Nim cache lived
beneath a randomly named `HOME`. With that `HOME` path fixed, the rebuilds were
byte-identical. Nim's type-info hashes depend on the build and cache paths, so
the niltempus product builder's per-run `--nimcache` and scratch paths
(`product_artifact.rs`) are the confirmed cause class for Hagia and Narthex.
Bemenu's and the xtask's embedded paths are still inferred from strings.

## Validation and remaining work

The exit is two release builds of the same plan, in different directories,
with byte-identical product binaries; or, per product, a documented reason
and an accepted exception. Tracked as n001 in `todo.md`. The Nix flake
prototype (n002) measures reproducibility for kleis as one data point; it
does not close this.

## Connections

- [Nix flake prototype](../plans/c7g8cnd5-nix-flake-prototype-for-reproducible-desktop-builds.md):
  measures a fixed-path sandboxed build for kleis.
- `docs/component-updates.md` and `installer/README.md`: release identity and
  component selections, which rely on exact artifacts.
