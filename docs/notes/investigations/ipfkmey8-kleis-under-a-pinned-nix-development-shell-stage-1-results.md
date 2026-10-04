---
id: ipfkmey8
date: 2026-10-03
kind: investigation
status: investigating
tags: [investigation]
---
# kleis under a pinned Nix development shell: stage 1 results

## Question

Can a pinned Nix development shell build and gate kleis from exactly the
reviewed dependency closure, offline, and what does it cost? This is stage 1
of the [Nix flake plan](../plans/c7g8cnd5-nix-flake-prototype-for-reproducible-desktop-builds.md)
(n002).

## Evidence

- **Candidate:** kleis branch `nix/devshell`, commit `81400dcf` (signed), on
  `342a4a50`. It holds `flake.nix`, `flake.lock` and `nix/`.
- **Pins:** nixpkgs `c59305bab2065cfecc4944690d9eedbb56f3a9fa`; Nim registry
  `nim-lang/packages` `09f05a91e9fc09b3e4626aa4d8f9a067716552ed`.
- **Evidence directory:** `development-evidence/n002-kleis-devshell-01/`.
- **Host:** Nix 2.30.5 on Void, with nix-daemon enabled through runit and
  `/etc/nix/nix.conf` set to `sandbox = true` and
  `experimental-features = nix-command flakes`. Your user is not trusted.

| Check | Result |
| --- | --- |
| Nix sandbox on Void | A sandboxed `runCommand` built, and `/home/niltempus` was invisible inside it |
| Toolchain | Nim 2.2.12 (equal to the review); gcc 14.4.0 (host review 14.2.1); nimble 0.24.1 (host 0.22.3); nph 0.7.0 (host: a prerelease build) |
| Reviewed closure | 51 of 51 manifest file rows match by sha256 and size; 0 extra files. The check runs inside each package derivation. A manifest with one altered hash fails the build and names the file |
| Gates, with the network unshared (`nix develop -i --offline`, `nimble --offline --useSystemNim`) | `nimble test` 98 OK; `fmtCheck` 19 files unchanged; `nimble build` gives a binary with sha256 `ecc1c297b08bebca71937925b65f4f6f1d577d384fb8ca77d8812186b7fcf9dd` |
| Production `LockFileService` (scratch harness, outside the Sophia tree) | `matrix_images=3 presented=7 retired=5 indigo=true back_to_matrix=true blank=true` |
| Offline rebuild | Each of the 4 package derivations rebuilt with `nix build --rebuild --offline`, with identical outputs |
| Time | Sandbox check, cold: 16.2 s. Dev shell, cold, including downloads: 12.5 s. Offline gates (tests, format check, build): 18.8 s |
| Store | Dev shell closure 1.2 GiB in 160 paths; `/nix/store` 1.8 GB in total |

## Finding and resolution

**The dev shell works and stays faithful to the reviewed closure.**
- It rebuilds the reviewed Nim closure byte for byte from upstream revisions.
- It passes every kleis gate offline.
- It presents through the real lock export.
- Its package derivations rebuild offline to identical outputs.

**Findings that shape stage 2:**

1. **Nimble 0.24 needs its registry even offline.** It reads the package
   registry before every task, even with `--offline` and every dependency
   installed, and otherwise downloads `packages.json`. The registry is now a
   pinned input. The host's nimble 0.22.3 avoids this only through its
   mutable `~/.nimble` cache.
2. **The binary is tied to the Nix store.** It links Nix's glibc loader from
   `/nix/store`, so it runs only where the store is visible. Sophia's
   lock-provider sandbox may not expose `/nix`. The release builder's
   host-built kleis remains the deliverable. Stage 2 or 3 must choose: either
   expose the store to the provider, or keep host-toolchain products.
3. **The toolchain differs from the review.** gcc 14.4.0 against 14.2.1, nimble
   0.24.1 against 0.22.3, and the host's prerelease nph. The binary hash
   therefore differs from the host-built kleis. These are recorded
   differences, not failures.
4. **The dev shell provides no isolation.** It standardizes tools and
   environment only, so the bwrap-isolated product build stays the gate.

## Validation and remaining work

**Stage 1's exit is met:**
- identities;
- time;
- store size;
- an offline rebuild;
- the real `LockFileService` run.

**The remaining part of n002's exit is the decision on stage 2.** It belongs to
niltempus and w9:pT. My recommendation: admit stage 2 as a measurement. Build a
sandboxed kleis derivation, run the equivalence and gate checks, then build it
twice in different directories (for n001), and only then consider replacing a
bwrap gate. Deployment stays unchanged until finding 2 is decided.

## Connections

- [Nix flake plan](../plans/c7g8cnd5-nix-flake-prototype-for-reproducible-desktop-builds.md):
  stage 1 of n002.
- [Product build reproducibility (n001)](rm8sjc2m-desktop-product-builds-are-not-byte-reproducible-across-build-directories.md):
  the package derivations rebuilt identically here. Product binaries are not
  yet measured under Nix.
