---
id: c7g8cnd5
date: 2026-10-03
kind: plan
tags: [plan, milestone]
---
# Nix flake prototype for reproducible desktop builds

## Scope and exit

niltempus hand-rolls pinning, SDK vendoring, Nim and toolchain manifests,
private cargo homes, nested bwrap build sandboxes, and release and component
install ordering. That layering caused avoidable failures in the CPU and kleis
release fixtures (`/tmp` hidden from nested builds, a mismatched
`XDG_CONFIG_HOME`). This plan prototypes Nix flakes for the build half, in
stages that each stand alone. niltempus approved it on 2026-10-04, and its
implementation starts only after that release is handed off. It changes no
installer code and migrates no caches during a release.

**Stage 1 (n002): kleis dev shell.**

- **Toolchain.** A `devShell` pins the reviewed closure: Nim 2.2.12, nimkdl
  2.1.0, bigints 1.0.0, graphemes 0.12.0, unicodedb 0.13.0, gcc 14.2.1, and the
  vendored C desktop SDK v0.8.0.
- **Gates.** kleis compiles and passes its tests inside
  `nix develop -i --command ...`, then presents through Sophia's real
  `LockFileService` with the C peer.
- **What to record:**
  - source identities and output identities;
  - build time;
  - Nix store size;
  - an offline rebuild.

**The dev shell standardizes tools and environment only.** `nix develop -i`
provides no filesystem, device or network isolation and proves no hermetic
build, so the existing bwrap gates stay.

**Stage 2: kleis derivation.**

- **Sandboxed build.** `nix build` in Nix's sandbox. It may replace a bwrap
  gate only after it passes positive and negative controls equivalent to that
  gate's.
- **Reproducibility.** Measure it with two builds (see
  [n001](../investigations/rm8sjc2m-desktop-product-builds-are-not-byte-reproducible-across-build-directories.md)).
- **Independent repositories.** Each repository keeps building on its own.
- **One reviewed SDK.** The SDK input pins one reviewed, released C SDK, and
  the contract byte-equality checks remain, as flake checks.

**Stage 3 (later design, not admitted).**

- **Install configuration.** A Home Manager or `nix profile` configuration for
  the session and its components.
- **What it must prove first, on Void:**
  - the session entries;
  - the root-owned PAM stack and helper;
  - activation;
  - component and profile rollback, together.
- **Atomicity.** Nix profile generations do not make those external, root-level
  changes atomic.

**The exit for this plan is a recorded stage 1 result**: identities, time,
store size, offline rebuild and the real `LockFileService` run. It is followed
by a decision, with evidence, on whether stage 2 is admitted.

**Non-goals.**

- Removing the separate SDK repositories, signed commits, the review process
  or integration tests such as the release sequence.
- Assuming Nix alone makes builds reproducible.

## Task details

- **n002.** Stage 1 as above. The dev-shell gate runs beside, not instead of,
  the existing kleis gates and the bwrap-isolated product build. Record
  results in a new investigation linked here.

## Connections

- [Reproducibility investigation (n001)](../investigations/rm8sjc2m-desktop-product-builds-are-not-byte-reproducible-across-build-directories.md):
  the build-path evidence this plan measures for kleis.
- Proposal record:
  `development-evidence/nix-flake-proposal-01/PROPOSAL.md`.
- Nix `nix develop` reference, including the build environment and
  `--ignore-env`:
  https://nix.dev/manual/nix/2.28/command-ref/new-cli/nix3-develop.html
