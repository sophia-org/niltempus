------------------------------- MODULE MC -------------------------------
EXTENDS Client
Next == \/ \E stream \in 1..2: Open(10, stream)
        \/ \E gen \in 1..2: Submit(20 + gen, gen)
        \/ Close(30)
        \/ Opened(10, s.key)
        \/ Closed(10, s.key) \/ Closed(30, s.key)
        \/ RefuseOpen(10, s.key) \/ RefuseClose(30, s.key)
        \/ RefuseCommit(s.pending_tx, s.pending_gen, s.key)
        \/ Prepared(s.pending_tx, s.pending_gen, s.key)
        \/ \E returned \in BOOLEAN: Terminal(s.pending_tx, s.pending_gen, s.key, returned)
        \/ Fault
Spec == Init /\ [][Next]_vars
=============================================================================
