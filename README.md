# rust-blk-probe

`blk.probe` — open a disk image (raw, or a qcow2 / VHD / VHDX / VMDK
container), walk its partition table, and emit JSON describing what is
inside.

**The interface is a command-line binary.** Every sibling in the family
(`rust-fs-*`, `rust-img-*`, `rust-partitions`) hands back a static archive and
headers; this one hands back one executable that a host application runs as a
child process. There is no `staticlib` task here and nothing links this crate.

The probe itself is a **library beside it**, and that is a testing decision
rather than a second interface. `blk_probe::probe_path` is what the binary
runs and `blk_probe::probe_bytes` is the same probe over an image already in
memory. A probe reachable only by running a process cannot be fuzzed, cannot
be compared against `blkid` from a test, and can only be unit-tested from
inside its own `main.rs` — which is what this crate was, and why it was the
one component of the family with no fuzz target.

## The name

The **repository and the crate** are both `rust-blk-probe`, matching every
sibling. The **tool** is `blk.probe`, dotted the way `mkfs.ext4` is, and it has no
hyphenated spelling.

Cargo refuses a dot in a target name, so the `[[bin]]` target is `blk_probe`
and `scripts/stage-binary.sh` renames it on the way out: what `chore binary`
leaves in `dist/` is `blk.probe`, and nothing named after the cargo target
is shipped. The name is load-bearing: a consumer that stages the file and
invokes it by name, rather than resolving it on PATH, builds cleanly and
then fails at run time if the two drift.
`tests/scripts/test-staged-binary-name.sh` builds the target, stages it and
runs what was staged.

## Usage

```
blk.probe <path>
blk.probe <path> --container=qcow2|vhd|vhdx|vmdk
blk.probe --version
```

`--version` prints `blk.probe (rust-blk-probe) <version>` and exits 0, the
same `<tool> (<package>) <version>` line every tool in the family answers
with. It probes nothing, wherever it appears on the line.

With `--container` omitted the container kind is auto-detected from the magic
at offset 0, or from the trailing 512-byte footer for fixed VHDs. If nothing
is recognised the file is treated as a raw disk image, which is correct for
whole-disk `.img` / `.dd` dumps.

Exit codes:

| code | meaning |
|-----:|---------|
| 0 | JSON written to stdout |
| 1 | argument / option error |
| 2 | file open / container layer error |
| 3 | partition probe error — the table could not be read |

A disk with **no** partition table is not exit 3. That is a normal answer,
described below, and it exits 0. Exit 3 is the opposite case: a table is
there and `blk.probe` could not read it — a CRC mismatch, a truncated
table, an I/O failure — and nothing is written to stdout.

## Output

```json
{
  "path": "/path/to/file",
  "container": "qcow2|vhd|vhdx|vmdk|raw",
  "container_size_bytes": 12345,
  "table": "gpt|mbr|none",
  "partitions": [
    {
      "index": 0,
      "slot": 0,
      "start": 1048576,
      "length": 268435456,
      "available_length": 268435456,
      "issues": 0,
      "fs_kind": "ext4",
      "type_byte": 131,
      "type_guid": "0fc63daf-8483-...",
      "label": "boot"
    }
  ]
}
```

`slot` is the entry's zero-based slot in the on-disk table (the system's
partition number is `slot + 1`), and is absent for an entry that has none. It
is not `index`: a table with a hole in it is routine.

`length` is what the table claims. `available_length` is how many of those
bytes the image actually holds — equal to `length` for every partition that
fits, less for one running past the end of an image that stops early (a `dd`
that ended short, a table left stale after a shrink), and 0 for one starting
past the end. Size a buffer with `available_length`.

`issues` is a bit set of the table rules the entry breaks, as
`am-partitions`' `PARTITIONS_ENTRY_*` bits: 1 starts before the first usable
LBA, 2 ends past the last usable LBA, 4 overlaps another entry. 0 means none.
A non-zero value is reported, not refused.

`fs_kind` is one of `ext2`, `ext3`, `ext4`, `ntfs`, `fat32`, `fat16`,
`exfat`, `hfs_plus`, `apfs`, `linux_swap`, `iso9660`, `squashfs`, `xfs`,
`btrfs`, `erofs`, `unknown` or `error`.

A whole-device filesystem with no partition table reports `"table": "none"`,
an empty `partitions` array, and a `device_fs_kind` field naming what was
sniffed at offset 0.

### When a filesystem sniff fails

`unknown` means the sniff ran and recognised nothing. A sniff that *failed*
is reported separately, in-band rather than by exit code, because the rest
of the document is still true — the partition table was read, the other
partitions are described correctly, and throwing that away over one
unreadable partition would help nobody. The affected entry carries
`"fs_kind": "error"` and an `"fs_kind_error"` field holding the reason, and
the reason is also written to stderr:

```json
{ "index": 1, "fs_kind": "error", "fs_kind_error": "read failed at offset 1048576" }
```

The whole-device equivalents are `device_fs_kind` and `device_fs_error`.
Consumers switching on `fs_kind` should treat `error` as "ask again", not
as "no filesystem here".

## Building

```sh
chore binary            # universal arm64+x86_64 binary into ./dist
chore binary out=/some/where
chore build             # plain debug build
chore test
```

`chore binary` is the whole interface a consumer needs: give it an output
directory and it leaves a single `blk.probe` there. It owns the two target
triples, the release profile, the `lipo` step and the rename from the cargo
target, so nothing outside this
repository has to know them.

## Release tarball

A version tag publishes one tarball per platform on the GitHub release --
`rust-blk-probe-<version>-darwin-arm64.tar.gz` and
`rust-blk-probe-<version>-linux-x86_64.tar.gz` -- each with a `.sha256` beside
it and a build-provenance attestation, and each laid out the same:

```
bin/blk.probe
LICENSE
```

```sh
gh attestation verify rust-blk-probe-<version>-<platform>.tar.gz \
  --repo antimatter-studios/rust-blk-probe \
  --signer-workflow antimatter-studios/rust-blk-probe/.github/workflows/release.yml
```

## Dependencies

Six sibling checkouts, resolved by `path = "../rust-*"` from this
repository's parent directory:

| crate | repository |
|---|---|
| `am-fs-core` | `rust-fs-core` |
| `am-img-qcow2` | `rust-img-qcow2` |
| `am-img-vhd` | `rust-img-vhd` |
| `am-img-vhdx` | `rust-img-vhdx` |
| `am-img-vmdk` | `rust-img-vmdk` |
| `am-partitions` | `rust-partitions` |

Check them out beside this one.

## History

Extracted from the application repository that first used it, at
`vendor/rust-disk-probe`, with the five commits that touched that path
preserved.

## Licence

MIT — see [LICENSE](LICENSE).
