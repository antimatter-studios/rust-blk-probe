//! diskprobe — open a disk image (raw or container), walk its partition
//! table, emit JSON describing what's inside.
//!
//! Usage:
//!   diskprobe <path>
//!   diskprobe <path> --container=qcow2|vhd|vhdx|vmdk
//!
//! When `--container` is omitted, container kind is auto-detected from
//! the magic at offset 0 (or the trailing 512-byte footer for fixed
//! VHDs). If no container is recognised the file is treated as a raw
//! disk image (which is correct for whole-disk `.img` / `.dd` dumps).
//!
//! Exit codes:
//!   0  — JSON written to stdout
//!   1  — argument / option error
//!   2  — file open / container layer error
//!   3  — partition probe error: the table could not be read
//!
//! A disk with *no* partition table is not exit 3. That is a normal answer
//! — a whole-device filesystem — and it exits 0 with `"table":"none"` plus a
//! `device_fs_kind` naming whatever was sniffed at offset 0. Exit 3 is the
//! opposite case: a table is there and this program could not read it (CRC
//! mismatch, truncation, I/O failure), so nothing is written to stdout.
//!
//! A failed *filesystem sniff* is reported in-band instead, because the rest
//! of the document is still true. The partition gets `"fs_kind":"error"` and
//! an `"fs_kind_error"` holding the reason; the whole-device equivalents are
//! `"device_fs_kind"` and `"device_fs_error"`. `"unknown"` therefore means
//! only what it says — the sniff ran and recognised nothing.
//!
//! JSON shape:
//!   {
//!     "path": "/path/to/file",
//!     "container": "qcow2"|"vhd"|"vhdx"|"vmdk"|"raw",
//!     "container_size_bytes": 12345,        // virtual size after container unwrap
//!     "table": "gpt"|"mbr"|"none",
//!     "device_fs_kind": "ext4",             // only when "table" is "none"
//!     "device_fs_error": "read failed: ..", // only when that sniff failed
//!     "partitions": [
//!       {
//!         "index": 0,
//!         "start": 1048576,
//!         "length": 268435456,
//!         "fs_kind": "ext2"|"ext3"|"ext4"|"ntfs"|"fat32"|"fat16"|"exfat"|"hfs_plus"|"apfs"|"linux_swap"|"iso9660"|"squashfs"|"unknown"|"error",
//!         "fs_kind_error": "read failed: ..", // only when fs_kind is "error"
//!         "type_byte": 131,                 // MBR partition type byte (0 for GPT)
//!         "type_guid": "0fc63daf-8483-...", // GPT type GUID (zeros for MBR)
//!         "label": "boot"                   // optional, may be absent
//!       },
//!       ...
//!     ]
//!   }

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

use fs_core::ffi::{
    fs_core_device_close, fs_core_device_size_bytes, fs_core_file_open, fs_core_last_error_message,
    FsCoreDevice, FsCoreErrorCode,
};
use partitions::capi::{
    partitions_count, partitions_get, partitions_list_free, partitions_probe, partitions_sniff,
    partitions_sniff_device, partitions_table_kind, FsKindCode, PartitionInfo, PartitionList,
    TableKindCode,
};

// Force the container-reader rlibs to be linked. We only call into them
// via extern "C" declarations below (since we don't want to depend on
// each crate's Rust API surface), but cargo would otherwise drop the
// rlibs entirely because nothing in this crate refers to them by name
// in Rust source. The `use ... as _;` keeps the rlib in the link line
// so the `#[no_mangle]` symbols resolve.
#[allow(unused_imports)]
use qcow2 as _;
#[allow(unused_imports)]
use vhd as _;
#[allow(unused_imports)]
use vhdx as _;
#[allow(unused_imports)]
use vmdk as _;

const USAGE: &str = "usage: diskprobe <path> [--container=qcow2|vhd|vhdx|vmdk]";

// The exit-code table, named. It is published in three places — the module
// doc-comment above, the README, and here — and the point of naming the
// codes is that the `die` call sites say which contract line they are.
const EXIT_ARG_ERROR: i32 = 1;
const EXIT_OPEN_ERROR: i32 = 2;
const EXIT_PROBE_ERROR: i32 = 3;

/// The exact text `partitions::Error::NoPartitionTable` renders. See
/// [`classify_probe`] for why matching on a message, rather than on a
/// return code, is the only option available here.
const NO_PARTITION_TABLE_MESSAGE: &str = "no GPT or MBR signature found";

