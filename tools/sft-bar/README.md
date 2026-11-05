# CPU reference for Sophia Frame Transactions

A separate, default-off SFT experiment. The existing `gpu-bar/dma` executable,
including its GPU and W2 modes, is unchanged. This workspace pins the same SDK
as the real Lom SFT implementation and does not retarget the root runner.

```sh
cargo-slot niltempus cargo test --manifest-path tools/sft-bar/Cargo.toml --all-features --locked
cargo-slot niltempus cargo clippy --manifest-path tools/sft-bar/Cargo.toml --all-features --all-targets --locked -- -D warnings
```

The peer uses the existing authenticated launch fd, files transport and two-slot
owned ring. Its config is the held CPU recipe:

```json
{"wire":"files","source":"owned_ring","producer":"cpu","ring_slots":2,"output":1,"frames":2120,"height":24,"hold_msec":60000}
```

That selects r10. Adding `"frame_transactions":true` requires the
`experimental-frame-transactions` build feature and requests exactly r13/bit18.
The native owner must independently permit SFT. Unsupported modes, unknown keys
and a missing owner/build permission refuse; there is no implicit downgrade.
There is no GPU, W2 or instrumented mode in this executable.

Both paths use exactly the held `shade_mod128_a255` CPU fill, two alternating
resource IDs, one opaque placement and one outstanding frame. The legacy path
keeps image acceptance/slot release, demand/permit, candidate and explicit
predecessor retirement. SFT opens once, submits the ring pixels and complete
scene in one commit, then waits for exact presentation, source settlement and
predecessor release before starting the next frame. It closes before the final
hold; the last displayed resource stays owned until disconnect. The SDK owns
stream credit and correlation; this peer does not implement a second ledger.

## Evidence journal

Legacy remains schema 7. SFT is schema 8 and has no `permit` records. Both carry
start/ring_ready, ring_offer, ring_accepted, slot_released, offer, terminal and end.
SFT adds exactly one frame_opened, one frame_prepared per frame, frame_retired
for each predecessor, and one frame_closed. A frame_retired carries its successor
candidate generation, exact predecessor ID/version and observation time. The end
counters require zero explicit retires and N-1 predecessor releases.

The SFT `offer` timestamp is recorded after the complete commit enters the local
SDK queue and before its first I/O drive. Source status, source release and
presentation may interleave: the journal records their actual observed order.
`slot_generation` and ring admission timestamps remain null because this SDK
API does not expose them. All timestamps are peer CLOCK_MONOTONIC observations,
not host event times. No invented permit or timing row fills the removed exchange.

The device-free owner test runs the actual reference loops and Sophia's real
Session file/image/copy/candidate owners at heights 24 and 512, with both r10 and
r13. It compares every captured byte and confirms 4 versus 0 demands, all four
Presented records, N-1 releases and quiescent teardown. Authentication verdicts,
allocation policy and native completion are supplied. It does not test Factotum,
protected launch, GL/DRM, physical tear-free display or native CPU/frame.
