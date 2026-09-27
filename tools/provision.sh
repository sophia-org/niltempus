#!/bin/sh
# Provision the pinned Sophia crates and every crates.io dependency into a
# private CARGO_HOME (.provision/cargo-home) so that all later gates build with
#   CARGO_HOME="$PWD/.provision/cargo-home" cargo <command> --offline --locked
#
# usage: sh tools/provision.sh [--source ABSOLUTE-SOPHIA-REPO]
#                              [--generate-lockfile | --update-lockfile]
#
# Default route: fetch the pinned revision from its canonical public URL
# (pins/sophia.toml). Optional route: --source redirects that URL to an
# explicit local clone for the cargo and git children of this script only,
# through GIT_CONFIG_COUNT/KEY/VALUE in their environment. No Git or Cargo
# configuration file is written anywhere.
#
# Why a private CARGO_HOME and not `cargo vendor`: sophia-conformance (the
# public record reader) builds only from an exact git checkout; its
# conformance-host bins use ../ paths that the vendored layout breaks. Cargo's
# git cache keeps the whole checkout at the pinned revision.
#
# --generate-lockfile creates Cargo.lock when it does not exist yet (first
# provisioning only). --update-lockfile resolves what a manifest or pin change
# requires (`cargo fetch` without --locked); the change must then be reviewed
# and committed. Otherwise Cargo.lock is required and used --locked.
#
# Seeding: a new private CARGO_HOME gets a COPY (cp -a --reflink=auto) of the
# operator's registry index, cache and src from $SOPHIA_PROVISION_SEED_REGISTRY
# (default ~/.cargo/registry). Nothing else is copied (no credentials, no
# config) and nothing links back to a shared writable cache.
#
# Network accounting: online mode is recorded separately from actual network
# use. Fingerprints of the registry index, the downloaded crate archives and
# the git database are taken before and after fetching; the log states
# whether the index changed and lists every crate that was downloaded
# (.provision/provision.log). Every gate afterwards runs --offline --locked.
#
# The result is accepted only after `cargo xtask check-pins` passes offline
# against it (and `audit-pins` when a local clone was named): only then is
# .provision/accepted written, binding url, rev and the Cargo.lock sha256.
# Gates re-run check-pins and check-provision themselves; a marker for another
# pin or lock file fails them.
set -eu

usage() {
    echo "usage: sh tools/provision.sh [--source ABSOLUTE-SOPHIA-REPO] [--generate-lockfile | --update-lockfile]" >&2
    exit 2
}

source_repo=
generate=
while [ $# -gt 0 ]; do
    case $1 in
    --source)
        [ $# -ge 2 ] || usage
        source_repo=$2
        shift 2
        ;;
    --generate-lockfile)
        [ -z "$generate" ] || usage
        generate=generate
        shift
        ;;
    --update-lockfile)
        [ -z "$generate" ] || usage
        generate=update
        shift
        ;;
    *) usage ;;
    esac
done

repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
cd "$repo"

pin() {
    value=$(sed -n "s/^$1 = \"\\([^\"]*\\)\"\$/\\1/p" pins/sophia.toml)
    [ -n "$value" ] || {
        echo "provision: pins/sophia.toml has no $1" >&2
        exit 1
    }
    printf '%s\n' "$value"
}
url=$(pin url)
rev=$(pin rev)
cargo_home="$repo/.provision/cargo-home"

: "${CARGO_BUILD_JOBS:=2}"
export CARGO_BUILD_JOBS

