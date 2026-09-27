# Explicit Sophia inputs for the moved physical runners, archives and
# verifiers. Sophia is not this repository, so nothing here derives Sophia's
# identity or files from this checkout or a sibling path.
#
# SOPHIA_SOURCE  an absolute Sophia git repository (a checkout, or the
#                temporary `git clone --shared --no-checkout` the self-tests
#                make) that holds the signed Sophia commits evidence names.
# SOPHIA_ROOT    an absolute Sophia tree at the pinned revision (the staged
#                `git archive` the gates make) for the generic files that stay
#                in Sophia and are read, never copied: the probe profiles,
#                core.kdl, atomic-scanout preflight, the DRM-master guard.

# Print the Sophia repository, or fail (status 2) when it is not explicit.
sophia_source_repo() {
    local repo="${SOPHIA_SOURCE:-}"
    if [[ "$repo" != /* ]] || ! git -C "$repo" rev-parse --git-dir >/dev/null 2>&1; then
        echo "SOPHIA_SOURCE must name an absolute Sophia git repository (no default)." >&2
        return 2
    fi
    printf '%s\n' "$repo"
}

# Print the pinned Sophia tree, or fail (status 2) when it is not explicit.
sophia_pinned_root() {
    local root="${SOPHIA_ROOT:-}"
    if [[ "$root" != /* || ! -d "$root/tools" ]]; then
        echo "SOPHIA_ROOT must name an absolute Sophia tree at the pinned revision (no default)." >&2
        return 2
    fi
    printf '%s\n' "$root"
}
