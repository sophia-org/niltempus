----------------------------- MODULE Client -----------------------------
EXTENDS Naturals
VARIABLE s
vars == <<s>>

\* Projected client state: frame_stream.rs:32-41 at SDK b483e1e.
Initial == [phase |-> "Closed", key |-> 0, opening |-> 0, closing |-> 0,
            last_stream |-> 0, last_candidate |-> 0, credit |-> FALSE,
            closed_notice |-> FALSE, pending_tx |-> 0, pending_gen |-> 0,
            prepared |-> FALSE]
Init == s = Initial
\* frame_stream.rs:333-337: physical source/resource lifetimes are separate.
Drain(q) == IF q.closed_notice /\ q.pending_gen = 0 /\ q.closing = 0
            THEN [q EXCEPT !.phase = "Closed"] ELSE q
ClearPending(q) == [q EXCEPT !.pending_tx = 0, !.pending_gen = 0,
                             !.prepared = FALSE]

\* Successful queue admission only: frame_stream.rs:71-97, 151-164, 172-195.
Open(tx, stream) ==
    /\ s.phase = "Closed" /\ stream > s.last_stream
    /\ s' = [s EXCEPT !.phase = "Opening", !.key = stream,
             !.opening = tx, !.closing = 0, !.closed_notice = FALSE,
             !.credit = FALSE]
Submit(tx, gen) ==
    /\ s.phase = "Open" /\ s.credit /\ s.pending_gen = 0
    /\ gen > s.last_candidate
    /\ s' = [s EXCEPT !.last_candidate = gen, !.credit = FALSE,
             !.pending_tx = tx, !.pending_gen = gen, !.prepared = FALSE]
Close(tx) ==
    /\ s.phase \in {"Open", "Closing"} /\ s.closing = 0
    /\ ~s.closed_notice /\ tx # s.opening
    /\ s' = [s EXCEPT !.phase = "Closing", !.credit = FALSE, !.closing = tx]

\* Validated journal observations: frame_stream.rs:226-331.
Opened(tx, stream) ==
    /\ s.phase = "Opening" /\ tx = s.opening /\ stream = s.key
    /\ s' = [s EXCEPT !.phase = "Open", !.credit = TRUE,
             !.last_stream = stream]
Closed(tx, stream) ==
    /\ s.phase \in {"Open", "Closing"} /\ ~s.closed_notice
    /\ stream = s.key /\ tx \in {s.opening, s.closing} /\ tx # 0
    /\ s' = Drain([s EXCEPT !.closed_notice = TRUE, !.credit = FALSE,
                    !.closing = IF tx = s.closing THEN 0 ELSE @,
                    !.phase = "Closing"])
RefuseOpen(tx, stream) ==
    /\ s.phase = "Opening" /\ tx = s.opening /\ stream = s.key
    /\ s' = [s EXCEPT !.phase = "Closed"]
RefuseClose(tx, stream) ==
    /\ s.phase = "Closing" /\ tx = s.closing /\ tx # 0 /\ stream = s.key
    /\ s' = Drain([s EXCEPT !.closing = 0])
RefuseCommit(tx, gen, stream) ==
    /\ s.phase # "Unknown" /\ s.pending_gen # 0 /\ ~s.prepared
    /\ tx = s.pending_tx /\ gen = s.pending_gen /\ stream = s.key
    /\ s' = Drain(ClearPending([s EXCEPT !.credit = s.phase = "Open"]))
Prepared(tx, gen, stream) ==
    /\ s.phase # "Unknown" /\ s.pending_gen # 0 /\ ~s.prepared
    /\ tx = s.pending_tx /\ gen = s.pending_gen /\ stream = s.key
    /\ s' = [s EXCEPT !.prepared = TRUE]
Terminal(tx, gen, stream, returned) ==
    /\ s.phase # "Unknown" /\ s.pending_gen # 0
    /\ tx = s.pending_tx /\ gen = s.pending_gen /\ stream = s.key
    /\ ~(s.closed_notice /\ returned)
    /\ LET credit == s.phase = "Open" /\ returned
       IN s' = Drain(ClearPending([s EXCEPT !.credit = credit,
                      !.phase = IF credit THEN @ ELSE "Closing"]))
\* frame_stream.rs:198-221: codec/tuple failures abstracted as Fault.
Fault == s' = [s EXCEPT !.phase = "Unknown", !.credit = FALSE]

Step(e) == CASE e.action = "Open" -> Open(e.tx, e.stream)
             [] e.action = "Submit" -> Submit(e.tx, e.generation)
             [] e.action = "Close" -> Close(e.tx)
             [] e.action = "Opened" -> Opened(e.tx, e.stream)
             [] e.action = "Closed" -> Closed(e.tx, e.stream)
             [] e.action = "RefuseOpen" -> RefuseOpen(e.tx, e.stream)
             [] e.action = "RefuseClose" -> RefuseClose(e.tx, e.stream)
             [] e.action = "RefuseCommit" -> RefuseCommit(e.tx, e.generation, e.stream)
             [] e.action = "Prepared" -> Prepared(e.tx, e.generation, e.stream)
             [] e.action = "Terminal" -> Terminal(e.tx, e.generation, e.stream, e.returned)
             [] e.action = "Fault" -> Fault
             [] OTHER -> FALSE
CreditSafe == s.credit => (s.phase = "Open" /\ s.pending_gen = 0)
DrainSafe == s.phase = "Closed" => (s.pending_gen = 0 /\ s.closing = 0 /\ ~s.credit)
PendingSafe == (s.pending_gen = 0) = (s.pending_tx = 0)
               /\ (s.prepared => s.pending_gen # 0)
=============================================================================
