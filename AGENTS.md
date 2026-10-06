# Working in rust-blk-probe (agent guide)

`blk.probe` — one command-line binary that opens a disk image (raw, or a
qcow2 / VHD / VHDX / VMDK container), walks its partition table and writes JSON
describing what is inside to stdout. Every other sibling in this family hands
back a static archive and headers; this one hands back an executable that a
host application runs as a child process, by name, from a fixed path. This file
is the fast path for an agent picking up work here, so the workflow does not
have to be re-derived each time. It points at the existing docs rather than
duplicating them:

- **README** → `## The name` (why `[[bin]]` cannot be renamed alone), `## Usage`
  and the exit-code table, `## Output`, `## Dependencies`.
- **`.github-guard`** → why one required check and not three, argued at length.

The section between the BEGIN/END markers below is **shared, byte-identical,
with every repository in this family**. Do not edit it here: change the
canonical copy and propagate it, or `chore lint` will fail. Everything after
the END marker is specific to this repository.

<!-- BEGIN SHARED BLOCK: agent-core v2 sha256:38af4d2c5377d38ab382baa4eab4aa679841e2b4eba4f4d01dacd255ffa7d32e -->
## Claiming work

Several agents work these repositories at the same time. Before you start on
an issue, claim it, so nobody else spends a session on what you are already
doing. The lock is a **GitHub label**, because labels are shared state that
every agent can read and change without posting comments into the thread.

**Before starting.** Check, claim, then read back:

```sh
gh issue view <N> --json labels                      # holds `claimed`? pick another
gh issue edit <N> --add-label claimed --add-label claim/<session>
gh issue view <N> --json labels                      # read back and confirm
```

`<session>` is your session name — `agent-<random4>-<isodate>`, e.g.
`agent-3f7c-2026-09-22`. Create the `claim/<session>` label if it does not
exist.

**Resolving a race.** Adding a label is not compare-and-swap: two agents can
both add `claimed` and both believe they won. That is what the read-back is
for. If it shows more than one `claim/*` label, the **lexically lowest**
session keeps the issue; every other agent removes its own `claim/*` label and
picks different work. Each racer computes the same answer independently, so no
further coordination is needed.

**When you finish or stop.** Remove both labels — on merge, or the moment you
abandon the work:

```sh
gh issue edit <N> --remove-label claimed --remove-label claim/<session>
```

Delete your `claim/<session>` label from the repository at the end of your
session so they do not accumulate.

**Reclaiming a stale claim.** An agent that dies holding a claim would block an
issue forever. If `claimed` was applied more than 12 hours ago and the holder's
branch has no commits since, any agent may take it: remove the stale `claim/*`,
add your own, and say so in the issue.

**This is a convention, not a fence.** Nothing enforces it. An agent that
ignores it duplicates work; it cannot corrupt anything. Honour it anyway.

## Work in a worktree

Every working copy is a **git worktree** of an existing checkout, made with
`git worktree add`. Never `git clone` a second, unlinked copy — not for a
branch, a PR, a review, or a sibling you need at another ref:

```sh
git -C <checkout> fetch origin
git -C <checkout> worktree add <path> -b <type>/<name> origin/main   # new work
git -C <checkout> worktree add --detach <path> <tag>                 # a sibling at a pinned ref
git -C <checkout> worktree remove <path>                             # when done
```

A worktree shares the checkout's objects and remotes, and `git worktree list`
shows it to every agent on the machine, so nobody else mistakes it for
abandoned work or loses track of it. An unlinked clone copies all the history
again, is invisible to that list, and gets left behind in `/tmp` long after the
work that made it is merged. Remove your worktree when you finish.

## Skills to use

- **`dev-loop`** — the required loop for any non-trivial change: baseline the
  full suite → change → re-run (no baseline test may regress) → enhance tests →
  vet. Always run it.
- **`commit`** / **`pr`** — for grouping commits and opening pull requests.

Each repository names any further skills of its own below.

