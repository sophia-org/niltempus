---
id: ui2l9fuy
date: 2026-10-03
kind: investigation
status: investigating
tags: [investigation]
---
# Nix flakes for every desktop product and Sophia: phase A results

## Question

Can every binary in the niltempus desktop release be built in Nix's sandbox
from one reviewed nixpkgs revision, reproducibly, and pass its repository's
gate there? This is phase A of the
[Nix flake plan](../plans/c7g8cnd5-nix-flake-prototype-for-reproducible-desktop-builds.md)
(n002), after [stage 1 and 2](ipfkmey8-kleis-under-a-pinned-nix-development-shell-stage-1-results.md).

## Evidence

All flakes pin nixpkgs `c59305ba`. The Rust flakes add crane and
rust-overlay for the toolchain each `rust-toolchain.toml` names (1.96.1;
nixpkgs has 1.98.1). Every commit is signed, on a local branch, and not
pushed. Evidence lives in `development-evidence/n002-*`.

| Repository | Branch, commit | Build | `--rebuild` | Gate under Nix |
| --- | --- | --- | --- | --- |
| kleis | `nix/devshell` `d8430385` | nimble build | identical | tests and format pass |
| Hagia | `nix/devshell` `155daab6` | nimble build | identical | format passes; tests pass except one: the PID-namespace SIGTERM test needs `/usr/bin/unshare` |
| narthex | `nix/devshell` `56d513fa` | nimble build | identical | tests and format pass |
| Bemenu | `nix/devshell` `75b70971` | `make bemenu-sophia`, 3.7 s | identical | `make check-sophia` passes |
| Lom | `nix/devshell` `aa238221` | crane, 277 s cold, about 5 s warm | identical | fmt, clippy, doc tests, tools pass; tests 60 pass, 4 skipped |
| Sophia | `nix/flake` `10bf14ff` | crane, 138 s cold, about 48 s warm | identical | fmt and clippy pass; workspace tests 6610 pass, 107 fail |

## Finding and resolution

**Status: the builds and deterministic rebuilds pass; test qualification is
incomplete.** The host isolated gates stay authoritative until equivalent
Nix coverage passes. Nothing is skipped to turn a result green, except Lom's
four signal tests, which are named and still run in the host gate.

**Builds:**
- Every product binary and Sophia's three session binaries build in Nix's
  sandbox and rebuild byte for byte.
- That contrasts with [n001](rm8sjc2m-desktop-product-builds-are-not-byte-reproducible-across-build-directories.md),
  where the host builder's hagia, narthex and bemenu differ between releases
  built from the same commits. It showed again in the t034 unlock hotfix:
  only Nim type-info hashes and Bemenu build paths changed.

**Gates: the host layout is the obstacle, not the build.**
1. **Hagia**'s PID-namespace SIGTERM test runs `/usr/bin/unshare`.
2. **Bemenu** reads fonts only from `/usr/share/fonts`. Its gate runs in a
   bwrap root that mounts a pinned font set there.
3. **Lom**'s four signal tests start the test binary under bwrap with the host
   `/usr` and `/etc` only, so a Nix binary cannot find `/nix/store`. They
   are skipped in Nix and stay in the host gate.
4. **Sophia's workspace tests** expect `/usr/bin/bwrap`, `/usr/bin/true`,
   `sleep`, `sh`, xterm, Go and a C compiler, and protection domains that
   bind only `/usr`. All 107 failures are listed by name with evidence in
   `n002-sophia-nix-01/RECONCILIATION.tsv`:
   - 40 protection-domain or supervisor spawns (phase B), confirmed;
   - 19 libxshmfence `dlopen` from test binaries, confirmed;
   - 23 host tools at fixed paths or on `PATH`, confirmed, plus 1 cascade;
   - 16 host-path fixtures, 3 re-executed children, 2 GBM probe statuses and
     3 PAM-helper timeout stand-ins, whose exact causes are not yet proven.

   An earlier count of 6611 passed and 110 failed also summed result lines
   that re-executed child tests print inside their captured output.

   These tests are a separate package (`workspace-tests`), not a check.

**Host dependency, by design:** the PAM helper uses the host's libpam through
`DT_RPATH` `/usr/lib`, because Nix's `pam_unix` cannot run the host's
setuid `unix_chkpwd`. The host libpam needs only libc up to `GLIBC_2.34`,
and Nix's glibc is 2.44.

**Graphics drivers are the open phase D question** (`nix-flake-proposal-01/PHASE-D-GPU-NOTES.txt`):
- Nix's libgbm names only `/run/opengl-driver/lib/gbm` as its backend path.
  That string alone does not prove GBM fails; a runtime check must.
- Nix's Vulkan loader would load host Mesa drivers whose `/usr/lib`
  dependencies a Nix process cannot resolve.
- Agreed with Codex as the phase D candidate: package-scoped pinned Mesa and
  loader paths (GBM, EGL and DRI, Vulkan ICDs, their closure and dlopen
  libraries), with no global loader environment and no new `/run` state.
  Driver loading and rendering must be shown in QEMU.

## Validation and remaining work

- **Deterministic:** builds, rebuilds and gates as in the table.
- **Not yet shown:** a Nix binary running inside a Sophia sandbox; that needs
  phase B (`/nix/store` visible, bwrap path configurable).
- **Next:**
  - phase B in Sophia, t301 (read-only `/nix/store` plus the configured
    Bubblewrap path), reviewed by Codex;
  - the graphics-driver decision;
  - the niltempus Nix release kind (phase C);
  - an FHS test root for Sophia's workspace tests.

## Connections

- [n001 reproducibility](rm8sjc2m-desktop-product-builds-are-not-byte-reproducible-across-build-directories.md):
  the Nix builds are the clean comparison.
- [n002 plan](../plans/c7g8cnd5-nix-flake-prototype-for-reproducible-desktop-builds.md).
