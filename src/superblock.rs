//! The filesystems this crate identifies from their own superblocks.
//!
//! `am-partitions` sniffs twelve filesystems and answers `Unknown` for the
//! rest. Three of the rest are XFS, Btrfs and EROFS, and it cannot simply be
//! asked about them: its `FsKindCode` has no discriminant for any of the
//! three, and its sniff window ends at 64 KiB, which is exactly where the
//! Btrfs superblock begins. So they are recognised here, from a few fields of
//! each superblock read through the device itself.
//!
//! **This runs only after that sniff has answered `Unknown`**, never instead
//! of an answer it gave, so nothing it already recognised can be renamed by
//! a match here.
//!
//! Like that sniff, this says what a region *is likely to be*; it does not
//! validate the filesystem. Each rule is the magic plus the fields that give
//! the rest of the superblock its meaning, so that eight right bytes in the
//! middle of some other structure are not enough on their own. The layouts
//! are the formats' own (`xfs_format.h`, `btrfs_tree.h`, `erofs_fs.h`), and
//! `tests/oracle_tools.rs` holds every answer to what `blkid -p` says about
//! an image each format's own mkfs wrote.

/// A read of `buf.len()` bytes at `offset`, relative to the start of the
/// region being identified. The caller has already checked that the range is
/// inside the region, so a failure is a real I/O error.
pub type ReadAt<'a> = dyn FnMut(u64, &mut [u8]) -> Result<(), String> + 'a;

/// One rule: where its superblock is, how much of it is read, and whether
/// those bytes are that filesystem.
struct Rule {
    kind: &'static str,
    offset: u64,
    len: usize,
    matches: fn(&[u8]) -> bool,
}

/// XFS: the primary superblock is sector 0 of the filesystem, big-endian.
pub const XFS_MAGIC: u32 = 0x5846_5342; // "XFSB"

/// Btrfs: the primary superblock is at 64 KiB; its magic is 64 bytes in.
pub const BTRFS_SUPERBLOCK_OFFSET: u64 = 0x1_0000;
pub const BTRFS_MAGIC: &[u8; 8] = b"_BHRfS_M";

/// EROFS: the superblock is at 1 KiB, little-endian.
pub const EROFS_SUPERBLOCK_OFFSET: u64 = 1024;
pub const EROFS_MAGIC: u32 = 0xE0F5_E1E2;

const RULES: &[Rule] = &[
    Rule {
        kind: "xfs",
        offset: 0,
        len: 128,
        matches: is_xfs,
    },
    Rule {
        kind: "erofs",
        offset: EROFS_SUPERBLOCK_OFFSET,
        len: 16,
        matches: is_erofs,
    },
    Rule {
        kind: "btrfs",
        offset: BTRFS_SUPERBLOCK_OFFSET,
        len: 72,
        matches: is_btrfs,
    },
];

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn le64(b: &[u8], at: usize) -> u64 {
    let mut v = [0u8; 8];
    v.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(v)
}

/// `struct xfs_dsb`, big-endian: `sb_magicnum` at 0, `sb_blocksize` at 4,
/// `sb_agcount` at 88, `sb_sectsize` at 102, `sb_blocklog` at 120 and
/// `sb_sectlog` at 121.
///
/// The block and sector sizes are each stored twice, once as a size and once
/// as a log2, and must agree -- which is the check that turns four magic
/// bytes into a superblock. The ranges are the format's: a block of 512 bytes
/// to 64 KiB, a sector of 512 bytes to 32 KiB, and at least one allocation
/// group.
pub fn is_xfs(sb: &[u8]) -> bool {
    if sb.len() < 122 || be32(sb, 0) != XFS_MAGIC {
        return false;
    }
    let block = be32(sb, 4);
    let sector = u32::from(be16(sb, 102));
    let (block_log, sector_log) = (u32::from(sb[120]), u32::from(sb[121]));
    (9..=16).contains(&block_log)
        && block == 1 << block_log
        && (9..=15).contains(&sector_log)
        && sector == 1 << sector_log
        && be32(sb, 88) > 0
}

/// `struct btrfs_super_block`, little-endian: `bytenr` at 48, the magic at
/// 64.
///
/// `bytenr` is where the superblock says it lives. The primary copy's is its
/// own offset, 64 KiB, so a stray `_BHRfS_M` -- or a mirror copy seen through
/// the wrong window -- does not pass for a primary superblock.
pub fn is_btrfs(sb: &[u8]) -> bool {
    sb.len() >= 72 && &sb[64..72] == BTRFS_MAGIC && le64(sb, 48) == BTRFS_SUPERBLOCK_OFFSET
}

/// `struct erofs_super_block`, little-endian: the magic at 0 and `blkszbits`
/// at 12, the log2 of the block size, which the format allows from 512 bytes
/// to 64 KiB.
pub fn is_erofs(sb: &[u8]) -> bool {
    sb.len() >= 13 && le32(sb, 0) == EROFS_MAGIC && (9..=16).contains(&sb[12])
}

