# Instrumentation map

SDK source: b483e1ee9d79730705d372ffcebd47c7b12c4514,
crates/sophia-shell-client/src/frame_stream.rs. Apply only the adjacent patch to
a disposable clean worktree. No protocol or producer changes are needed.

| Model action | Real mutation / observation |
| --- | --- |
| Open | open(), after enqueue succeeds and Opening is installed |
| Submit | submitted(), common to retained and owned-ring submission |
| Close | close(), after enqueue succeeds and admissions stop |
| Opened, Closed, Refuse*, Prepared, Terminal | observe(), after apply succeeds |
| Fault | disconnect(), including observe validation failure |

trace() reads all eleven fields in the model's projected record directly from
the object. None means zero for nonzero identities; an absent pending object
also means prepared=false. The trace id is instrumentation-only and moves with
the object. The event arguments come from the submitted/received record.
Timestamps are actual UNIX nanoseconds encoded as strings, never a fake clock.

To add a projection field, update the writer, Client.Initial/actions and the
modeling brief together; Trace.ValidatePostState compares the entire record.
To collect again, reverse the exact instrumentation.patch, verify a clean pinned
checkout, and run run.sh with a new output directory. Tests and TLC have outer
timeouts. Timing binaries always use the unmodified published SDK commit.