/// `fs_kind` / `device_fs_kind` value meaning the sniff itself failed — as
/// distinct from `"unknown"`, which means it ran and recognised nothing.
const SNIFF_FAILED_LABEL: &str = "error";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Container {
    Raw,
    Qcow2,
    Vhd,
    Vhdx,
    Vmdk,
}

impl Container {
    fn label(self) -> &'static str {
        match self {
            Container::Raw => "raw",
            Container::Qcow2 => "qcow2",
            Container::Vhd => "vhd",
            Container::Vhdx => "vhdx",
            Container::Vmdk => "vmdk",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "raw" => Some(Container::Raw),
            "qcow2" => Some(Container::Qcow2),
            "vhd" => Some(Container::Vhd),
            "vhdx" => Some(Container::Vhdx),
            "vmdk" => Some(Container::Vmdk),
            _ => None,
        }
    }
}

fn auto_detect_container(path: &str) -> std::io::Result<Container> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let mut head = [0u8; 16];
    let n = f.read(&mut head).unwrap_or(0);
    if n >= 4 && head[0] == 0x51 && head[1] == 0x46 && head[2] == 0x49 && head[3] == 0xfb {
        return Ok(Container::Qcow2);
    }
    if n >= 8 && &head[..8] == b"vhdxfile" {
        return Ok(Container::Vhdx);
    }
    if n >= 4 && &head[..4] == b"KDMV" {
        return Ok(Container::Vmdk);
    }
    if n >= 8 && &head[..8] == b"conectix" {
        return Ok(Container::Vhd);
    }
    // Fixed VHD: footer at file_size - 512.
    let len = f.metadata()?.len();
    if len >= 512 {
        f.seek(SeekFrom::Start(len - 512))?;
        let mut footer = [0u8; 8];
        if f.read(&mut footer).unwrap_or(0) == 8 && &footer == b"conectix" {
            return Ok(Container::Vhd);
        }
    }
    Ok(Container::Raw)
}

extern "C" {
    fn qcow2_open_rw_on_device(inner: *mut FsCoreDevice) -> *mut FsCoreDevice;
    fn qcow2_open_on_device(inner: *mut FsCoreDevice) -> *mut FsCoreDevice;
    fn vhd_open_rw_on_device(inner: *mut FsCoreDevice) -> *mut FsCoreDevice;
    fn vhd_open_on_device(inner: *mut FsCoreDevice) -> *mut FsCoreDevice;
    fn vhdx_open_rw_on_device(inner: *mut FsCoreDevice) -> *mut FsCoreDevice;
    fn vhdx_open_on_device(inner: *mut FsCoreDevice) -> *mut FsCoreDevice;
    fn vmdk_open_rw_on_device(inner: *mut FsCoreDevice) -> *mut FsCoreDevice;
    fn vmdk_open_on_device(inner: *mut FsCoreDevice) -> *mut FsCoreDevice;
}

unsafe fn open_container_on(
    inner: *mut FsCoreDevice,
    kind: Container,
    writable: bool,
) -> *mut FsCoreDevice {
    match (kind, writable) {
        (Container::Raw, _) => inner,
        (Container::Qcow2, true) => qcow2_open_rw_on_device(inner),
        (Container::Qcow2, false) => qcow2_open_on_device(inner),
        (Container::Vhd, true) => vhd_open_rw_on_device(inner),
        (Container::Vhd, false) => vhd_open_on_device(inner),
        (Container::Vhdx, true) => vhdx_open_rw_on_device(inner),
        (Container::Vhdx, false) => vhdx_open_on_device(inner),
        (Container::Vmdk, true) => vmdk_open_rw_on_device(inner),
        (Container::Vmdk, false) => vmdk_open_on_device(inner),
    }
}

