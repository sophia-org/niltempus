# Self-test fixtures for Sophia identity (the G1 design): the identity is
# Sophia's own signed pinned commit, taken from a temporary
# `git clone --shared --no-checkout` of the explicit SOPHIA_TEST_SOURCE. No
# sibling checkout (../hagia, ../narthex) and no checkout layout is read; where
# a fixture needs a signed "policy" repository it uses the same clone.
#
# test_sophia_setup WORK_DIR ROOT_DIR
#   exports SOPHIA_SOURCE (the clone), SOPHIA_TEST_COMMIT (the pinned
#   revision from ROOT_DIR/pins/sophia.toml) and, when SOPHIA_TEST_TREE is
#   set, SOPHIA_ROOT (the staged pinned tree for Sophia's retained files).
test_sophia_setup() {
    local work="$1" root="$2" source="${SOPHIA_TEST_SOURCE:-}" rev
    [[ "$source" == /* ]] && git -C "$source" rev-parse --git-dir >/dev/null 2>&1 || {
        echo "SOPHIA_TEST_SOURCE must name an absolute Sophia repository holding the pin." >&2
        return 2
    }
    rev="$(sed -n 's/^rev = "\([0-9a-f]\{40\}\)"$/\1/p' "$root/pins/sophia.toml")"
    [[ "$rev" =~ ^[0-9a-f]{40}$ ]] || { echo "pins/sophia.toml has no revision" >&2; return 2; }
    git -c init.templateDir= clone --quiet --shared --no-checkout "$source" "$work/sophia-source"
    git -C "$work/sophia-source" cat-file -e "$rev^{commit}" || {
        echo "SOPHIA_TEST_SOURCE does not hold the pinned revision $rev" >&2
        return 2
    }
    export SOPHIA_SOURCE="$work/sophia-source"
    export SOPHIA_TEST_COMMIT="$rev"
    if [[ -n "${SOPHIA_TEST_TREE:-}" ]]; then
        [[ "$SOPHIA_TEST_TREE" == /* && -d "$SOPHIA_TEST_TREE/tools" ]] || {
            echo "SOPHIA_TEST_TREE must name the absolute staged pinned tree." >&2
            return 2
        }
        export SOPHIA_ROOT="$SOPHIA_TEST_TREE"
    fi
}

# test_sophia_require_tree: the staged pinned tree is required by this check.
test_sophia_require_tree() {
    [[ -n "${SOPHIA_ROOT:-}" ]] || {
        echo "SOPHIA_TEST_TREE must name the absolute staged pinned tree." >&2
        return 2
    }
}
