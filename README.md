# rust-blk-probe

`diskprobe` — open a disk image (raw, or a qcow2 / VHD / VHDX / VMDK
container), walk its partition table, and emit JSON describing what is
inside.

This repository builds a **command-line binary**, not a library. Every
sibling in the family (`rust-fs-*`, `rust-img-*`, `rust-partitions`) hands
back a static archive and headers; this one hands back one executable that a
host application runs as a child process.

## The name

The **repository** is `rust-blk-probe`. The **crate and the binary** are both
`diskprobe`, and the binary name is load-bearing: DiskJockey stages it as
`lib/diskprobe/diskprobe` and invokes it by that name from
`FSKitMountService`, `AgentImpl` and `SwiftPartitionProbe`. Renaming
`[[bin]]` breaks the app without breaking the build — the compile succeeds
and the app reports *"diskprobe binary not found"* at runtime.

## Usage

```
diskprobe <path>
diskprobe <path> --container=qcow2|vhd|vhdx|vmdk
```

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
| 3 | partition probe error |

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
      "start": 1048576,
      "length": 268435456,
      "fs_kind": "ext4",
      "type_byte": 131,
      "type_guid": "0fc63daf-8483-...",
      "label": "boot"
    }
  ]
}
```

A whole-device filesystem with no partition table reports `"table": "none"`,
an empty `partitions` array, and a `device_fs_kind` field naming what was
sniffed at offset 0.

## Building

```sh
chore binary            # universal arm64+x86_64 binary into ./dist
chore binary out=/some/where
chore build             # plain debug build
chore test
```

`chore binary` is the whole interface a consumer needs: give it an output
directory and it leaves a single `diskprobe` there. It owns the two target
triples, the release profile and the `lipo` step, so nothing outside this
repository has to know them.

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

Extracted from `antimatter-studios/diskjockey` at `vendor/rust-disk-probe`,
with the five commits that touched that path preserved.

## Licence

MIT — see [LICENSE](LICENSE).