fn last_error() -> String {
    unsafe {
        let p = fs_core_last_error_message();
        if p.is_null() {
            String::new()
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

fn fs_kind_label(code: i32) -> &'static str {
    // Mirror the partitions::FsKindCode enum.
    match code {
        x if x == FsKindCode::Ext2 as i32 => "ext2",
        x if x == FsKindCode::Ext3 as i32 => "ext3",
        x if x == FsKindCode::Ext4 as i32 => "ext4",
        x if x == FsKindCode::Ntfs as i32 => "ntfs",
        x if x == FsKindCode::ExFat as i32 => "exfat",
        x if x == FsKindCode::Fat32 as i32 => "fat32",
        x if x == FsKindCode::Fat16 as i32 => "fat16",
        x if x == FsKindCode::HfsPlus as i32 => "hfs_plus",
        x if x == FsKindCode::Apfs as i32 => "apfs",
        x if x == FsKindCode::LinuxSwap as i32 => "linux_swap",
        x if x == FsKindCode::Iso9660 as i32 => "iso9660",
        x if x == FsKindCode::Squashfs as i32 => "squashfs",
        _ => "unknown",
    }
}

fn table_kind_label(code: i32) -> &'static str {
    match code {
        x if x == TableKindCode::Gpt as i32 => "gpt",
        x if x == TableKindCode::Mbr as i32 => "mbr",
        _ => "none",
    }
}

/// What `partitions_probe` actually said.
#[derive(Debug, PartialEq, Eq)]
enum ProbeOutcome {
    /// A GPT or MBR table was parsed and the list handle is usable.
    Table,
    /// The device carries no partition table. Not an error — a
    /// whole-device filesystem is an ordinary disk.
    NoTable,
    /// The probe failed, and this is why.
    Failed(String),
}

/// Tell "this disk has no partition table" apart from "I could not read
/// the partition table".
///
/// The return code alone cannot do it. `partitions_probe` lifts every
/// `partitions::Error` — a missing signature, a GPT CRC mismatch, a short
/// read — through `fs_core::Error::Custom`, so a corrupt table and an
/// unpartitioned ext4 image arrive here as the same `FsCoreErrorCode`. The
/// only thing that crosses the boundary still carrying the distinction is
/// the last-error text, so that is what this matches on.
///
/// That makes this function a coupling to a sibling crate's error *string*,
/// which is worth saying out loud: if `partitions::Error::NoPartitionTable`
/// is ever reworded, every unpartitioned image starts exiting 3. The
/// alternative — folding both cases into exit 0, which is what this code
/// used to do — is worse, because it is wrong silently rather than loudly.
/// A dedicated `FsCoreErrorCode` for "no table" upstream would retire this.
///
/// `last_error` is consulted only when `rc` is non-Ok: the thread-local it
/// is read from can hold a stale message from an earlier call otherwise.
fn classify_probe(rc: FsCoreErrorCode, list_is_null: bool, last_error: &str) -> ProbeOutcome {
    if rc == FsCoreErrorCode::Ok {
        // The ABI promises a non-NULL list whenever it reports success, so
        // a NULL one is a broken promise — something to report, not a disk
        // that happens to have no partitions.
        return if list_is_null {
            ProbeOutcome::Failed("reported success but returned a NULL partition list".to_string())
        } else {
            ProbeOutcome::Table
        };
    }
    if last_error.trim() == NO_PARTITION_TABLE_MESSAGE {
        return ProbeOutcome::NoTable;
    }
    ProbeOutcome::Failed(if last_error.is_empty() {
        format!("failed with {rc:?} (no detail recorded)")
    } else {
        last_error.to_string()
    })
}

/// Interpret a `partitions_sniff` / `partitions_sniff_device` return value.
///
/// `-1` is the documented error sentinel, with the reason waiting in
/// `fs_core_last_error_message()`. Every other value is an `FsKindCode`
/// discriminant, of which `Unknown` (0) means the sniff ran to completion
/// and matched nothing.
///
/// Those are two different facts and this is where they stop being the same
/// string. Passing the sentinel straight to [`fs_kind_label`] does not work:
/// -1 lands on that function's catch-all arm and comes back as `"unknown"`,
/// which is exactly what a successful no-match returns. The `detail` closure
/// is only called on the error path, so the last-error thread-local is read
/// only when it holds something relevant.
fn sniff_outcome(code: i32, detail: impl FnOnce() -> String) -> Result<&'static str, String> {
    if code < 0 {
        let detail = detail();
        Err(if detail.is_empty() {
            format!("sniff returned {code} with no detail recorded")
        } else {
            detail
        })
    } else {
        Ok(fs_kind_label(code))
    }
}

