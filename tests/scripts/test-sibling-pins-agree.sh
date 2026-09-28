#!/usr/bin/env bash
# The sibling pins are written once, and everything that needs them reads
# that one place.
#
# WHAT IS ACTUALLY PINNED HERE. Every sibling is a `path = "../rust-*"`
# dependency, so what gets compiled is whatever is in those directories. The
# version requirement in Cargo.toml is consulted when publishing and nowhere
# else; the REF each sibling is checked out at is the real pin, and it lives
# in scripts/clone-siblings.sh.
#
# So there are two lists that have to name the same six crates, and one rule
# about who may write a ref:
#
#   1. every `path = "../rust-*"` dependency has a pin, and every pin has a
#      dependency -- a pinned crate nobody depends on is a clone that costs a
#      minute of every CI run, and a dependency with no pin is one whose
#      version is whatever the runner happened to have;
#   2. every requirement names a RELEASE (x.y.z), not a floor. `version =
#      "0.2"` is satisfied by 0.2.0 through 0.2.13, so the lock alone decides
#      what is built -- and it sat seven patches behind while nothing
#      complained (#27);
#   3. NO WORKFLOW CLONES A SIBLING ITSELF. ci.yml and fuzz.yml both used to
#      carry the whole list; a pin in two places is a pin that moves in one of
#      them, and fuzz.yml is exactly the file where that goes unnoticed
#      because it never reports on a pull request.
#
#   bash tests/scripts/test-sibling-pins-agree.sh
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SELF="$(basename "${BASH_SOURCE[0]}")"
fails=0
ok()   { printf 'ok    %s\n' "$1"; }
fail() { printf 'FAIL  %s\n' "$1" >&2; fails=$(( fails + 1 )); }

# --- the two lists ---------------------------------------------------------
pins="$(bash "$REPO/scripts/clone-siblings.sh" --pins)"
if [ -z "$pins" ]; then
    echo "FAIL  scripts/clone-siblings.sh --pins printed nothing" >&2
    exit 1
fi

# `am-fs-core = { path = "../rust-fs-core", version = "0.2.13" }`
deps="$(grep -oE '^[a-z0-9-]+ = \{ path = "\.\./[a-z0-9-]+", version = "[^"]+" \}' \
    "$REPO/Cargo.toml")"
[ -n "$deps" ] && ok "Cargo.toml declares $(grep -c . <<<"$deps") path dependencies" \
    || fail "no `path = \"../rust-*\"` dependency found in Cargo.toml"

while read -r dep; do
    [ -n "$dep" ] || continue
    checkout="$(sed -E 's#.*path = "\.\./([^"]+)".*#\1#' <<<"$dep")"
    version="$(sed -E 's#.*version = "([^"]+)".*#\1#' <<<"$dep")"
    if grep -qE "^$checkout v" <<<"$pins"; then
        ok "$checkout is pinned in scripts/clone-siblings.sh"
    else
        fail "$checkout is a dependency with no pin in scripts/clone-siblings.sh"
    fi
    if [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        ok "$checkout is required at the release $version, not at a floor"
    else
        fail "$checkout is required at \"$version\", which is a floor rather than a release: \
everything from $version.0 upwards satisfies it, so Cargo.lock alone decides what is built"
    fi
done <<<"$deps"

while read -r name _ref; do
    [ -n "$name" ] || continue
    grep -q "path = \"\.\./$name\"" "$REPO/Cargo.toml" \
        && ok "$name is pinned and depended on" \
        || fail "$name is pinned in scripts/clone-siblings.sh and nothing depends on it"
done <<<"$pins"

# --- nobody else writes a ref ---------------------------------------------
strays="$(grep -rnE 'git clone.*antimatter-studios' "$REPO/.github/workflows/" || true)"
if [ -n "$strays" ]; then
    fail "a workflow clones a sibling itself instead of calling scripts/clone-siblings.sh:"$'\n'"$strays"
else
    ok "no workflow clones a sibling itself"
fi
callers="$(grep -rlE 'clone-siblings\.sh' "$REPO/.github/workflows/" | wc -l)"
[ "$callers" -ge 2 ] \
    && ok "$callers workflows check the siblings out through the one script" \
    || fail "only $callers workflow calls scripts/clone-siblings.sh; ci.yml and fuzz.yml both build against the siblings"

# --- the guard can fail ----------------------------------------------------
#
# Without this, a grep that stopped matching -- a reformatted manifest, a
# changed quoting style -- would pass the real tree having checked nothing.
mkdir -p "$REPO/tmp"
SANDBOX="$(mktemp -d "$REPO/tmp/sibling-pins.XXXXXX")"
trap 'rm -rf "$SANDBOX"' EXIT HUP INT TERM
mkdir -p "$SANDBOX/scripts" "$SANDBOX/tests/scripts" "$SANDBOX/.github/workflows"
cp "$REPO/tests/scripts/$SELF" "$SANDBOX/tests/scripts/$SELF"
cat > "$SANDBOX/scripts/clone-siblings.sh" <<'FAKE'
#!/usr/bin/env bash
[ "${1:-}" = "--pins" ] && { printf 'rust-fs-core v0.2.13\nrust-orphan v1.0.0\n'; exit 0; }
FAKE
cat > "$SANDBOX/Cargo.toml" <<'FAKE'
[dependencies]
am-fs-core = { path = "../rust-fs-core", version = "0.2" }
am-partitions = { path = "../rust-partitions", version = "0.4.1" }
FAKE
cat > "$SANDBOX/.github/workflows/ci.yml" <<'FAKE'
      - run: git clone --depth 1 --branch v0.2.6 https://github.com/antimatter-studios/rust-fs-core.git ../rust-fs-core
FAKE
seen="$(bash "$SANDBOX/tests/scripts/$SELF" 2>&1)"
for want in \
    "rust-partitions is a dependency with no pin" \
    "which is a floor rather than a release" \
    "rust-orphan is pinned in scripts/clone-siblings.sh and nothing depends on it" \
    "a workflow clones a sibling itself" \
    "workflow calls scripts/clone-siblings.sh"
do
    grep -qF "$want" <<<"$seen" \
        && ok "the guard reports: $want" \
        || fail "the guard missed '$want'; it said:"$'\n'"$seen"
done

if [ "$fails" -gt 0 ]; then
    echo "FAIL  $fails check(s) failed in $SELF" >&2
    exit 1
fi
echo "PASS  the sibling pins are written once and read everywhere"
