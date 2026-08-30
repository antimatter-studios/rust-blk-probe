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

### M4 — short reads were treated as end-of-file — **fixed**

```rust
let n = f.read(&mut head).unwrap_or(0);
```

Two things wrong in one line. `unwrap_or(0)` discarded an `io::Error` inside a
function that already returns `io::Result`, so **an unreadable device and an
empty file gave the same answer**. And `Read::read` is allowed to return fewer
bytes than asked for without an error and without being at end of input — a
short read is not a short file.

The count is then read as a statement about the file. A read returning 4 makes
the `n >= 8` guards false, so the vhdx and vhd magics become untestable and the
image falls through to `Raw` — the wrong answer, silently, for a file whose
first eight bytes say exactly what it is. The footer check at the bottom had the
same shape with `== 8` standing in for the guard.

`read_up_to` loops until the buffer is full or the source genuinely ends,
retries `Interrupted` (a signal arrived; nothing is wrong) and propagates
everything else. A file shorter than the buffer still reports its length rather
than failing, because a tiny raw image is legitimate — which is why this is not
`read_exact` with its `UnexpectedEof` mapped back.

Four tests, over a reader that hands back one byte per call and one that fails
partway. A local regular file fills a 16-byte buffer in a single call, which is
exactly why the old form survived: it was right on every machine anyone tried
it on. Mutation-checked — restoring `read(buf).unwrap_or(0)` fails three of the
four.

### M2, M3, M7 — magics, sizes and JSON writers — **fixed**

**M2.** Three signatures were readable ASCII at the call site and the fourth was
four loose hex bytes — so the qcow2 arm was the only one a reader could not check
against the format's documentation by eye. `mod magic` names all four, and says
why qcow2 resisted the ASCII spelling: `QFI\xfb` is three printable characters
and one that is not.

The four `if` arms became a list, and a second test asserts **no signature is a
prefix of another** — the probe returns the first match, so a prefixing magic
would shadow the one after it, and the arms no longer have a meaningful order.

**M3.** The bare `512` reads like a sector size and is not one: a VHD footer is
512 bytes because the format says so, and would stay 512 on a 4Kn device.
`VHD_FOOTER_SIZE` says that.

**M7.** Two JSON writers, each with its own format string, sharing `path`,
`container`, `container_size_bytes` and `table`. Nothing made them agree — so
renaming or reordering a key was a change a reader had to remember to make twice,
in a document a consumer parses. `json_envelope` writes the four once; each
caller appends only its own body.

Mutation-checked: a wrong last byte in the qcow2 magic fails 4 tests, a wrong
footer size 3, and reordering the envelope's keys 4.

---

## Verification

40 tests pass, up from 35. `chore lint` clean, and the crate now builds
with no warnings at all — removing `_unused` removed the reason for the
`#[allow]`.