## A bug fix starts with a red

**Prove it is broken first** — a failing check or test — *then* fix it, *then*
prove that same check is green, *then* confirm the full baseline still passes.
Never write the fix before you have a red. A fix with no failing test to its
name is a claim, not a result.

## Nothing skips

A test that cannot run **fails**, naming the task that would provide what it
needed. Never add an early return for a missing fixture, tool or VM: a skipped
test reads exactly like a passing one, and a suite that quietly declines to run
is indistinguishable from a suite that passes.

Where a tier reports skips or ignored tests, that is a gate, not a note.

## Validate against something that is not us

A driver's own readers share its interpretation of the format, so they cannot
catch a misreading: the mistake is baked into the fixture *and* the parser, and
they agree with each other while disagreeing with every real filesystem. Unit
tests over self-built fixtures prove self-consistency, not correctness.

Every structure that is parsed or written gets a cross-validation test against
an **independent oracle** — the platform's own tools, a real kernel, or a third
implementation — before it is considered done. Each repository names its
oracles below.

## Output is budgeted

Test tiers run through `scripts/tier.sh`, which runs the suite **quietly**: the
whole run goes to `tmp/logs/<tier>.log`, a pass prints one verdict line naming
that log, and a failure prints the verdict, the command's status and the log's
path — `--tail N`, or `OUTPUT_BUDGET_FAIL_TAIL=N`, prints the tail for whoever
is watching. **Read the log**: a failing tier names it and does not recite it.
CI keeps the logs as an artifact, so the detail is always retrievable.

The budget caps the log, not merely what is shown, and every number in the
table was measured. A run that passes but prints more than its budget **fails**.

The reader who pays most for a noisy suite is an agent that re-reads its whole
transcript on every step, and so pays for one loud run many times over. If a
tier legitimately grows, raise its row **with the measurement that justifies
it**. Do not silence output to fit, and do not route around `tier.sh`.

## Commits and branches

- Branches are `<type>/<name>`, matching the commit type: `fix/`, `feat/`,
  `ci/`, `docs/`, `chore/`, `test/`.
- A commit is a subject plus flat one-sentence bullets. Subjects are
  declarative, not imperative: "the run-end bound is checked", not "check the
  run-end bound".
- **No AI attribution and no co-author trailers**, in commits or in pull
  request descriptions.
- `main` takes **squash merges only**.

## Project rules

- **No GPL/LGPL/AGPL dependencies.** Permissive only (MIT/BSD/Apache).
  Shelling out to a copyleft CLI as a *test oracle* is fine — linking or
  copying it is not.
- **Each of these is a standalone project.** Never mention a consuming
  application in the README, the source, or CLI help.
<!-- END SHARED BLOCK: agent-core v2 -->
## What this is

A command-line probe that identifies what is on a block device or image file,
built on `rust-fs-core` and `rust-disk-partitions`. It is a **binary**, not a staticlib —
`chore binary` builds it, and there is no `staticlib` task, because nothing
links this into the app.

The tool is named `blk.probe`. It was `diskprobe` until #11, then a
hyphenated name until #36, and each consumer side landed in lockstep; there
is no fallback to an old name anywhere and none should be added. Cargo
refuses a dot in a target name, so the `[[bin]]` is `blk_probe` and
`scripts/stage-binary.sh` renames it when `chore binary` stages it —
`tests/scripts/test-staged-binary-name.sh` holds both halves.

## Running tests

```sh
chore build          # debug build
chore test           # every tier: test:debug then test:release
chore test:debug     # one tier, quietly, under its budget and floor
chore test:release   # the profile that ships, which the debug tier does not cover
chore lint           # the agent-core check, the shell tests, fmt, clippy
```

CI runs both tiers on each matrix leg, plus `fmt`, and `ci-ok` aggregates them.

