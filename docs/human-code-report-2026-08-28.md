# Human-code report — rust-blk-probe

**This document is analysis only. No code was changed, no tests were added,
nothing was committed.** It is the output of Phase 0 (Understand) and Phase 1
(Scan and Triage) of the `human-code` skill, written up in the Phase 3 report
format. Phase 2 — the test-gated fix loop — was deliberately not run. Read
this, decide what you want fixed, and the loop can start from a confirmed
list.

| | |
|---|---|
| **Date** | 2026-08-28 |
| **Scope** | the whole crate — `src/main.rs` (385 lines), `Cargo.toml`, `chores.yml`, `README.md` |
| **Project type** | Rust binary crate (`diskprobe`), edition 2021, toolchain pinned 1.95.0 |
| **Findings** | **19** — 5 High, 8 Medium, 6 Low |
| **Fixed** | 0 (report-only run) |
| **Dismissed during triage** | 5 (recorded below so they don't get re-raised) |
| **Baseline tests** | 0 passing, 0 failing — the crate has no `#[cfg(test)]` block at all |
| **Baseline static analysis** | 16 clippy warnings, all `improper_ctypes` on `FsCoreDevice` |

The whole crate is one file. There is no library surface, no test module, and
no CI workflow; `chore test` runs `cargo test`, which currently reports
`0 passed; 0 failed`.

---

## The headline

Three separate detection tables live in this file, and each one decides
something a reader cannot recover from the code alone:

1. **`auto_detect_container`** (`src/main.rs:102-134`) — container magic, in a
   fixed order, with a fallback to `Raw`.
2. **`fs_kind_label` / `table_kind_label`** (`src/main.rs:176-201`) — integer
   codes to strings, hand-mirroring an enum that lives in another repository.
3. **The partition-table-or-whole-device fork** (`src/main.rs:294`) — one
   branch for "this disk has partitions", one for everything else.

Table 1 is where the ordering question bites, and it is the finding I would
fix first after tests exist. The rest of this document works through all
three.

---

## Findings

Sorted High → Medium → Low. Every item carries a file:line, a category from
the skill's smell list, a severity, and its current test coverage. **The test
coverage column is the same for every single item: none.** That is itself
finding H2, and it is why nothing here should be touched before a test module
exists.

### High

---

#### H1 — The container probe order is load-bearing and nothing says so

- **Where:** `src/main.rs:102-134` (`auto_detect_container`)
- **Category:** comments that explain WHAT, not WHY / missing rationale at a decision point
- **Severity:** High
- **Coverage:** none

The function tries six things in a fixed sequence:

```
1. qcow2   magic at offset 0        (0x51 0x46 0x49 0xfb)
2. vhdx    magic at offset 0        ("vhdxfile")
3. vmdk    magic at offset 0        ("KDMV")
4. vhd     magic at offset 0        ("conectix")
5. vhd     magic at len - 512       ("conectix" in the trailing footer)
6. raw     — fallback, no signature required
```

As a *table* this reads fine. The problem is that the sequence encodes a rule
the code never states, and the rule is not uniform across the six lines:

- **Steps 1–4 are mutually exclusive.** They all read offset 0, and no two of
  the four magics can match the same bytes. Their relative order is therefore
  arbitrary — you could shuffle them freely. But a reader cannot know that
  without independently checking all four magic values for overlap, which is
  exactly the work the ordering appears to be doing for them.
- **Step 5 must come after 1–4, and must come before 6.** This is the one that
  genuinely cannot move. A fixed VHD is byte-for-byte a raw disk image with a
  512-byte footer glued on the end; offset 0 is ordinary partition-table data.
  So the trailing footer is the *only* thing distinguishing a fixed VHD from a
  raw image, and it is a far weaker signal than the offset-0 magics — eight
  bytes at a position that is payload in every other format. Putting it last
  among the signature checks means any strong offset-0 magic wins over it;
  putting it before the `Raw` fallback is what stops every fixed VHD being
  reported as raw.
- **Step 6 is a policy decision, not a detection.** "Nothing matched" is
  reported as `"container": "raw"` and exit 0. The module doc-comment
  (`src/main.rs:10-11`) explains that this is correct for whole-disk `.img` /
  `.dd` dumps — good — but does not say what it costs: a **corrupt** container
  is indistinguishable from a raw disk. A qcow2 file with a damaged header
  does not produce an error; it produces `"container":"raw"` plus whatever the
  partition prober makes of qcow2 metadata read as an MBR.

The contrast with the sibling crate is sharp, and worth pointing at as the
model to follow. `rust-partitions/src/sniff.rs` runs the equivalent gauntlet
for filesystem magic, and annotates the reason at each step —
`// SquashFS first — magic at offset 0, cheap` — so a reader knows which
positions are cost-driven, which are correctness-driven and which are
arbitrary. `auto_detect_container` has one comment (`// Fixed VHD: footer at
file_size - 512.`) and it explains *where* the bytes are, not why that check
is fifth.

**What a fix looks like:** a short block comment above the function stating
the rule — strongest offset-0 signature first (order among them arbitrary,
they cannot collide), then the weak trailing-footer signal, then the
permissive fallback — and a one-line note at the `Ok(Container::Raw)` return
recording that an unrecognised *and* a corrupt container are reported
identically. No behaviour change; this is the highest comprehension payoff in
the crate for the lowest risk.

---

#### H2 — Zero tests, in a crate that is entirely testable pure functions

- **Where:** `src/main.rs` — no `#[cfg(test)]` module anywhere; `cargo test` reports `0 passed; 0 failed`
- **Category:** missing coverage (gates every other item here)
- **Severity:** High
- **Coverage:** none, by definition

Six functions in this file are pure, take no FFI, and are trivial to test:
`Container::parse` (`:90`), `Container::label` (`:80`),
`auto_detect_container` (`:102`, needs only a `tempfile` or a byte slice
behind a small seam), `fs_kind_label` (`:176`), `table_kind_label` (`:195`),
`json_escape` (`:203`) and `fmt_guid` (`:221`). `fmt_guid` in particular
implements the mixed-endian GPT GUID layout by hand — three little-endian
fields then eight big-endian bytes — and has no test asserting a single known
GUID round-trips to its canonical string.

This matters procedurally, not just morally: the `human-code` fix loop is
test-gated. Every other item in this report is a refactor, and refactoring
385 lines of FFI glue with no regression net is how a working binary quietly
stops working. **Nothing else on this list should be attempted first.**

---

#### H3 — `fs_kind_label` mirrors a foreign enum, and a comment is the only thing holding them together

- **Where:** `src/main.rs:176-193` (`fs_kind_label`), `src/main.rs:195-201` (`table_kind_label`)
- **Category:** duplication + fallthrough that hides drift
- **Severity:** High
- **Coverage:** none

The comment at `:177` says `// Mirror the partitions::FsKindCode enum.` That
is an accurate description of the intent and a completely unenforced one. The
authoritative enum lives in `rust-partitions/src/capi.rs:40-54` with twelve
non-zero variants. If a sibling release adds `Btrfs = 13` — plausible, given
the family already has an XFS and a Btrfs driver — this file compiles
unchanged, the new code hits the `_ => "unknown"` arm at `:191`, and
`diskprobe` reports every Btrfs partition as `unknown` with no warning at
build time or run time. The consumer (`FSKitMountService`, per the README)
believes the disk has no recognisable filesystem.

The twelve guard arms already name every variant, so the information needed
to detect drift is present; the catch-all arm is what discards it. Options
worth weighing: a `TryFrom<i32>` on the code with an exhaustive `match` on the
resulting enum (drift becomes a compile error), or, cheaper, a test that
asserts the highest known discriminant still maps to a non-`"unknown"` string
so a new variant fails the suite. `table_kind_label` has the same shape and
the same exposure with three variants instead of thirteen.

---

#### H4 — The sniff error sentinel is folded into the same string as "recognised nothing"

- **Where:** `src/main.rs:296-297` (whole-device path) and `src/main.rs:335-340` (per-partition path)
- **Category:** dense/misleading logic + defensive code that does nothing
- **Severity:** High
- **Coverage:** none

Both call sites do this:

```rust
let sniffed = unsafe { partitions_sniff(list, i) };
let fs_label = if sniffed >= 0 { fs_kind_label(sniffed) } else { "unknown" };
```

Two things are wrong and they compound.

First, **the guard is inert.** `partitions_sniff` returns `-1` on error;
`fs_kind_label(-1)` already falls through to `_ => "unknown"` at `:191`. The
`if sniffed >= 0` test produces the identical string on both branches. It
reads like error handling and is a no-op.

Second, **the error is real and it is being thrown away.** Per
`rust-partitions/src/capi.rs:261-263` and `:293-296`, `-1` means the sniff
*failed* — a read error, an out-of-bounds index, or a caught panic — with the
detail waiting in `fs_core_last_error_message()`. This crate already has a
`last_error()` helper (`:165`) and uses it correctly for the two
container-open failures at `:277` and `:285`. Here it is never called. So an
I/O failure mid-probe and a partition full of unrecognised bytes both emit
`"fs_kind":"unknown"`, and the reason the first one happened is discarded a
few instructions after being recorded.

This is the finding most likely to be actively hiding a bug today.

---

#### H5 — Documented exit code 3 can never be returned

- **Where:** `src/main.rs:17` (doc-comment), `README.md` exit-code table, contradicted at `src/main.rs:294`
- **Category:** comments/docs that lie
- **Severity:** High
- **Coverage:** none

Both the module doc-comment and the README publish a four-value exit contract,
of which `3 — partition probe error` is one. There is no `die(3, ...)` in the
file. The only `die` codes present are `1` (four sites) and `2` (three sites).

The reason it is unreachable is the condition at `:294`:

```rust
if rc != fs_core::ffi::FsCoreErrorCode::Ok || list.is_null() {
```

A genuine probe *failure* and a disk with *no partition table* take the same
branch. That branch emits `"table":"none"`, an empty `partitions` array, and
**exit 0**. A caller cannot distinguish "this is a whole-device filesystem
with no partition table" — a normal, expected, correct answer — from "I could
not read the partition table", which is an error the caller should probably
surface. The published contract says it can.

Either the branch should split (`rc != Ok` → `die(3, ...)`; `list.is_null()`
with `rc == Ok` → the whole-device sniff) or the contract should be corrected
in both places to say exit 3 is unused. The first is almost certainly what was
intended, since the code went to the trouble of documenting it.

---

### Medium

---

#### M1 — `main()` is a 143-line god function

- **Where:** `src/main.rs:238-380`
- **Category:** god function
- **Severity:** Medium
- **Coverage:** none

In one body: argument parsing and flag validation (`:239-260`), container
auto-detection dispatch (`:262-269`), path-to-`CString` conversion and file
open (`:271-278`), container stacking (`:280-287`), device sizing (`:289`),
the partition probe and its whole-device fallback branch including a complete
JSON document and an early `exit(0)` (`:291-308`), a per-partition loop that
hand-initialises a 14-field FFI struct, sniffs, extracts a label from a raw
pointer and formats an entry (`:310-364`), a second complete JSON document
(`:366-374`), and FFI cleanup (`:376-379`).

The abstraction levels are mixed throughout — `std::slice::from_raw_parts` on
a `*const c_char` sits four lines from `format!` string assembly. The
blank-line paragraphs and the section comments (`// Open underlying file.`,
`// Probe partitions.`) are the classic tell: each one names a function that
wants to exist. Natural seams are `parse_args`, `open_device`,
`describe_whole_device`, `describe_partition` and `emit_json`.

Do not attempt this before H2.

---

#### M2 — Three magics are readable ASCII, the fourth is four loose hex bytes

- **Where:** `src/main.rs:107-112`
- **Category:** magic numbers / inconsistent expression of the same idea
- **Severity:** Medium
- **Coverage:** none

```rust
if n >= 4 && head[0] == 0x51 && head[1] == 0x46 && head[2] == 0x49 && head[3] == 0xfb {
```

versus the three checks immediately below it, which are all
`&head[..8] == b"vhdxfile"` in shape. `0x51 0x46 0x49 0xfb` is `b"QFI\xfb"` —
the qcow2 signature, and a recognisable string to anyone who has read the
format spec. Written as four indexed byte comparisons it is the one magic in
the function a reader has to decode by hand, and it is four times as long as
the alternative. The five named container magics also want to be `const` items
at module scope alongside each other rather than inline literals — same domain,
same table, one home.

---

#### M3 — `512` appears three times for two different meanings, `8` once, `16` once

- **Where:** `src/main.rs:105` (`[0u8; 16]`), `:126` (`len >= 512`), `:127` (`len - 512`), `:129` (`== 8`)
- **Category:** magic numbers
- **Severity:** Medium
- **Coverage:** none

`512` at `:126` is a minimum-file-size guard; `512` at `:127` is a footer
offset. They are the same number because they are the same quantity — the VHD
footer size — but the code does not say so, so a reader has to prove it. One
`const VHD_FOOTER_BYTES: u64 = 512;` names both, and the `8` at `:129` is
`b"conectix".len()`.

The `16`-byte `head` buffer at `:105` is separately arbitrary: the longest
magic any check reads is 8 bytes. Sizing it to the longest signature (with the
constant named) removes the question of whether 16 means something.

---

#### M4 — Short reads are silently treated as end-of-file

- **Where:** `src/main.rs:106` and `src/main.rs:129`
- **Category:** defensive code that converts a failure into a wrong answer
- **Severity:** Medium
- **Coverage:** none

```rust
let n = f.read(&mut head).unwrap_or(0);
```

`unwrap_or(0)` discards the `io::Error` — in a function that already returns
`io::Result<Container>` and could simply propagate it with `?`. Worse, a
single `read` is not obliged to fill the buffer even without an error. If it
returns 4, the `n >= 8` guards on the vhdx and vhd checks are false, those
formats become undetectable, and the file falls through to `Raw` with no
diagnostic. The footer read at `:129` has the same shape with an `== 8`
equality test standing in for the guard.

For a regular local file this is close to unobservable, which is precisely why
it would survive a long time before biting. `read_exact` (with its `UnexpectedEof`
mapped deliberately, since a file shorter than the buffer is a legitimate raw
image) is both shorter and correct.

---

#### M5 — A `writable` parameter that is always `false`, and four unreachable externs behind it

- **Where:** `src/main.rs:136-163` (extern block, `open_container_on`), single call site at `:281`
- **Category:** speculative code for a scenario that cannot happen
- **Severity:** Medium
- **Coverage:** none

`open_container_on` is called exactly once, always with `writable = false`.
The four `(_, true)` match arms at `:154`, `:156`, `:158`, `:160` and the four
`*_open_rw_on_device` declarations at `:137`, `:139`, `:141`, `:143` are
unreachable. Half the extern block is a capability with no caller.

The comment at `:273-274` and `:280` explaining *why* read-only ("we don't
want to risk taking a write lock just to inspect") is genuinely good and
should be kept whatever happens to the parameter. The question is only whether
the RW half is being held for a planned feature — if so it wants a note saying
so, because right now it reads as live dispatch.

---

#### M6 — `_unused` and its import justify each other and nothing else

- **Where:** `src/main.rs:39-40` (`use std::os::raw::c_char;`), `src/main.rs:382-385`
- **Category:** speculative code + comment that lies
- **Severity:** Medium
- **Coverage:** none

```rust
// Suppress unused-import lint when this binary is built without
// referring to specific types.
#[allow(dead_code)]
fn _unused(_a: c_char) {}
```

`c_char` is imported at `:40` and used in exactly one place: the signature of
`_unused`. `_unused` exists to give `c_char` a use. Remove the function and
the import has no unused-import lint to suppress, because the import goes with
it. The comment describes a constraint that does not exist. (The raw-pointer
cast at `:343` uses `*const u8`, not `c_char`, so nothing else depends on it.)

Contrast this with the four `use qcow2 as _;` declarations at `:59-66`, whose
comment at `:53-58` explains a real and genuinely non-obvious linker
constraint. That one is exemplary. This one is its cargo-culted echo.

---

#### M7 — Two hand-rolled JSON writers that share four keys and are free to drift

- **Where:** `src/main.rs:298-304` (no-table branch) and `src/main.rs:366-373` (table branch)
- **Category:** duplication
- **Severity:** Medium
- **Coverage:** none

Both are `format!` calls building the same envelope — `path`, `container`,
`container_size_bytes`, `table`, `partitions` — with the first adding
`device_fs_kind` and hardcoding `"table":"none"` and `"partitions":[]`. Any
change to the envelope has to be made twice, in two places 60 lines apart, and
**they have already drifted**: `device_fs_kind` is emitted by the first and
documented in `README.md` but is absent from the module doc-comment's JSON
shape at `:19-37`, which is the copy a maintainer reading this file will
trust.

Two instances is below the skill's three-instance threshold for extracting a
helper, so this is a judgement call rather than an automatic fix. The
argument for acting anyway is that the drift is not hypothetical — it is
already in the file.

---

#### M8 — The doc-comment's `fs_kind` value list omits two values the code emits

- **Where:** `src/main.rs:30` (module doc), `README.md` JSON sample; contradicted by `src/main.rs:179-180`
- **Category:** comments that lie
- **Severity:** Medium
- **Coverage:** none

The doc enumerates
`"ext4"|"ntfs"|"fat32"|"fat16"|"exfat"|"hfs_plus"|"apfs"|"linux_swap"|"iso9660"|"squashfs"|"unknown"`.
`fs_kind_label` also returns `"ext2"` (`:179`) and `"ext3"` (`:180`), which
`am-partitions` distinguishes via feature flags. A consumer writing a `switch`
over the documented set — the README explicitly says `FSKitMountService.swift`
mirrors this JSON shape — silently mishandles every ext2 and ext3 volume.

---

### Low

---

#### L1 — `--container=raw` works but is undocumented

- **Where:** `src/main.rs:92` (accepted by `Container::parse`) vs `:68` (`USAGE`) and the README usage block
- **Category:** misleading docs
- **Severity:** Low
- **Coverage:** none

`USAGE` advertises `--container=qcow2|vhd|vhdx|vmdk`. `raw` parses fine and is
a meaningful request — "skip auto-detection, treat this as a raw image" —
which is exactly what someone with a false-positive footer match would want.
Either document it or reject it; being silently accepted while officially
unknown is the worst of the three.

---

#### L2 — A 14-field FFI struct hand-initialised inside the loop body

- **Where:** `src/main.rs:315-328`
- **Category:** dense block / noise
- **Severity:** Low
- **Coverage:** none

Fourteen lines of zero-initialisation, including two `_pad` arrays that exist
only for layout, re-declared on every iteration and accounting for roughly a
quarter of the loop. `am-partitions` derives `Debug, Clone` on `PartitionInfo`
but not `Default`.

**Trade-off worth stating before anyone acts on this:** the explicit literal
means that if `am-partitions` adds a field, this file fails to compile — which
is the correct and desirable outcome. A `..Default::default()` spread would
*remove* that protection and let a new field silently default. If this gets
tidied, hoist it into a named `fn blank_partition_info() -> PartitionInfo`
that keeps the exhaustive literal, rather than reaching for `Default`.

---

#### L3 — `rc` and `grc` fifty lines apart

- **Where:** `src/main.rs:293` (`rc`), `src/main.rs:329` (`grc`); also `f`/`n`/`p`/`b`/`d1`/`l`/`a` throughout
- **Category:** opaque names
- **Severity:** Low
- **Coverage:** none

Most of the single-letter names are in scopes small enough to be fine, and
`d1/d2/d3` in `fmt_guid` match the field names in the GUID spec, which is a
point in their favour. The one that actually costs a reader something is
`grc` — "get return code", presumably — sitting in the same function as `rc`
with no relationship between them beyond both being return codes. `probe_rc`
and `get_rc`, or better, names describing what failed.

---

#### L4 — 16 clippy warnings, all the same one, all unfixable here

- **Where:** `src/main.rs:137-144` (extern block)
- **Category:** noise that trains readers to ignore warnings
- **Severity:** Low
- **Coverage:** n/a

Every build emits 16 `improper_ctypes` warnings, two per extern declaration,
because `fs_core::ffi::FsCoreDevice` is an opaque handle with no `#[repr(C)]`.
The crate is correct — it only ever passes the pointer through — but the
warning is real, permanent, and drowns out anything new. Either add
`#[repr(C)]` (or `#[repr(transparent)]`) upstream in `rust-fs-core`, or put a
scoped `#[allow(improper_ctypes)]` on this extern block with a one-line
comment saying the handle is opaque and never dereferenced on this side.
Silencing it without the comment would be worse than leaving it.

---

#### L5 — No CI, in a family whose toolchain pin exists because of CI

- **Where:** repository root — no `.github/` directory
- **Category:** process (adjacent to the readability remit, included because it explains the rest)
- **Severity:** Low
- **Coverage:** n/a

`rust-toolchain.toml` carries a comment explaining that the channel is pinned
because a new clippy release turns `-D warnings` into a hard CI error. There
is no CI in this repository to be hard-errored. This is the mechanism by which
16 warnings and 0 tests went unremarked.

---

#### L6 — `dist/diskprobe` is tracked in the working tree while `/dist/` is gitignored

- **Where:** `.gitignore` (`/dist/`), `dist/diskprobe` present on disk
- **Category:** confusing state
- **Severity:** Low
- **Coverage:** n/a

Not a source-readability issue, noted only because someone reading the repo
cold will wonder whether the committed-looking binary is an artefact or an
input. It is an artefact — `chore binary` writes it and `chore artifact`
prints its path — and the ignore rule is correct. No action needed beyond
knowing it.

---

## Considered and dismissed

Recorded so these do not get raised again on the next pass.

| # | Item | Reason |
|---|---|---|
| D1 | The twelve `x if x == FsKindCode::Ext2 as i32 =>` guard arms in `fs_kind_label` look verbose and repetitive | **Acceptable pattern.** Rust will not match an enum discriminant as an integer pattern, and the guard form keeps the mapping tied to the real enum rather than to copied literals. The verbosity is the price of correctness. The catch-all arm is the actual problem — raised separately as H3. |
| D2 | `CString::new(path.as_str()).unwrap()` at `:272` panics instead of exiting cleanly | **False positive.** `path` comes from `std::env::args()`, and argv strings are NUL-terminated by the kernel, so an interior NUL is unreachable. The `unwrap` is provably safe. |
| D3 | `let path = path.unwrap_or_else(|| die(1, USAGE));` at `:260` looks dead — `args` is known non-empty by `:240` | **False positive.** It is reachable: `diskprobe --container=qcow2` gives a non-empty argv with no positional, and this is the line that catches it. |
| D4 | `json_escape` is applied to `path` and `label` but not to `container.label()`, `fs_label` or `fmt_guid`'s output | **False positive.** All three of the unescaped values are `&'static str` from a closed set, or lowercase hex. Escaping them would be noise. |
| D5 | The four `#[allow(unused_imports)] use qcow2 as _;` declarations at `:59-66` | **Acceptable pattern**, and the comment above them at `:53-58` is the best writing in the file — it explains a real linker constraint that is invisible from the code. Leave it exactly as it is. Same for the long rationale comments in `chores.yml`, which pre-empt several questions a reader would otherwise have to ask. |

---

## What to fix first

The order matters, because the first item unblocks the rest and two of the
others change behaviour.

1. **H2 — add the test module.** Not optional and not negotiable as a
   starting point. `Container::parse`, `json_escape`, `fmt_guid`,
   `fs_kind_label`, `table_kind_label` are pure and cost minutes.
   `auto_detect_container` needs a small seam (take a `Read + Seek` rather
   than a path, or write temp files) and is worth the seam — it is the
   function every other high-severity item touches. Nothing below this line
   should start before it is green.

2. **H1 — write down the container probe order.** Zero behaviour risk, and it
   is the specific thing that is obvious to whoever wrote this file and to
   nobody else: which of the six steps can move and which cannot, and what the
   `Raw` fallback costs. Do it as a comment block; do not reorder anything.

3. **H4, then H5 — the two error paths that report success.** H4 discards a
   diagnostic that has already been captured; H5 publishes an exit code the
   code cannot produce and folds a real failure into a normal answer. These
   are the two most likely to be concealing a bug right now, and both are
   small. Both change observable behaviour, hence their position after tests.

4. **H3 — close the enum-drift hole** with either a `TryFrom` or a
   canary test. The sibling crates move independently and this file is
   the seam between them.

5. **M2, M3, M6 — the cheap, purely-local cleanups.** Name the magics, write
   `b"QFI\xfb"` like the other three, delete `_unused` and its import. These
   are safe under test and make the detection table read as a table.

6. **M1 — split `main()`**, last among the substantive work, because it moves
   the most code and benefits most from the tests and the naming fixes
   already being in place.

7. **M4, M5, M7, M8 and the Low tier** as taste and time allow. M8 is a
   one-line doc fix with a real consumer downstream and could be pulled
   forward at any point.

---

## Baseline (for comparison after any future fix run)

| Measure | Before |
|---|---|
| Tests passing | 0 |
| Tests failing | 0 |
| Test files | 0 — no `#[cfg(test)]` module in the crate |
| Coverage | 0% |
| `cargo clippy --all-targets` | 16 warnings (`improper_ctypes` on `FsCoreDevice`, 2 per extern declaration × 8 declarations); 0 errors |
| `cargo test` | `0 passed; 0 failed; 0 ignored` |
| `src/main.rs` | 385 lines, 1 file, longest function `main()` at 143 lines |
| Working tree | clean at the time of this scan (this report is the only new file) |
