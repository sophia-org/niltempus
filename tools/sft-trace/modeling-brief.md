# Client lifecycle trace scope and coverage

Category A: a single caller owns FrameStream and applies ordered messages. No
shared-memory linearization or kernel scheduling is being modeled. The exact
implementation is SDK b483e1e frame_stream.rs; the model is a projection of it,
not a new normative wire specification. Sophia's design/host models remain the
source for admission, custody and resource invariants.

| Scenario | Real test / trace | Model gate |
| --- | --- | --- |
| Credit only after exact terminal, never custody/Prepared | one_credit_returns_only_on_exact_terminal_once | CreditSafe, Prepared, Terminal; TraceMatched |
| Close and terminal in either order, then reopen | closing_drains_in_both_orders_before_reopen | DrainSafe, Closed, Terminal; TraceMatched |
| Automatic expiry racing explicit close | automatic_close_waits_for_the_racing_close_refusal | RefuseClose, DrainSafe; TraceMatched |
| Refused admission preserves high water | refusal_releases_local_credit_but_not_generation | RefuseCommit, Submit; TraceMatched |
| Queue saturation / wrong binding changes no state | existing queue/wrong_submission tests | Real assertions; no transition emitted |
| Protocol fault keeps outstanding work, cannot reopen | wrong_completion_tuple / disconnect tests | Fault, full post-state equality; TraceMatched |

MC.cfg enables CreditSafe, DrainSafe and PendingSafe for two stream ids and two
candidate generations; 143 reachable states. Trace.cfg enables the same safety
invariants and the mandatory TraceMatched completion property. There are no
silent actions and no synthesized post-states. Every captured state field is
checked by equality with the independently computed model state.

The real typed tests supply transport replies, and actual SDK queue/custody/
journal/FrameStream code processes them. They do not prove the host generated
those replies. Full tuple/codec refusal is deliberately abstracted to Fault;
identity correctness remains covered by the existing typed tests, not this
model projection. Successful client admissions are traced; failed local calls
are checked by existing assertions only. Physical source slots and resource
leases are separate owners and are not represented by client frame credit.

Mutation checks change real SDK code only in the disposable worktree: grant
credit on Prepared; drain without waiting for pending work; drop pending work
on disconnect. Runtime traces for all three must fail TraceMatched. These are
negative controls, not bugs found in the unmodified SDK. The disconnect mutation
passes the existing tests but fails trace validation, demonstrating added
coverage for preservation of outstanding work.
