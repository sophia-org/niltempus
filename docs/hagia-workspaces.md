<!-- Provenance: moved from Sophia docs/configuration.md, section "Globally numbered, monitor-owned workspaces" (under Application catalogs), at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13), as Hagia semantics. Sophia keeps the generic desktop-profile grammar (the `policy`, `output` and `shortcut` blocks and `policy:` actions it passes to the policy client). The final paragraph about the panel gate's Hagia build is rewritten for this repository's prepared-inputs custody. -->
# Hagia workspaces

## Globally numbered, monitor-owned workspaces

The WM profile owns both assignments and shortcuts. For independent DP-1 and
DP-2 workspace sets, add these settings to the selected desktop/WM profile,
retaining its existing named-output mode, position and scale settings:

```kdl
policy {
    workspace 1 output-key=1
    workspace 2 output-key=1
    workspace 3 output-key=1
    workspace 4 output-key=2
    workspace 5 output-key=2
    workspace 6 output-key=2
}
output {
    inherit-sophia #true
    named "DP-1" { policy-key 1; }
    named "DP-2" { policy-key 2; }
}
shortcut {
    bind "Super+1" "policy:focus-workspace 1"
    bind "Super+2" "policy:focus-workspace 2"
    bind "Super+3" "policy:focus-workspace 3"
    bind "Super+4" "policy:focus-workspace 4"
    bind "Super+5" "policy:focus-workspace 5"
    bind "Super+6" "policy:focus-workspace 6"
}
```

Numbers are unique across the session. Selection follows the workspace's live
host; after unplug, the existing fallback migration keeps those numbers and
preferred ownership. Reconnection uses the configured key, not enumeration
order. A cold-start absent output has no fabricated live view. Profiles without
`workspace` assignments retain legacy per-output slots. Assignment changes on
an established Hagia model require an explicit migration; arbitrary live
renumbering is refused. Initial migration of an unambiguous legacy ordinal
profile preserves view/window identity, layout and focus. Ambiguous tag or
dynamic-workspace collisions refuse the candidate without changing committed
state. Assigned dynamic workspaces use unused action-addressable numbers up to
9 and refuse exhaustion.

## In the physical gates

The Hagia gates take Hagia from prepared physical inputs, and the Lom panel
gate from a prepared product artifact: in both, the exact signed Hagia tree,
built with its reviewed Nim dependency manifest, with its commit and binary
digest recorded. No gate trusts a
prebuilt binary at a filesystem path, and none reads the user's own default
WM. All profile bindings remain WM configuration; the gates add none.