/// Identify a region of `available` bytes by its superblock.
///
/// `Ok(None)` means none of the three is there -- including when the region
/// is too short to hold the superblock a rule reads, which is a region that
/// cannot be that filesystem rather than a failure. `Err` is a read that
/// failed inside the region.
pub fn identify(available: u64, read_at: &mut ReadAt<'_>) -> Result<Option<&'static str>, String> {
    for rule in RULES {
        let Some(end) = rule.offset.checked_add(rule.len as u64) else {
            continue;
        };
        if end > available {
            continue;
        }
        let mut buf = vec![0u8; rule.len];
        read_at(rule.offset, &mut buf)?;
        if (rule.matches)(&buf) {
            return Ok(Some(rule.kind));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fields `is_xfs` reads, as `mkfs.xfs` (xfsprogs 6.6) wrote them
    /// for a 320 MiB image, and nothing else.
    fn xfs_superblock() -> Vec<u8> {
        let mut sb = vec![0u8; 128];
        sb[0..4].copy_from_slice(b"XFSB");
        sb[4..8].copy_from_slice(&4096u32.to_be_bytes());
        sb[88..92].copy_from_slice(&4u32.to_be_bytes());
        sb[100..102].copy_from_slice(&0xb4a5u16.to_be_bytes());
        sb[102..104].copy_from_slice(&512u16.to_be_bytes());
        sb[120] = 12;
        sb[121] = 9;
        sb
    }

    fn btrfs_superblock() -> Vec<u8> {
        let mut sb = vec![0u8; 72];
        sb[48..56].copy_from_slice(&BTRFS_SUPERBLOCK_OFFSET.to_le_bytes());
        sb[64..72].copy_from_slice(BTRFS_MAGIC);
        sb
    }

    fn erofs_superblock() -> Vec<u8> {
        let mut sb = vec![0u8; 16];
        sb[0..4].copy_from_slice(&EROFS_MAGIC.to_le_bytes());
        sb[12] = 12;
        sb
    }

    /// A region held in memory, read the way `identify` reads a device.
    fn identify_bytes(region: &[u8]) -> Result<Option<&'static str>, String> {
        identify(region.len() as u64, &mut |offset, buf| {
            let start = offset as usize;
            buf.copy_from_slice(&region[start..start + buf.len()]);
            Ok(())
        })
    }

    fn region_with(at: u64, sb: &[u8], len: usize) -> Vec<u8> {
        let mut region = vec![0u8; len];
        region[at as usize..at as usize + sb.len()].copy_from_slice(sb);
        region
    }

    #[test]
    fn each_superblock_is_named_where_its_format_puts_it() {
        let cases = [
            ("xfs", 0, xfs_superblock()),
            ("erofs", EROFS_SUPERBLOCK_OFFSET, erofs_superblock()),
            ("btrfs", BTRFS_SUPERBLOCK_OFFSET, btrfs_superblock()),
        ];
        for (kind, at, sb) in cases {
            let region = region_with(at, &sb, 0x2_0000);
            assert_eq!(identify_bytes(&region), Ok(Some(kind)), "{kind}");
        }
    }

    #[test]
    fn a_zeroed_region_is_none_of_them() {
        assert_eq!(identify_bytes(&vec![0u8; 0x2_0000]), Ok(None));
    }

    /// A region too short to hold a superblock cannot be that filesystem,
    /// and must not be read past its end to find out.
    #[test]
    fn a_region_too_short_for_a_superblock_is_not_read_past() {
        let sb = btrfs_superblock();
        let full = region_with(BTRFS_SUPERBLOCK_OFFSET, &sb, 0x2_0000);
        let short = &full[..(BTRFS_SUPERBLOCK_OFFSET as usize + 71)];
        assert_eq!(identify_bytes(short), Ok(None));
        assert_eq!(identify_bytes(&[]), Ok(None));
    }

    /// The magic alone is not enough: each rule's other fields have to agree.
    #[test]
    fn a_magic_whose_fields_disagree_is_not_a_superblock() {
        let mut xfs = xfs_superblock();
        xfs[120] = 13; // blocklog says 8 KiB, blocksize says 4 KiB
        assert!(!is_xfs(&xfs));

        let mut xfs = xfs_superblock();
        xfs[88..92].copy_from_slice(&0u32.to_be_bytes()); // no allocation groups
        assert!(!is_xfs(&xfs));

        let mut btrfs = btrfs_superblock();
        btrfs[48..56].copy_from_slice(&0x400_0000u64.to_le_bytes()); // the 64 MiB mirror
        assert!(!is_btrfs(&btrfs));

        let mut erofs = erofs_superblock();
        erofs[12] = 8; // 256-byte blocks
        assert!(!is_erofs(&erofs));
        erofs[12] = 17; // 128 KiB blocks
        assert!(!is_erofs(&erofs));
    }

    #[test]
    fn a_failed_read_is_reported_rather_than_read_as_no_match() {
        let got = identify(0x2_0000, &mut |_, _| Err("device went away".to_string()));
        assert_eq!(got, Err("device went away".to_string()));
    }
}
