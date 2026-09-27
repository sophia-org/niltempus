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
# The result is accepted only after `cargo xtask check-pins` passes offline
# against it (and `audit-pins` when a local clone was named): until then no
# .provision/accepted marker exists.
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
mkdir -p "$cargo_home"

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

# Accept the provisioned home only when every pin agrees, offline.
check() {
    env CARGO_HOME="$cargo_home" nice -n 19 cargo run --offline --locked --package xtask -- "$@"
}
check check-pins
if [ -n "$source_repo" ]; then
    check audit-pins "$source_repo"
fi
printf 'sophia=%s\n' "$rev" >.provision/accepted
echo "provision status=pass sophia=$rev cargo_home=.provision/cargo-home"
