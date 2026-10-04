<!-- Provenance: moved from Sophia docs/shell-reference-sheets.md, sections "Narthex behavior and configuration" and the product parts of "Reference and acceptance" (the Hagia binding, the Narthex private configuration and UI, the installed pairing and the live acceptance), at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13). Sophia keeps the generic read-only reference-sheet contract (ownership, wire and lifetime, and the Engine-side regression coverage). -->
# Narthex reference sheets

Narthex presents Sophia's read-only shell reference sheet (the keyboard
shortcut overview). Sophia owns the generic contract: which bindings are
published, the wire, the sheet's lifetime, capture and withdrawal. This page
is the product side: Hagia's binding, Narthex's own configuration and UI, and
the operator acceptance of the pair.

## Hagia binding

Hagia's compiled default and the user's desktop binding use:

```kdl
shortcut {
    profile "daily"
    bind "Super+?" "session:shortcut-help" label="Show keyboard shortcuts" group="Session"
}
```

`Super+?`, `Super+Question`, `Super+Shift+/` and `Super+Shift+slash` normalize
to the same physical chord; defining two of them is a duplicate. Optional
`label` and `group` properties also work on `pointer-bind`; existing
two-argument bindings are unchanged.

## Narthex configuration

Narthex's private file is `$XDG_CONFIG_HOME/narthex/config.kdl`, falling back to
`~/.config/narthex/config.kdl`. An explicit `SOPHIA_SHELL_CONFIG` selects
another file. The session mounts only that selected file read-only and passes
its path; it never parses shell vocabulary. No file means startup help is
enabled.

```kdl
hotkey-overlay {
    skip-at-startup #false
}
```

Set the boolean to `#true` to disable the once-per-login display.

## Behavior

Narthex groups all configured bindings by purpose, preserves order within a
group, and uses optional labels or readable public-action names. There are no
group headings, search field, toolbar or pagination footer. Page Up/Down and
wheel input change pages; the next ordinary key dismisses and is consumed.
Modifier transitions alone do not dismiss. Pointer input cannot activate
windows through the sheet. The emergency chord and VT routing keep
precedence; the window-switcher chord replaces help. Geometry stays fixed
across pages of the same catalog, style and output.

Visual policy follows Triad `fb8fb27` (`src/daemon/hotkey_overlay_render.nim`):
24px padding, 10px row gap, 28px key/label gap, 32px column gap, 48px screen
margin, 14px body, 16px title, 4px square border and its six ARGB colors.
JetBrains Mono replaces Triad's host-selected sans font. Triad is a
development comparison source, never a runtime or build dependency.

## Acceptance

Live acceptance is one normal installed session: startup once, toggle, page,
dismiss, open the switcher, and verify unchanged terminal focus and camera and
emergency recovery. It does not reopen the same-hardware comparison matrix.
Sophia and Narthex are installed together as one release (`nix build
.#desktop`; see [install](install.md)); rebuild Hagia for the compiled binding
and parser support. Reloading only Hagia cannot install Engine or protocol
changes.