if [ -n "$source_repo" ]; then
    case $source_repo in
    /*) ;;
    *)
        echo "provision: --source must be absolute: $source_repo" >&2
        exit 2
        ;;
    esac
    resolved=$(git -C "$source_repo" rev-parse --verify --end-of-options "$rev^{commit}")
    [ "$resolved" = "$rev" ] || {
        echo "provision: $source_repo does not hold $rev" >&2
        exit 1
    }
    run_cargo() {
        env CARGO_HOME="$cargo_home" CARGO_NET_GIT_FETCH_WITH_CLI=true \
            GIT_CONFIG_COUNT=2 \
            GIT_CONFIG_KEY_0="url.file://$source_repo.insteadOf" \
            GIT_CONFIG_VALUE_0="$url" \
            GIT_CONFIG_KEY_1=uploadpack.allowReachableSHA1InWant \
            GIT_CONFIG_VALUE_1=true \
            nice -n 19 cargo "$@"
    }
else
    run_cargo() {
        env CARGO_HOME="$cargo_home" CARGO_NET_GIT_FETCH_WITH_CLI=true nice -n 19 cargo "$@"
    }
fi

rm -f .provision/accepted
log="$repo/.provision/provision.log"

if [ ! -d "$cargo_home/registry" ]; then
    seed=${SOPHIA_PROVISION_SEED_REGISTRY:-$HOME/.cargo/registry}
    mkdir -p "$cargo_home/registry"
    for part in index cache src; do
        if [ -d "$seed/$part" ]; then
            cp -a --reflink=auto -- "$seed/$part" "$cargo_home/registry/$part"
        fi
    done
    # Refuse a seed that smuggled a link back to the shared cache.
    if [ -n "$(find "$cargo_home/registry" -maxdepth 3 -type l -print -quit)" ]; then
        echo "provision: seeded registry contains symbolic links; refusing" >&2
        exit 1
    fi
    echo "provision seed=$seed" >"$log"
else
    echo "provision seed=reused" >"$log"
fi
mkdir -p "$cargo_home"

fingerprint() { # DIR
    if [ -d "$1" ]; then
        (cd "$1" && find . -type f -printf '%P %s %T@\n' | LC_ALL=C sort | sha256sum | cut -d' ' -f1)
    else
        echo none
    fi
}
crates() {
    find "$cargo_home/registry/cache" -type f -name '*.crate' -printf '%P\n' 2>/dev/null | LC_ALL=C sort
}
index_before=$(fingerprint "$cargo_home/registry/index")
git_before=$(fingerprint "$cargo_home/git/db")
crates >"$repo/.provision/crates.before"

if [ "$generate" = update ]; then
    [ -e Cargo.lock ] || {
        echo "provision: --update-lockfile needs an existing Cargo.lock" >&2
        exit 1
    }
    run_cargo fetch
elif [ "$generate" = generate ]; then
    if [ -e Cargo.lock ]; then
        echo "provision: Cargo.lock exists; refusing to regenerate it" >&2
        exit 1
    fi
    run_cargo generate-lockfile
elif [ ! -e Cargo.lock ]; then
    echo "provision: Cargo.lock is missing; pass --generate-lockfile once" >&2
    exit 1
fi

run_cargo fetch --locked

index_after=$(fingerprint "$cargo_home/registry/index")
git_after=$(fingerprint "$cargo_home/git/db")
crates >"$repo/.provision/crates.after"
downloaded=$(LC_ALL=C comm -13 "$repo/.provision/crates.before" "$repo/.provision/crates.after")
{
    echo "provision mode=online source=${source_repo:-canonical}"
    [ "$index_before" = "$index_after" ] && echo "registry_index changed=false" \
        || echo "registry_index changed=true"
    [ "$git_before" = "$git_after" ] && echo "git_db changed=false" || echo "git_db changed=true"
    echo "crates_downloaded count=$(printf '%s' "$downloaded" | grep -c . || true)"
    [ -z "$downloaded" ] || printf '%s\n' "$downloaded" | sed 's/^/crate_downloaded /'
} >>"$log"
cat "$log"

# Accept the provisioned home only when every pin agrees, offline.
check() {
    env CARGO_HOME="$cargo_home" nice -n 19 cargo run --offline --locked --package xtask -- "$@"
}
check check-pins
if [ -n "$source_repo" ]; then
    check audit-pins "$source_repo"
fi
lock_sha256=$(sha256sum Cargo.lock | cut -d' ' -f1)
printf 'url=%s\nrev=%s\ncargo_lock_sha256=%s\n' "$url" "$rev" "$lock_sha256" >.provision/accepted
echo "provision status=pass sophia=$rev cargo_home=.provision/cargo-home"