**Every tier is budgeted.** `scripts/tier.sh` runs it quietly: the transcript
goes to `tmp/logs/<tier>.log`, a pass prints one verdict line naming that log,
a failure prints one line naming the command's own status, and a run that
passed but printed more than its budget exits **65**. `chore test -- --verbose`
(or `OUTPUT_BUDGET_VERBOSE=1` — *not* `FLTH_VERBOSE`, which is not read)
streams the run without lifting the budget. `OUTPUT_BUDGET_FAIL_TAIL=40` brings
back the tail of a failure for whoever is watching.

The budgets and the executed-test floors are in `chores.yml`, measured, in a
table at the top of it — and repeated in `ci.yml`, because the workflow cannot
read `chores.yml` without installing `chore` on three runners.
`tests/scripts/test-tier-budgets-agree.sh` fails a pull request in which the
two disagree.

**There is no `scripts/output-budget.sh` here.** The wrapper is `rust-fs-core`'s
and is resolved at run time — the `../rust-fs-core` sibling first, then cargo's
answer for `rust-fs-core` — verified by its `--version` string and copied into
gitignored `tmp/` for the run. A copy that is present and answers something
else is **fatal**, not a reason to look elsewhere.

## Fuzzing: two tiers, one corpus

`fuzz/` is the **explorer** — `cargo-fuzz` targets on nightly, run for a
bounded time by `.github/workflows/fuzz.yml` nightly and on demand. It is not
a required check and must not become one: what a fuzzer finds depends on how
long it ran, so a fresh finding would fail whichever unrelated pull request
happened to be open.

`tests/fuzz_decoders.rs` is the **gate** — deterministic, stable toolchain, in
every pull request. It replays every file in `fuzz/corpus/` and then applies
seeded, length-preserving mutations to them, under a deadline (a hang fails
the run) and an executed-case floor of 15,000 (a suite that stopped generating
cases fails rather than passing empty). Anything the explorer finds is
committed to the corpus, which is why both tiers read the same directory.

## The oracles

`tests/oracle_tools.rs` runs **`blkid -o export`** and **`sfdisk --dump`** over
the committed corpus and compares them with this crate's document, field by
field: the table, the whole-device filesystem, and every partition's start,
length, type and name. None of those numbers came from this crate.

It is compiled on **Linux only** — they are util-linux's tools and macOS has
no equivalent — and on Linux a missing tool **fails**, naming the package. It
does not skip. The recorded half of the same check, in `tests/fuzz_decoders.rs`,
runs on every platform with neither tool installed, and this file is what keeps
those recorded expectations honest.

XFS, Btrfs and EROFS are identified here, in `src/superblock.rs`, not by
`rust-disk-partitions`, and only where its sniff answered `unknown`. Their oracle
rows write images at test time with each format's own mkfs (xfsprogs,
btrfs-progs, erofs-utils, installed by CI's Linux legs) rather than
committing them: mkfs.xfs will not go under 300 MiB.

The interesting rows are the containers: `blkid` reads a qcow2 or a VMDK as an
unrecognised blob, and this crate unwraps it and finds the same GPT `blkid`
finds in the raw original. That is the reason the crate exists, written as a
test rather than as a paragraph.

`scripts/make-fuzz-corpus.sh` rebuilds that corpus from images `sgdisk`,
`sfdisk`, `mkfs.ext4`, `mksquashfs`, `mkfs.xfs`, `mkfs.btrfs`, `mkfs.erofs`
and `qemu-img` wrote. Those images are
also the **oracle**: `every_committed_image_is_read_as_what_the_tool_that_
wrote_it_says` holds each one to the container and the table its maker put
there, on a machine with none of those tools installed. A seed that stopped
probing would otherwise go on being mutated and go on not failing.

**The corpus has already earned it.** `fuzz/corpus/probe/vhdx-head.img` — the
first 320 KiB of a real VHDX — turned a use-after-free into a SIGBUS on the
first run of the gate that replayed it. The four `*_open_on_device` functions
consume the handle they are given *before* they try to parse anything, so a
NULL return does not mean nothing happened; closing the inner handle on that
path is a double free. See `open_container_on`'s doc comment.

