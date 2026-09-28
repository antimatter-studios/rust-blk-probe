# Changelog

Notable changes to `rust-blk-probe`, newest first. This is a `0.x` crate, so the
**minor** is the compatibility boundary: a minor bump may break API, a patch
never does.

## [Unreleased]

This crate is not yet released — there are no tags and nothing is published to
crates.io. Everything below is what the first release will contain.

### Added

- **A partition-table probe CLI.** Reports the table type and partition layout
  of a device, and sniffs a filesystem on a device with no table at all.
- A lint gate and CI.
- **Every test tier is budgeted, logged and floored** (#17). `scripts/tier.sh`
  runs a tier quietly — the transcript goes to `tmp/logs/<tier>.log`, a pass
  prints one verdict line naming it, and a run that passed but printed more
  than its measured budget exits 65. `scripts/test-floor.sh` fails a tier that
  executed fewer tests than its floor, which is the failure a budget cannot
  see: `cargo test` exits 0 on "0 passed". The wrapper itself is
  `rust-fs-core`'s and is resolved at run time rather than committed here.
- **A release-profile tier.** Every test used to run only against
  `opt-level = 1` with `debug_assert!` bodies live and overflow checks on,
  while the shipped binary is built with `opt-level = 3`, LTO and none of
  those (#13).
- CI uploads `tmp/logs/` as an artifact on every leg, with `if: always()`.
- **The probe is a library, and the binary is a front for it** (#16).
  `blk_probe::probe_path` is what the CLI runs; `blk_probe::probe_bytes` is
  the same probe over an image already in memory, backed by an `FsCoreDevice`
  built from callbacks rather than a file.
- **Two fuzzing tiers over one corpus.** `fuzz/` holds `cargo-fuzz` targets
  for the whole-device probe and for an image wearing another format's magic;
  `tests/fuzz_decoders.rs` replays and mutates the same corpus on the stable
  toolchain in every pull request, under a deadline and a floor of 15,000
  executed cases.
- `scripts/make-fuzz-corpus.sh` builds the corpus from images `sgdisk`,
  `sfdisk`, `mkfs.ext4`, `mksquashfs` and `qemu-img` wrote, and those images
  are the oracle for what the probe reports about them.

### Fixed

- **A use-after-free on a container that fails to open.** `*_open_on_device`
  consumes the handle it is given before it parses anything, so closing the
  inner device after a NULL return frees it twice; a truncated VHDX crashed
  the probe with SIGBUS instead of exiting 2. Found by the fuzz corpus on the
  first run of the gate that replayed it.
- The toolchain is pinned, matching the sibling crates.
- **One check gates a merge, and it stands for every job.** `ci.yml` grew a
  `ci-ok` job that `needs:` every other job in the workflow and fails when any
  of them failed, was cancelled or was **skipped**; `.github-guard` requires it
  and nothing else, in place of the three names it pinned before (`fmt`,
  `test / ubuntu-latest`, `test / macos-latest`). Two of those three were legs
  of one matrix, so adding a leg — the `ubuntu-24.04-arm` one #15 needs —
  would have produced a check that reports on every pull request and gates
  nothing. `tests/ci_aggregate_gate.rs` holds both halves to it: every job in
  `ci.yml` must appear in `ci-ok`'s `needs`, `ci-ok` must carry `if: always()`,
  and `.github-guard` must require `ci-ok` alone.

### Changed

- **The package is `rust-blk-probe` and the binary is `blk-probe`.** Both were
  `diskprobe`. The package now matches its repository, as every sibling will,
  and the binary matches the package. **A consumer that stages or runs the
  binary by name must change with it**: the build writes `dist/blk-probe`, and
  error output is prefixed `blk-probe:`.
- **Builds into its own `dist/` and returns that path**, rather than writing
  into whatever consumes it. Where the output lands is this crate's business,
  not its consumer's.
- **The container magics have names, and the JSON envelope is written once.**

### Fixed

- **A failed probe is distinguishable from a disk that simply has no partition
  table.** Both had been reported the same way, so a consumer could not tell
  "this disk is unpartitioned" from "I could not read this disk".
- **A short read is not a short file.** Treating one as the other truncates
  content silently instead of erroring.
