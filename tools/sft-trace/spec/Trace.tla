------------------------------ MODULE Trace ------------------------------
EXTENDS Client, Sequences, Json, IOUtils
VARIABLE l
JsonFile == IF "JSON" \in DOMAIN IOEnv THEN IOEnv.JSON ELSE "../traces/trace.ndjson"
TraceLog == ndJsonDeserialize(JsonFile)
ValidatePostState(e) == s' = e.state
TraceInit == /\ Init /\ l = 1
TraceNext == /\ l <= Len(TraceLog)
             /\ TraceLog[l].tag = "trace"
             /\ Step(TraceLog[l])
             /\ ValidatePostState(TraceLog[l])
             /\ l' = l + 1
TraceSpec == TraceInit /\ [][TraceNext]_<<s, l>> /\ WF_<<s,l>>(TraceNext)
TraceMatched == <>(l > Len(TraceLog))
=============================================================================