## The siblings, and two known traps

`scripts/clone-siblings.sh` is the **only** place a sibling ref is written.
`ci.yml` and `fuzz.yml` both call it, `Cargo.toml` names the matching release
for each, and `tests/scripts/test-sibling-pins-agree.sh` refuses a floor
requirement, a dependency with no pin, a pin with no dependency, or a workflow
that clones a sibling itself.

- **#14 / #28** — `rust-disk-partitions` adds fields to `PartitionInfo` between
  releases (0.5.0 added `slot`, `issues` and `available_length`, which had sat
  on its `main` since v0.4.1). `src/lib.rs` zero-initialises the out-parameter
  rather than naming its fields, so it builds against the pinned tag and the
  `main` alike; do not turn it back into a struct literal. `Cargo.lock` and
  `fuzz/Cargo.lock` are resolved against the pinned tags — a lock resolved
  against a sibling's `main` (rust-img-vmdk's adds `flate2`) makes
  `cargo build --locked` refuse on every CI leg.
- **#15** — an `unnecessary_cast` in the partition-label slice fires only on
  aarch64, where `c_char` is unsigned. `.cast::<u8>()` is the spelling that is
  right on every target; `as *const u8` is a no-op cast there and clippy
  refuses it, so the pre-commit hook can block a commit over a warning two of
  the three CI legs will never show you.

## What gates a merge

One required check, `ci-ok`, declared in `.github-guard` and aggregating every
job in `ci.yml`. `needs:` names the matrix **job**, not its legs, so a leg can
be added without editing branch protection.

`chore check:ci-gate` holds both halves of that mechanically — every job in
`ci.yml` must appear in `ci-ok`'s `needs:`, and `.github-guard` must require
`ci-ok` and nothing else. The task runs `scripts/core.sh ci-gate` and nothing else,
so the script is what can be tested, reviewed and run without `chore` at all.
It replaced `tests/ci_aggregate_gate.rs`: that parsed a YAML file and compared
strings, exercising nothing this crate ships, and as a `cargo test` it counted
towards the executed-test floor the gate itself enforces.

Judging mergeability from check **conclusions** is unreliable: an in-progress
`CheckRun` reports its conclusion as an empty string, and a `StatusContext` has
no conclusion field at all. Read `mergeStateStatus` and
`statusCheckRollup.state`.

## Releases

A pushed `v*.*.*` tag runs `.github/workflows/release.yml`: it checks that the tag
matches `Cargo.toml`, runs the whole of `ci.yml` as its gate, then builds
`rust-blk-probe-<version>-<platform>.tar.gz` (`bin/blk.probe` and `LICENSE`)
for `darwin-arm64` on macOS and `linux-x86_64` on Ubuntu, each natively, and
one job attests both and attaches them and their `.sha256` files to the GitHub
release. Nothing goes to crates.io.

**Tags are pushed by the owner, not by an agent.** What an agent can break is
the packaging, and that is why it is not tag-only: `scripts/package.sh` writes
the tarball, `scripts/check-package.sh` refuses one with the wrong layout,
checksum or platform, or whose binary does not run, and `ci.yml`'s `package`
legs run both on every pull request, for both platforms.
`tests/scripts/test-package.sh` proves the check refuses each of those, and
`tests/scripts/test-release-platforms.sh` fails a pull request in which either
workflow stops packaging a platform, or `release.yml` stops attesting or
attaching one (#45).

## Never grow a shared tool to solve a problem here

**Never grow a shared tool to solve a problem in this repository.** `chore` is
a general-purpose task runner this project merely consumes; the same goes for
`github-guard` and the agent-skills hooks. If something needed here looks like
it belongs inside one of them, it does not. Solve it here, or ask first. The
tell is a release: if a shared tool needs a new version cut whose only purpose
is to unblock this project, the code is in the wrong repository.
