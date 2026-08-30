# Human-code findings — status

Tracks every **High** and **Medium** finding from
[`human-code-report-2026-08-28.md`](human-code-report-2026-08-28.md). The report
predates the work; this is the current position. Updated 2026-08-30.

**24 findings** — 5 High, 8 Medium, 11 Low. This covers the 13 High and Medium.

| | High | Medium |
|---|---|---|
| Fixed | 5 | 2 |
| Left for a human decision | 0 | 2 |
| Fixable, not yet done | 0 | 4 |

---

## High — all closed

### H1 — the container probe order is load-bearing and nothing said so — **fixed**

`auto_detect_container` now documents which parts of the sequence can move and
which cannot.

Steps 1–4 test offset-0 magics and are mutually exclusive, so their order does
not matter. The trailing-footer VHD check does: **a fixed VHD is byte-for-byte a
raw disk image with a 512-byte footer glued on the end**, so offset 0 is
ordinary partition-table data and the footer is the only thing separating the
two — eight bytes at a position that is payload in every other format. It must
come after the strong magics so any of them wins, and before the `Raw` fallback
or every fixed VHD is reported as raw.

### H2 — zero tests — **fixed earlier**

`70c9353`. 31 tests now.

### H3 — `fs_kind_label` mirrors a foreign enum, held together by a comment — **fixed earlier**

`fs_kind_label_names_every_discriminant_it_mirrors` is now the thing holding
them together.

### H4 — the sniff error sentinel was folded into "recognised nothing" — **fixed earlier**

`70c9353`. Two compounding problems: the `if sniffed >= 0` guard was **inert**
(`fs_kind_label(-1)` already returned `"unknown"`, so both branches produced the
same string), and the error it looked like it was handling was real and
discarded. A read failure, an out-of-range index and a caught panic were
indistinguishable from a partition holding no recognised filesystem.

Now `sniff_outcome` separates them: a failed sniff reports `"fs_kind":"error"`
with the reason, in-band, because the rest of the document is still true.

### H5 — documented exit code 3 could never be returned — **fixed earlier**

Same commit. Exit 3 now means what the docs say: a table is present and could
not be read. A disk with *no* table is a normal answer and exits 0.

---

## Medium

### M6 — `_unused` and its import justified each other — **fixed**

```rust
#[allow(dead_code)]
fn _unused(_a: c_char) {}
```

A function whose only purpose was to stop the compiler complaining about an
import whose only purpose was that function. Both gone; the crate builds without
a warning.

### M8 — the doc-comment's `fs_kind` list omitted values the code emits — **fixed earlier**

### M1 — `main()` is a 143-line god function — **needs your decision**

Accurate. It is also the whole program, and splitting it is a judgement about
where the seams belong.

### M5 — a `writable` parameter that is always `false`, and four unreachable externs behind it — **needs your decision**

The parameter is dead *today*. Whether it is scaffolding for a write mode that
is coming, or leftovers to delete, is a question about intent — and deleting the
externs is the harder half to reverse.

### M2, M3, M4, M7 — magics, sizes, short reads and JSON writers — **fixable, not yet done**

Three magics written as readable ASCII and the fourth as loose hex; `512` for
two different meanings; short reads treated as end-of-file; two hand-rolled JSON
writers sharing four keys and free to drift.

**M4 is the one to do first** — a short read reported as EOF is the same class
of defect as the squashfs gzip bug, where a truncated input was served as a
complete one.

---

## Verification

31 tests pass, unchanged in number. `chore lint` clean, and the crate now builds
with no warnings at all — removing `_unused` removed the reason for the
`#[allow]`.
