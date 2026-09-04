# Changelog

Notable changes to `diskprobe`, newest first. This is a `0.x` crate, so the
**minor** is the compatibility boundary: a minor bump may break API, a patch
never does.

## [Unreleased]

This crate is not yet released — there are no tags and nothing is published to
crates.io. Everything below is what the first release will contain.

### Added

- **A partition-table probe CLI.** Reports the table type and partition layout
  of a device, and sniffs a filesystem on a device with no table at all.
- A lint gate and CI.
- The toolchain is pinned, matching the sibling crates.

### Changed

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
