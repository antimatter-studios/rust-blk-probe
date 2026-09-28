#!/usr/bin/env bash
# tier.sh LABEL LOG-NAME MAX-LINES MAX-BYTES -- COMMAND [ARG...]
#
# One test tier, run QUIETLY and under a budget. The whole run goes to
# tmp/logs/<LOG-NAME>.log; a pass prints one verdict line naming that log, a
# failure prints one line naming the command's status and that log, and a run
# that passed but printed more than its budget fails with status 65 -- a
# status a reader can tell apart from a failing suite.
#
# ONE WRAPPER FOR BOTH CALLERS. chores.yml runs the tiers for a person at a
# terminal and .github/workflows/ci.yml runs them for the gate, and both run
# the SAME command through the SAME budget -- so a tier that has outgrown its
# budget says so before the push rather than in a CI log nobody was going to
# read. The two files repeat the numbers because the workflow cannot read
# chores.yml without installing chore on three runner platforms, and
# tests/scripts/test-tier-budgets-agree.sh checks that they still match: the
# duplication is deliberate and it is checked rather than trusted.
#
# WHY THE BUDGET IS PART OF THE TASK. A passing run that prints three thousand
# lines hides the twenty that matter, and every reader pays -- a person
# scrolling, a CI log viewer, and an agent working in the repository, which
# re-reads its whole transcript on each step and so pays for one loud run many
# times over. Measured across this family: 4,661M cache-read tokens against
# 9.5M of output, with command output the largest single contributor a
# repository controls.
#
# THE BUDGETS ARE IN chores.yml, next to the command each one bounds, and
# every one of them was MEASURED -- the table at the top of that file records
# the run each number came from. Raise one deliberately when a tier grows, the
# way an executed-test floor is raised; a budget nobody can breach measures
# nothing, and one lowered to fit a noisy run measures less than nothing.
#
# VERBOSE. `OUTPUT_BUDGET_VERBOSE=1`, or `--verbose`/`-v` in the chore
# invocation's CLI_ARGS (`chore test:debug -- --verbose`), streams the run as
# it happens as well as logging it. It does NOT lift the budget: the log is
# the same size either way, and a tier that has outgrown its budget should say
# so whether or not anybody was watching.
#
# THE VARIABLE IS `OUTPUT_BUDGET_VERBOSE`, NOT `FLTH_VERBOSE`. The wrapper was
# fs-linux-test-harness's before it was rust-fs-core's, and the names moved
# with it. That rename fails SILENTLY where it is not caught -- the old name
# is simply not read, nothing errors, and the run stays quiet -- so it is
# written down here, where somebody grepping for the old name lands.
#
# A FAILURE PRINTS NO TAIL BY DEFAULT, from rust-fs-core v0.2.13 on. The tail
# is rarely where the assertion is and every later step pays to re-read it;
# CI uploads each tier log as an artifact, so the detail is one download away,
# and `OUTPUT_BUDGET_FAIL_TAIL=40 chore test` restores it for one run at a
# terminal.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# WHERE THE WRAPPER COMES FROM, AND WHY THERE IS NO COPY OF IT HERE.
#
# `output-budget.sh` lives in rust-fs-core and NOWHERE ELSE. A committed copy
# is a copy that drifts: measured across this family in September 2026 there
# were several, reached four different ways, each repository internally
# consistent and nothing comparing them. This crate already depends on
# am-fs-core, so the canonical copy is always within reach; it is resolved at
# RUNTIME and never committed.
#
# SIBLING FIRST, CARGO SECOND. The sibling path is built from this script's
# own location, so it is right from any working directory and it means a
# coordinated local change to the wrapper is actually exercised rather than
# shadowed by an unpacked registry copy. Cargo is asked only when there is no
# sibling, and then it is cargo -- which has already resolved am-fs-core --
# that answers, rather than this script guessing at CARGO_HOME's layout.
#
# FS_CORE_ROOT OVERRIDES BOTH AND IS AUTHORITATIVE. When it is set that
# directory is the ONLY place looked in: an override that silently falls
# through to something else is not an override.
#
# A PRESENT-BUT-WRONG COPY IS FATAL. It is not a reason to look somewhere
# else: "core is broken" reported as "core is missing" is the quieter and more
# confusing failure, and falling through would run the tier under a script
# nobody chose. NO CHECKSUM IS PINNED -- the same digest recorded in seven
# repositories has to be chased through seven repositories every time core
# touches a comment, which is the lockstep this arrangement exists to remove.
# The contract is the string the script prints for `--version`.
SCRIPT_REL="scripts/output-budget.sh"
EXPECTED_API="rust-fs-core-output-budget 1"
MIN_CORE_VERSION="v0.2.13"
SIBLING_ROOT="$REPO/../rust-fs-core"

