# Lom readiness update

Release `niltempus-faa200717adf38b055fe` is verified and selected for
`niltempus install`. Only Lom's source selection changed from the preceding
release. No install, activation or live restart was performed during preparation.

Lom `53d3a921d929c92213feeff1564f436d65759d74` uses Rust SDK
`ea9cf651d37649fc1be889b3cfef6c1729b055f8`. Its service loop waits for socket
readiness and continues immediately when local protocol work remains, replacing
the fixed sleeps that accumulated across serial 9P operations.

The scripted fixture's six small-panel presentations took 697 ms with fixed
sleeps and 30 ms with readiness waits. This covers indicator updates, actions and
retained resources; it is not a measurement of live display latency or GPU work.
The SDK passed 264 tests, focused default-feature tests and clippy; Lom passed
59 Rust and 17 tooling tests, formatting, audits and clippy. Restoring the sleep
caused the readiness regression test to fail.

## Packaged identities

| Binary | SHA-256 |
| --- | --- |
| Sophia | `5d58c93dd68c485ccf32c9efb0239373d88f3d46b5f472102aa62061c969fb44` |
| Hagia | `7314ee9c3b49285bc117fcd5e316cc1b31e7c00dba4019895584ecbfde6cb6c7` |
| Lom | `d5c63c314eaa755c3d9eae1931c8d71d17fa00b44848eda175ec7c03cd638be6` |
| Bemenu | `aa50abda4d56f79f6e9d40011d56996301160dc9d00195e8d721e90e205c153a` |

The full Go build, schema-7 verification and profile preflight passed. The exact
packaged Hagia passed all 24 protected pairing phases; packaged Bemenu passed
the 9P launcher fixture; packaged Lom passed the protected CPU content-path
fixture. Install, repeat install, second install, rollback and status passed
with the real CLI in private mounts.

The release is copied to durable installer state. Its selected manifest SHA-256
is `37fbffd6d5ab1893dc707b26a0ba9b9dcf7097efa23df8b776fc5e6877a6e668`.
The previous selection is backed up. Preparation preserved the installed current
link, personal Hagia, WM metadata, user profile and installer executable.
The existing personal Hagia remains `24cf71ac...`, qualified with the prior
release; installation preserves it.

Evidence is under `lom-indicator-latency/` and
`hagia-sdk-pairing/run6-lom-readiness-release/` in development evidence.
`approved-preparation.json` records the selection and preserved identities.

## Remaining update limitation

The running Session launches Lom from the sealed desktop release path. It has
no supported command to select a different component executable in place.
This release therefore takes effect after install and a new login. Immutable
artifacts need not imply a full-desktop rebuild: an independent component
selection and restart workflow remains to be implemented.