/// Render the `,"<key>":"<reason>"` fragment that accompanies a
/// `SNIFF_FAILED_LABEL`, or nothing at all when the sniff succeeded.
fn sniff_error_field(key: &str, detail: Option<&str>) -> String {
    match detail {
        Some(detail) => format!(",\"{}\":\"{}\"", key, json_escape(detail)),
        None => String::new(),
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

fn fmt_guid(b: &[u8; 16]) -> String {
    // Standard GPT GUID byte layout: first 4 bytes little-endian, next 2 LE,
    // next 2 LE, last 8 bytes big-endian.
    let d1 = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let d2 = u16::from_le_bytes([b[4], b[5]]);
    let d3 = u16::from_le_bytes([b[6], b[7]]);
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        d1, d2, d3, b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

fn die(code: i32, msg: &str) -> ! {
    eprintln!("diskprobe: {msg}");
    std::process::exit(code);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        std::process::exit(0);
    }
    let mut path: Option<String> = None;
    let mut explicit: Option<Container> = None;
    for a in &args {
        if let Some(rest) = a.strip_prefix("--container=") {
            match Container::parse(rest) {
                Some(c) => explicit = Some(c),
                None => die(EXIT_ARG_ERROR, &format!("unknown container kind: {rest}")),
            }
        } else if a.starts_with("--") {
            die(EXIT_ARG_ERROR, &format!("unknown flag: {a}\n{USAGE}"));
        } else if path.is_none() {
            path = Some(a.clone());
        } else {
            die(EXIT_ARG_ERROR, &format!("unexpected positional: {a}"));
        }
    }
    let path = path.unwrap_or_else(|| die(EXIT_ARG_ERROR, USAGE));

    // Auto-detect when not specified.
    let container = match explicit {
        Some(c) => c,
        None => match auto_detect_container(&path) {
            Ok(c) => c,
            Err(e) => die(EXIT_OPEN_ERROR, &format!("auto-detect: {e}")),
        },
    };

    // Open underlying file.
    let cpath = CString::new(path.as_str()).unwrap();
    // Always open RO for probe — partition probe is read-only and we
    // don't want to risk taking a write lock just to inspect.
    let file = unsafe { fs_core_file_open(cpath.as_ptr(), false) };
    if file.is_null() {
        die(
            EXIT_OPEN_ERROR,
            &format!("fs_core_file_open: {}", last_error()),
        );
    }

    // Stack the container reader (RO since we only probe).
    let dev = unsafe { open_container_on(file, container, false) };
    if dev.is_null() {
        die(
            EXIT_OPEN_ERROR,
            &format!("{}_open_on_device: {}", container.label(), last_error()),
        );
    }

    let dev_size = unsafe { fs_core_device_size_bytes(dev) };

    // Probe partitions.
    let mut list: *mut PartitionList = ptr::null_mut();
    let rc = unsafe { partitions_probe(dev, &mut list) };
    match classify_probe(rc, list.is_null(), &last_error()) {
        ProbeOutcome::Failed(detail) => {
            unsafe { fs_core_device_close(dev) };
            die(EXIT_PROBE_ERROR, &format!("partitions_probe: {detail}"));
        }
        ProbeOutcome::NoTable => {
            // No partition table — a whole-device filesystem. Sniff the
            // device itself and report it as such; this is a normal disk,
            // not a failure, so it exits 0.
            let sniffed = unsafe { partitions_sniff_device(dev, dev_size) };
            let (dev_fs_label, dev_fs_error) = match sniff_outcome(sniffed, last_error) {
                Ok(label) => (label, None),
                Err(detail) => {
                    eprintln!("diskprobe: whole-device filesystem sniff failed: {detail}");
                    (SNIFF_FAILED_LABEL, Some(detail))
                }
            };
            let json = format!(
                "{{\"path\":\"{}\",\"container\":\"{}\",\"container_size_bytes\":{},\"table\":\"none\",\"device_fs_kind\":\"{}\"{},\"partitions\":[]}}",
                json_escape(&path),
                container.label(),
                dev_size,
                dev_fs_label,
                sniff_error_field("device_fs_error", dev_fs_error.as_deref()),
            );
            println!("{json}");
            unsafe { fs_core_device_close(dev) };
            std::process::exit(0);
        }
        ProbeOutcome::Table => {}
    }

    let table_rc = unsafe { partitions_table_kind(list) };
    let count = unsafe { partitions_count(list) };

    let mut entries: Vec<String> = Vec::with_capacity(count);
    for i in 0..count {
        let mut info = PartitionInfo {
            start: 0,
            length: 0,
            fs_kind: FsKindCode::Unknown as i32,
            table_kind: 0,
            type_guid: [0u8; 16],
            type_byte: 0,
            _pad: [0u8; 7],
            label: ptr::null(),
            label_len: 0,
            bootable: 0,
            _pad2: [0u8; 7],
            attributes: 0,
        };
        let grc = unsafe { partitions_get(list, i, &mut info) };
        if grc != FsCoreErrorCode::Ok {
            continue;
        }
        // Sniff the FS — partitions_sniff fills the kind in-place but
        // we already have a copy of info, so capture the returned code.
        let sniffed = unsafe { partitions_sniff(list, i) };
        let (fs_label, fs_error) = match sniff_outcome(sniffed, last_error) {
            Ok(label) => (label, None),
            Err(detail) => {
                eprintln!("diskprobe: partition {i}: filesystem sniff failed: {detail}");
                (SNIFF_FAILED_LABEL, Some(detail))
            }
        };
        let label_str = if !info.label.is_null() && info.label_len > 0 {
            let bytes =
                unsafe { std::slice::from_raw_parts(info.label as *const u8, info.label_len) };
            std::str::from_utf8(bytes).ok().map(|s| s.to_string())
        } else {
            None
        };

        let mut entry = format!(
            "{{\"index\":{},\"start\":{},\"length\":{},\"fs_kind\":\"{}\",\"type_byte\":{},\"type_guid\":\"{}\"",
            i,
            info.start,
            info.length,
            fs_label,
            info.type_byte,
            fmt_guid(&info.type_guid),
        );
        entry.push_str(&sniff_error_field("fs_kind_error", fs_error.as_deref()));
        if let Some(l) = label_str {
            entry.push_str(&format!(",\"label\":\"{}\"", json_escape(&l)));
        }
        entry.push('}');
        entries.push(entry);
    }

    let json = format!(
        "{{\"path\":\"{}\",\"container\":\"{}\",\"container_size_bytes\":{},\"table\":\"{}\",\"partitions\":[{}]}}",
        json_escape(&path),
        container.label(),
        dev_size,
        table_kind_label(table_rc),
        entries.join(","),
    );
    println!("{json}");

    unsafe {
        partitions_list_free(list);
        fs_core_device_close(dev);
    }
}

// Suppress unused-import lint when this binary is built without
// referring to specific types.
#[allow(dead_code)]
fn _unused(_a: c_char) {}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // Test support
    // ---------------------------------------------------------------

    /// A disk image on disk, removed when the test that made it ends.
    ///
    /// `auto_detect_container` takes a path rather than a reader, so the
    /// only way to exercise it is through a real file. Each one is named
    /// after the test that created it so a crashed run leaves an
    /// identifiable corpse rather than a collision.
    struct TempImage(std::path::PathBuf);

    impl TempImage {
        fn new(name: &str, bytes: &[u8]) -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!("diskprobe-test-{}-{name}", std::process::id()));
            std::fs::write(&path, bytes).expect("write temp image");
            TempImage(path)
        }

        fn path(&self) -> &str {
            self.0.to_str().expect("temp path is utf-8")
        }
    }

    impl Drop for TempImage {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// A 1 KiB image with `head` at offset 0 and, optionally, `footer` at
    /// `len - 512` — the two positions `auto_detect_container` reads.
    fn image_with(head: &[u8], footer: Option<&[u8]>) -> Vec<u8> {
        let mut bytes = vec![0u8; 1024];
        bytes[..head.len()].copy_from_slice(head);
        if let Some(footer) = footer {
            let at = bytes.len() - 512;
            bytes[at..at + footer.len()].copy_from_slice(footer);
        }
        bytes
    }

    // ---------------------------------------------------------------
    // Container::label / Container::parse
    // ---------------------------------------------------------------

    #[test]
    fn container_label_round_trips_through_parse() {
        for kind in [
            Container::Raw,
            Container::Qcow2,
            Container::Vhd,
            Container::Vhdx,
            Container::Vmdk,
        ] {
            assert_eq!(
                Container::parse(kind.label()),
                Some(kind),
                "{} did not round-trip",
                kind.label()
            );
        }
    }

    #[test]
    fn container_labels_are_the_strings_the_json_contract_publishes() {
        assert_eq!(Container::Raw.label(), "raw");
        assert_eq!(Container::Qcow2.label(), "qcow2");
        assert_eq!(Container::Vhd.label(), "vhd");
        assert_eq!(Container::Vhdx.label(), "vhdx");
        assert_eq!(Container::Vmdk.label(), "vmdk");
    }

    #[test]
    fn container_parse_rejects_names_it_does_not_know() {
        assert_eq!(Container::parse(""), None);
        assert_eq!(Container::parse("iso"), None);
        assert_eq!(
            Container::parse("QCOW2"),
            None,
            "matching is case-sensitive"
        );
        assert_eq!(Container::parse("qcow"), None);
    }

    // ---------------------------------------------------------------
    // fs_kind_label / table_kind_label
    // ---------------------------------------------------------------

    #[test]
    fn fs_kind_label_names_every_discriminant_it_mirrors() {
        let expected = [
            (FsKindCode::Unknown, "unknown"),
            (FsKindCode::Ext2, "ext2"),
            (FsKindCode::Ext3, "ext3"),
            (FsKindCode::Ext4, "ext4"),
            (FsKindCode::Ntfs, "ntfs"),
            (FsKindCode::ExFat, "exfat"),
            (FsKindCode::Fat32, "fat32"),
            (FsKindCode::Fat16, "fat16"),
            (FsKindCode::HfsPlus, "hfs_plus"),
            (FsKindCode::Apfs, "apfs"),
            (FsKindCode::LinuxSwap, "linux_swap"),
            (FsKindCode::Iso9660, "iso9660"),
            (FsKindCode::Squashfs, "squashfs"),
        ];
        for (code, label) in expected {
            assert_eq!(fs_kind_label(code as i32), label, "for {code:?}");
        }
    }

    #[test]
    fn fs_kind_label_cannot_tell_the_error_sentinel_from_an_unmapped_code() {
        // This is the overlap that made `if sniffed >= 0` inert at the
        // sniff call sites: -1 is the documented error sentinel and it
        // lands on the same catch-all arm as a code this file has never
        // heard of. Nothing here is wrong — but it is why the sentinel
        // has to be caught before the label lookup, not after it.
        assert_eq!(fs_kind_label(-1), "unknown");
        assert_eq!(fs_kind_label(FsKindCode::Unknown as i32), "unknown");
        assert_eq!(fs_kind_label(FsKindCode::Squashfs as i32 + 1), "unknown");
        assert_eq!(fs_kind_label(i32::MIN), "unknown");
        assert_eq!(fs_kind_label(i32::MAX), "unknown");
    }

    #[test]
    fn table_kind_label_names_the_two_tables_and_calls_everything_else_none() {
        assert_eq!(table_kind_label(TableKindCode::Gpt as i32), "gpt");
        assert_eq!(table_kind_label(TableKindCode::Mbr as i32), "mbr");
        assert_eq!(table_kind_label(TableKindCode::None as i32), "none");
        assert_eq!(table_kind_label(-1), "none");
        assert_eq!(table_kind_label(99), "none");
    }

    // ---------------------------------------------------------------
    // json_escape
    // ---------------------------------------------------------------

    #[test]
    fn json_escape_leaves_ordinary_text_untouched() {
        assert_eq!(json_escape(""), "");
        assert_eq!(json_escape("boot"), "boot");
        assert_eq!(
            json_escape("/Volumes/My Disk/image.img"),
            "/Volumes/My Disk/image.img"
        );
    }

    #[test]
    fn json_escape_escapes_the_characters_json_requires() {
        assert_eq!(json_escape("say \"hi\""), "say \\\"hi\\\"");
        assert_eq!(json_escape("C:\\disks"), "C:\\\\disks");
        assert_eq!(json_escape("a\nb"), "a\\nb");
        assert_eq!(json_escape("a\rb"), "a\\rb");
        assert_eq!(json_escape("a\tb"), "a\\tb");
    }

    #[test]
    fn json_escape_renders_other_control_characters_as_u_sequences() {
        assert_eq!(json_escape("\u{0}"), "\\u0000");
        assert_eq!(json_escape("\u{8}"), "\\u0008");
        assert_eq!(json_escape("\u{b}"), "\\u000b");
        assert_eq!(json_escape("\u{1f}"), "\\u001f");
    }

    #[test]
    fn json_escape_passes_through_del_and_non_ascii() {
        // 0x7f and above need no escaping in JSON, and a partition label
        // is free to contain either.
        assert_eq!(json_escape("\u{7f}"), "\u{7f}");
        assert_eq!(json_escape("Système"), "Système");
        assert_eq!(json_escape("ボリューム"), "ボリューム");
    }

    // ---------------------------------------------------------------
    // fmt_guid
    // ---------------------------------------------------------------

    #[test]
    fn fmt_guid_renders_a_known_gpt_type_guid() {
        // The Linux filesystem-data type GUID, as it is written in the
        // GPT spec and as those bytes actually sit on disk.
        let on_disk = [
            0xaf, 0x3d, 0xc6, 0x0f, 0x83, 0x84, 0x72, 0x47, 0x8e, 0x79, 0x3d, 0x69, 0xd8, 0x47,
            0x7d, 0xe4,
        ];
        assert_eq!(fmt_guid(&on_disk), "0fc63daf-8483-4772-8e79-3d69d8477de4");
    }

    #[test]
    fn fmt_guid_byte_swaps_the_first_three_fields_and_not_the_last_two() {
        // Ascending bytes make the mixed-endian layout legible: the first
        // three groups read backwards, the last two read forwards.
        let ascending: [u8; 16] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ];
        assert_eq!(fmt_guid(&ascending), "03020100-0504-0706-0809-0a0b0c0d0e0f");
    }

    #[test]
    fn fmt_guid_zero_pads_every_field() {
        assert_eq!(fmt_guid(&[0u8; 16]), "00000000-0000-0000-0000-000000000000");
        let mut sparse = [0u8; 16];
        sparse[0] = 0x01;
        sparse[15] = 0x02;
        assert_eq!(fmt_guid(&sparse), "00000001-0000-0000-0000-000000000002");
    }

    // ---------------------------------------------------------------
    // auto_detect_container
    // ---------------------------------------------------------------

    #[test]
    fn auto_detect_recognises_each_magic_at_offset_zero() {
        let cases: [(&str, &[u8], Container); 4] = [
            ("qcow2", b"QFI\xfb", Container::Qcow2),
            ("vhdx", b"vhdxfile", Container::Vhdx),
            ("vmdk", b"KDMV", Container::Vmdk),
            ("vhd-header", b"conectix", Container::Vhd),
        ];
        for (name, magic, expected) in cases {
            let image = TempImage::new(name, &image_with(magic, None));
            assert_eq!(
                auto_detect_container(image.path()).unwrap(),
                expected,
                "{name} magic was not recognised"
            );
        }
    }

    #[test]
    fn auto_detect_recognises_a_fixed_vhd_by_its_trailing_footer() {
        // A fixed VHD is a raw image with a 512-byte footer glued on the
        // end; offset 0 is ordinary partition-table data, so the footer
        // is the only thing that distinguishes it.
        let image = TempImage::new("fixed-vhd", &image_with(&[0x00; 4], Some(b"conectix")));
        assert_eq!(auto_detect_container(image.path()).unwrap(), Container::Vhd);
    }

    #[test]
    fn auto_detect_prefers_a_magic_at_offset_zero_over_a_trailing_footer() {
        // The footer check runs last among the signatures on purpose:
        // eight bytes at a position that is payload in every other format
        // is a far weaker signal than an offset-0 magic, so any offset-0
        // magic has to win. This pins that ordering.
        let image = TempImage::new(
            "qcow2-with-footer",
            &image_with(b"QFI\xfb", Some(b"conectix")),
        );
        assert_eq!(
            auto_detect_container(image.path()).unwrap(),
            Container::Qcow2
        );
    }

    #[test]
    fn auto_detect_falls_back_to_raw_when_nothing_matches() {
        let image = TempImage::new("raw", &image_with(&[0x55; 8], None));
        assert_eq!(auto_detect_container(image.path()).unwrap(), Container::Raw);
    }

    #[test]
    fn auto_detect_falls_back_to_raw_for_files_too_short_to_hold_a_signature() {
        let empty = TempImage::new("empty", &[]);
        assert_eq!(auto_detect_container(empty.path()).unwrap(), Container::Raw);

        // Four bytes: shorter than the footer, and shorter than every
        // magic but qcow2's and vmdk's.
        let tiny = TempImage::new("tiny", &[0x00, 0x01, 0x02, 0x03]);
        assert_eq!(auto_detect_container(tiny.path()).unwrap(), Container::Raw);
    }

    // ---------------------------------------------------------------
    // classify_probe — the fork that used to report failure as success
    // ---------------------------------------------------------------

    #[test]
    fn classify_probe_reports_a_table_when_the_probe_succeeded() {
        assert_eq!(
            classify_probe(FsCoreErrorCode::Ok, false, ""),
            ProbeOutcome::Table
        );
    }

    #[test]
    fn classify_probe_ignores_stale_last_error_text_when_the_probe_succeeded() {
        // The last-error thread-local is not cleared on success, so a
        // message left by an earlier call must not be allowed to turn a
        // perfectly good partition table into "no table".
        assert_eq!(
            classify_probe(FsCoreErrorCode::Ok, false, NO_PARTITION_TABLE_MESSAGE),
            ProbeOutcome::Table
        );
    }

    #[test]
    fn classify_probe_calls_a_missing_signature_no_table_rather_than_a_failure() {
        // A whole-device filesystem is a normal disk. This is the one
        // non-Ok return that must still exit 0.
        assert_eq!(
            classify_probe(FsCoreErrorCode::Custom, true, NO_PARTITION_TABLE_MESSAGE),
            ProbeOutcome::NoTable
        );
        assert_eq!(
            classify_probe(
                FsCoreErrorCode::Custom,
                true,
                "  no GPT or MBR signature found\n"
            ),
            ProbeOutcome::NoTable,
            "surrounding whitespace should not change the verdict"
        );
    }

    #[test]
    fn classify_probe_keeps_a_real_failure_distinct_from_an_absent_table() {
        // Every one of these used to end up in the same branch as the
        // case above: "table":"none", empty partitions, exit 0. A caller
        // could not tell a corrupt table from a disk that has none.
        for message in [
            "GPT header CRC32 mismatch",
            "GPT partition-entry array CRC32 mismatch",
            "GPT corrupt: protective MBR present but no GPT signature",
            "MBR corrupt: extended chain loops",
            "read failed at offset 0",
        ] {
            assert_eq!(
                classify_probe(FsCoreErrorCode::Custom, true, message),
                ProbeOutcome::Failed(message.to_string()),
                "{message} should be reported as a failure"
            );
        }
    }

    #[test]
    fn classify_probe_still_reports_a_failure_when_no_detail_was_recorded() {
        let outcome = classify_probe(FsCoreErrorCode::Io, true, "");
        match outcome {
            ProbeOutcome::Failed(detail) => {
                assert!(
                    detail.contains("Io"),
                    "detail should name the code: {detail}"
                );
            }
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn classify_probe_treats_success_with_a_null_list_as_a_broken_promise() {
        // The ABI never does this. If it ever did, the honest answer is
        // "something is wrong", not "this disk has no partitions".
        match classify_probe(FsCoreErrorCode::Ok, true, "") {
            ProbeOutcome::Failed(detail) => assert!(detail.contains("NULL")),
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // sniff_outcome — the sentinel that used to read as "unknown"
    // ---------------------------------------------------------------

    #[test]
    fn sniff_outcome_labels_a_recognised_filesystem() {
        assert_eq!(
            sniff_outcome(FsKindCode::Ext4 as i32, || unreachable!()),
            Ok("ext4")
        );
        assert_eq!(
            sniff_outcome(FsKindCode::Squashfs as i32, || unreachable!()),
            Ok("squashfs")
        );
    }

    #[test]
    fn sniff_outcome_reports_a_successful_no_match_as_unknown() {
        // Code 0 means the sniff ran to completion and matched nothing.
        // The detail closure must not be called: reading the last-error
        // thread-local here would pick up an unrelated stale message.
        let asked = std::cell::Cell::new(false);
        let outcome = sniff_outcome(FsKindCode::Unknown as i32, || {
            asked.set(true);
            String::new()
        });
        assert_eq!(outcome, Ok("unknown"));
        assert!(
            !asked.get(),
            "the error detail was fetched on the success path"
        );
    }

    #[test]
    fn sniff_outcome_separates_the_error_sentinel_from_a_successful_no_match() {
        // The whole point. Both of these used to produce "unknown".
        let failed = sniff_outcome(-1, || "read failed at offset 1048576".to_string());
        assert_eq!(failed, Err("read failed at offset 1048576".to_string()));
        assert_ne!(failed, Ok("unknown"));
        assert_eq!(
            sniff_outcome(FsKindCode::Unknown as i32, String::new),
            Ok("unknown")
        );
    }

    #[test]
    fn sniff_outcome_substitutes_a_reason_when_none_was_recorded() {
        match sniff_outcome(-1, String::new) {
            Err(detail) => assert!(
                detail.contains("-1"),
                "detail should name the code: {detail}"
            ),
            Ok(label) => panic!("expected a failure, got {label}"),
        }
    }

    // ---------------------------------------------------------------
    // sniff_error_field
    // ---------------------------------------------------------------

    #[test]
    fn sniff_error_field_adds_nothing_when_the_sniff_succeeded() {
        assert_eq!(sniff_error_field("fs_kind_error", None), "");
    }

    #[test]
    fn sniff_error_field_emits_an_escaped_json_pair() {
        assert_eq!(
            sniff_error_field("fs_kind_error", Some("read \"failed\"")),
            ",\"fs_kind_error\":\"read \\\"failed\\\"\""
        );
        assert_eq!(
            sniff_error_field("device_fs_error", Some("line one\nline two")),
            ",\"device_fs_error\":\"line one\\nline two\""
        );
    }

    #[test]
    fn auto_detect_reports_the_io_error_when_the_file_cannot_be_opened() {
        let mut missing = std::env::temp_dir();
        missing.push("diskprobe-test-no-such-image");
        let _ = std::fs::remove_file(&missing);
        let err = auto_detect_container(missing.to_str().unwrap()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }
}
