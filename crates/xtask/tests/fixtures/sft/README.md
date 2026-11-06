These journals come from the real SFT reference peer / Session owner test at
heights 24 and 512, with r10 and r13. PROVENANCE.json pins the generating log.
They are device-free: authentication, allocation and display completion are
supplied by the fixture. The peer fill, SDK, owned-ring copy worker, candidate
stores, source release and predecessor retirement are real. These are not native
timing samples or a cadence/performance result.

Schema 8 requires Opened, every commit's source/Prepared/Presented/predecessor
obligations, Closed and exact end counters. Source release and terminal delivery
can interleave. No synthetic Permit rows or rewritten timestamps are present.
Historical schema 7 fixtures remain separate and must retain their permit rows.

## Terminal-only captures

`schema8-terminal-h24.ndjson` and `schema8-terminal-h512.ndjson` are unchanged
JSON lines from the terminal-only cases of the reference peer's real owner/copy
worker test. `terminal-provenance.json` pins the source log and source commits.
The peer still emits schema 8; only the nonterminal Prepared notice is absent.
These fixtures are device-free and do not record native presentation.

The native request must explicitly set `frame_terminal_only: true` alongside
`frame_transactions: true`, matching the peer's `terminal_only: true`. Only that
request selects `collect_terminal_frame_metrics`. The ordinary collector still
requires all Prepared records. The opted-in collector rejects Prepared and keeps
all other acceptance, release, terminal, retirement and close requirements.
Its output marks `frame.terminal_only=true`; false is omitted for old captures.

A native comparison must also bind the Session startup receipt
`sophia_native_frame_negotiated` to its grant and expected capability mask. The
runner stages that receipt in native.log but does not yet consume it itself;
the package postflight must check it before any performance claim. Do not infer
negotiation solely from an absent record. Gates and timing thresholds are unchanged.
