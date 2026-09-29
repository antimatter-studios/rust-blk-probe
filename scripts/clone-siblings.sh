#!/usr/bin/env bash
# Check out the six sibling crates this one is built against.
#
# THE PINS LIVE HERE AND NOWHERE ELSE. Every sibling is a `path =
# "../rust-*"` dependency, so what is actually compiled is decided by what is
# in those directories -- not by the version requirements in Cargo.toml, which
# cargo consults only when publishing. The ref is therefore the real pin, and
# it used to be written out twice: once in ci.yml and once in fuzz.yml. A pin
# in two places is a pin that moves in one of them (#27, and the family-wide
# sweep in rust-fs-core#168 that found three spellings across five
# repositories, two of which a single grep missed).
#
# WHY TAGS AND NOT BRANCHES. A tag cannot move under a green build, so a
# sibling's `main` advancing cannot change what this crate was tested against,
# and Cargo.lock is resolved against exactly these trees -- `cargo build
# --locked` refuses a lock resolved against a sibling's `main` whenever that
# `main` has added a dependency its tag lacks, as am-img-vmdk's has.
#
# Usage: scripts/clone-siblings.sh [destination-parent]
#   The default parent is the directory holding this checkout, which is where
#   the `path = "../rust-*"` dependencies look.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PARENT="${1:-$(dirname "$HERE")}"

# name                ref
SIBLINGS=(
    "rust-fs-core     v0.2.13"
    "rust-img-qcow2   v0.4.5"
    "rust-img-vhd     v0.4.0"
    "rust-img-vhdx    v0.4.0"
    "rust-img-vmdk    v0.3.5"
    "rust-partitions  v0.5.0"
)

# `--pins` prints the list and clones nothing, so
# tests/scripts/test-sibling-pins-agree.sh can compare it with Cargo.toml
# without a network.
if [ "${1:-}" = "--pins" ]; then
    for entry in "${SIBLINGS[@]}"; do
        # shellcheck disable=SC2086 -- splitting on whitespace is the point
        set -- $entry
        printf '%s %s\n' "$1" "$2"
    done
    exit 0
fi

for entry in "${SIBLINGS[@]}"; do
    # shellcheck disable=SC2086 -- splitting on whitespace is the point
    set -- $entry
    name="$1"
    ref="$2"
    if [ -e "$PARENT/$name" ]; then
        echo "$name: already at $PARENT/$name, left alone"
        continue
    fi
    git clone --depth 1 --branch "$ref" \
        "https://github.com/antimatter-studios/$name.git" "$PARENT/$name"
done