die() {
    echo "tier.sh: $*" >&2
    echo "         The output budget wrapper is rust-fs-core's, at $SCRIPT_REL." >&2
    echo "         Expected: bash \$core/$SCRIPT_REL --version  ->  $EXPECTED_API" >&2
    echo "         Sibling looked for: $SIBLING_ROOT/$SCRIPT_REL" >&2
    echo "         Minimum rust-fs-core release carrying it: $MIN_CORE_VERSION." >&2
    echo "         Set FS_CORE_ROOT to a checkout of it, or check one out beside" >&2
    echo "         this one -- ci.yml clones the six siblings that way." >&2
    exit 1
}

# A copy is accepted on its answer to --version and on nothing else.
insist_canonical() {
    local path="$1" found
    [ -f "$path" ] || die "$path does not exist."
    found="$(bash "$path" --version 2>/dev/null || true)"
    [ "$found" = "$EXPECTED_API" ] || \
        die "$path answered --version with '$found', not '$EXPECTED_API'."
}

resolve_budget_source() {
    if [ -n "${FS_CORE_ROOT:-}" ]; then
        SOURCE="$FS_CORE_ROOT/$SCRIPT_REL"
        # A relative FS_CORE_ROOT is also tried against the repository root, so
        # `bash scripts/tier.sh` means the same thing from any directory. It is
        # decided by looking rather than by the shape of the string.
        [ -f "$SOURCE" ] || [ ! -f "$REPO/$SOURCE" ] || SOURCE="$REPO/$SOURCE"
    elif [ -f "$SIBLING_ROOT/$SCRIPT_REL" ]; then
        SOURCE="$SIBLING_ROOT/$SCRIPT_REL"
    else
        command -v python3 >/dev/null 2>&1 || \
            die "no sibling rust-fs-core, and python3 is needed to read cargo metadata."
        # `|| true` on both halves: a cargo that refuses -- an out-of-date
        # Cargo.lock under --locked is the usual reason -- must reach the
        # message in die() rather than killing this script with cargo's own
        # status and no explanation.
        local metadata core_dir
        metadata="$(cargo metadata --format-version 1 --locked \
            --manifest-path "$REPO/Cargo.toml" 2>/dev/null || true)"
        core_dir="$(printf '%s' "$metadata" | python3 -c '
import json, sys
try:
    packages = json.load(sys.stdin)["packages"]
except Exception:
    sys.exit(0)
print(next((p["manifest_path"].rsplit("/", 1)[0]
            for p in packages if p["name"] == "am-fs-core"), ""))
' || true)"
        [ -n "$core_dir" ] || die "cargo could not say where am-fs-core is."
        SOURCE="$core_dir/$SCRIPT_REL"
    fi
    insist_canonical "$SOURCE"
}

# `tier.sh --resolve-budget` prints the wrapper this repository would use and
# does nothing else. It is how tests/scripts/test-tier-wrapper.sh gets hold of
# the REAL canonical script to run its behaviour checks against, so those
# checks cannot pass against a stand-in that merely agrees with them.
if [ "${1:-}" = "--resolve-budget" ]; then
    resolve_budget_source
    printf '%s\n' "$SOURCE"
    exit 0
fi

[ $# -ge 5 ] || { echo "tier.sh: usage: tier.sh LABEL LOG MAX-LINES MAX-BYTES -- CMD..." >&2; exit 2; }
LABEL="$1"; LOG_NAME="$2"; MAX_LINES="$3"; MAX_BYTES="$4"; shift 4
[ "${1:-}" = "--" ] && shift
[ $# -gt 0 ] || { echo "tier.sh: no command" >&2; exit 2; }

resolve_budget_source

# THE RUN GETS ITS OWN COPY, AND GIVES IT BACK. Core is a checkout somebody
# else may be moving while this runs -- `git worktree add`, a rebase, a
# coordinated edit -- and a private copy means a tier cannot have the script
# changed underneath it half way through. tmp/ is gitignored and is already
# where the tier logs live; $$ keeps two concurrent tiers apart.
mkdir -p "$REPO/tmp"
BUDGET="$REPO/tmp/output-budget.$$.sh"
cp "$SOURCE" "$BUDGET"
trap 'rm -f "$BUDGET"' EXIT

# `chore test:debug -- --verbose` arrives as CLI_ARGS. output-budget.sh reads
# OUTPUT_BUDGET_VERBOSE itself, so mapping the flag onto it is all that is
# needed -- and it means the flag and the environment variable cannot
# disagree.
case " ${CLI_ARGS:-} " in
    *" --verbose "*|*" -v "*) export OUTPUT_BUDGET_VERBOSE=1 ;;
esac

# `bash "$BUDGET"` rather than executing it: a copy's mode is not this
# script's business. It is not `exec`ed either -- that would replace this
# shell, the EXIT trap would never fire, and the copy would be left behind.
# `set -e` hands the command's own status on, which is the status this script
# must exit with.
bash "$BUDGET" \
    --log "$REPO/tmp/logs/$LOG_NAME.log" \
    --max-lines "$MAX_LINES" \
    --max-bytes "$MAX_BYTES" \
    --label "$LABEL" \
    -- "$@"
