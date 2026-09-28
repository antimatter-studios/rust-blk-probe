//! What `blkid` and `sfdisk` say about the same bytes.
//!
//! # Why this file exists
//!
//! Every other test in this crate hands a buffer to this crate's parser and
//! asserts on what this crate's formatter produced. The proposition "this
//! device carries a GPT with two partitions, the first starting at 17408" was
//! asserted by the code that computed it, and a probe that read a start from
//! the wrong offset would agree with itself all the way through (#13).
//!
//! `util-linux` has no stake in our reading of these formats. `blkid` is the
//! reference identifier for "what is on this device" and `sfdisk --dump`
//! prints the table one field at a time, so both can be compared field by
//! field against the document this crate emits.
//!
//! # Where it runs, and what happens when the tools are absent
//!
//! `blkid` and `sfdisk` are util-linux, which is Linux's. The file is
//! therefore compiled only on Linux -- on macOS there is no equivalent to
//! reach for, and a test that quietly returns because a tool is missing reads
//! exactly like a test that passed.
//!
//! On Linux a missing tool **fails**, naming what to install. It does not
//! skip: both are in the base `util-linux` package, present on every GitHub
//! Linux runner, so "absent" means somebody changed the image and the right
//! answer is to say so loudly.
//!
//! The corpus-recorded half of this check runs everywhere, on a machine with
//! neither tool: `every_committed_image_is_read_as_what_the_tool_that_wrote
//! _it_says` in tests/fuzz_decoders.rs holds the same images to what their
//! makers put in them. This file is what keeps those recorded expectations
//! honest.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/probe")
}

/// Run a tool, or fail the test naming what would provide it.
fn tool(program: &str, args: &[&str]) -> String {
    let output = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "{program} could not be run ({e}). It is the oracle this test compares against, \
             and it ships in util-linux, which every Linux runner has: \
             `sudo apt-get install -y util-linux`. This test does not skip -- a skipped \
             oracle reads exactly like a passing one."
            )
        });
    // blkid exits 2 when it recognises nothing, which is an ANSWER here and
    // not a failure: it is what a container file looks like to it.
    assert!(
        output.status.code().is_some_and(|c| c == 0 || c == 2),
        "{program} {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// `blkid -o export`, as a list of `KEY=value` pairs.
fn blkid(path: &Path) -> Vec<(String, String)> {
    tool(
        "blkid",
        &["-o", "export", path.to_str().expect("utf-8 path")],
    )
    .lines()
    .filter_map(|line| line.split_once('='))
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

fn blkid_value(pairs: &[(String, String)], key: &str) -> Option<String> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.to_string())
}

/// The document this crate produces for an image, as one string.
fn probe(path: &Path) -> String {
    let bytes = std::fs::read(path).expect("reading a corpus image");
    blk_probe::probe_bytes(&bytes, None, path.to_str().expect("utf-8 path"))
        .unwrap_or_else(|e| panic!("{}: would not probe: {e}", path.display()))
        .json
}

/// The value of a flat `"key":value` pair, as text, from `at` onwards.
///
/// Deliberately small: this crate writes the document by `format!`, and a
/// JSON dependency to read four scalars back would be a dependency the crate
/// itself does not have.
fn field(json: &str, at: usize, key: &str) -> String {
    let needle = format!("\"{key}\":");
    let start = json[at..]
        .find(&needle)
        .unwrap_or_else(|| panic!("no {key} in {}", &json[at..]))
        + at
        + needle.len();
    let rest = &json[start..];
    if let Some(quoted) = rest.strip_prefix('"') {
        quoted[..quoted.find('"').expect("closing quote")].to_string()
    } else {
        rest[..rest.find([',', '}']).expect("end of a numeric field")].to_string()
    }
}

/// The offset of each partition object in the document, in order.
fn partition_offsets(json: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(found) = json[at..].find("{\"index\":") {
        out.push(at + found);
        at += found + 1;
    }
    out
}

/// What `blkid` reports for each committed image, and what this crate must
/// say about the same file.
///
/// The container rows are the interesting ones: `blkid` reads a qcow2 or a
/// VMDK as an unrecognised blob, because it does not unwrap containers. This
/// crate does, and finds the same GPT `blkid` finds in the raw original --
/// which is the whole reason it exists, stated as a test rather than as a
/// paragraph.
#[test]
fn blkid_and_this_probe_agree_on_every_committed_image() {
    let cases: &[(&str, Option<&str>, Option<&str>)] = &[
        // image            blkid PTTYPE   blkid TYPE
        ("gpt.img", Some("gpt"), None),
        ("mbr.img", Some("dos"), None),
        // A fixed VHD is a raw image with a 512-byte footer glued on, so
        // blkid sees the table at the front. This crate names the container
        // as well, and must agree about the table.
        ("gpt-fixed.vhd", Some("gpt"), None),
        ("ext4.img", None, Some("ext4")),
        ("squashfs.img", None, Some("squashfs")),
        // Containers: nothing for blkid, a whole disk for this crate.
        ("gpt.qcow2", None, None),
        ("gpt.vmdk", None, None),
    ];

    for (name, want_pttype, want_type) in cases {
        let path = corpus().join(name);
        let pairs = blkid(&path);
        assert_eq!(
            blkid_value(&pairs, "PTTYPE").as_deref(),
            *want_pttype,
            "{name}: blkid no longer reports the partition table this test was written \
             against; the expectation, not the assertion, is what is stale",
        );
        assert_eq!(
            blkid_value(&pairs, "TYPE").as_deref(),
            *want_type,
            "{name}: blkid no longer reports the filesystem this test was written against",
        );

        let json = probe(&path);

        // blkid's `dos` is this crate's `mbr`; the other spelling is the
        // same word. Where blkid sees no table at all, this crate must
        // either see none either (a raw filesystem) or have unwrapped a
        // container to find one.
        let table = field(&json, 0, "table");
        match want_pttype {
            Some("gpt") => assert_eq!(table, "gpt", "{name}: blkid says gpt, this says {table}"),
            Some("dos") => assert_eq!(table, "mbr", "{name}: blkid says dos, this says {table}"),
            Some(other) => panic!("{name}: no rule for a blkid PTTYPE of {other}"),
            None => {}
        }

        if let Some(fs) = want_type {
            assert_eq!(
                table, "none",
                "{name}: blkid found a whole-device {fs} and this crate found a table",
            );
            assert_eq!(
                field(&json, 0, "device_fs_kind"),
                *fs,
                "{name}: blkid calls this {fs}",
            );
        }
    }
}

/// A container is where the two tools part company, and this is the claim
/// that makes this crate worth running at all.
#[test]
fn the_containers_blkid_cannot_read_are_read_here() {
    for name in ["gpt.qcow2", "gpt.vmdk"] {
        let path = corpus().join(name);
        let pairs = blkid(&path);
        assert!(
            blkid_value(&pairs, "PTTYPE").is_none() && blkid_value(&pairs, "TYPE").is_none(),
            "{name}: blkid now reads this container, so this test's premise has changed: {pairs:?}",
        );
        let json = probe(&path);
        assert_eq!(
            field(&json, 0, "table"),
            "gpt",
            "{name}: blkid cannot read it and neither can we, which is the point of the crate",
        );
    }
}

/// `sfdisk --dump`, as one `(start, size, type, name)` per partition.
///
/// The dump is line-oriented and one field per `key=value`, which is why it
/// is read rather than `--json`: a JSON parser to read four scalars would be
/// a dependency this crate does not otherwise have.
fn sfdisk_partitions(path: &Path) -> Vec<(u64, u64, String, Option<String>)> {
    let dump = tool("sfdisk", &["--dump", path.to_str().expect("utf-8 path")]);
    let sector: u64 = dump
        .lines()
        .find_map(|l| l.strip_prefix("sector-size:"))
        .map(|v| v.trim().parse().expect("a sector size"))
        .expect("sfdisk reports a sector size");
    dump.lines()
        .filter(|l| l.contains(" : start="))
        .map(|line| {
            let mut start = 0;
            let mut size = 0;
            let mut kind = String::new();
            let mut name = None;
            let (_, fields) = line.split_once(" : ").expect("a partition line");
            for field in fields.split(',') {
                let (key, value) = field.split_once('=').expect("a key=value field");
                let value = value.trim();
                match key.trim() {
                    "start" => start = value.parse::<u64>().expect("a start") * sector,
                    "size" => size = value.parse::<u64>().expect("a size") * sector,
                    "type" => kind = value.to_string(),
                    "name" => name = Some(value.trim_matches('"').to_string()),
                    _ => {}
                }
            }
            (start, size, kind, name)
        })
        .collect()
}

/// Every partition this crate reports is the one `sfdisk` reports, field by
/// field.
///
/// This is the check that a start read from the wrong offset, a length taken
/// from the wrong field, or a GUID byte-swapped the wrong way cannot survive:
/// none of these numbers came from this crate.
#[test]
fn sfdisk_and_this_probe_agree_on_every_partition() {
    for name in ["gpt.img", "mbr.img", "gpt-fixed.vhd"] {
        let path = corpus().join(name);
        let expected = sfdisk_partitions(&path);
        assert!(
            !expected.is_empty(),
            "{name}: sfdisk reported no partitions, so this comparison checks nothing",
        );

        let json = probe(&path);
        let offsets = partition_offsets(&json);
        assert_eq!(
            offsets.len(),
            expected.len(),
            "{name}: sfdisk found {} partitions and this crate found {}",
            expected.len(),
            offsets.len(),
        );

        for (i, (at, (start, size, kind, label))) in offsets.iter().zip(expected.iter()).enumerate()
        {
            assert_eq!(
                field(&json, *at, "start"),
                start.to_string(),
                "{name}: partition {i} starts where sfdisk does not",
            );
            assert_eq!(
                field(&json, *at, "length"),
                size.to_string(),
                "{name}: partition {i} is not the length sfdisk reports",
            );
            if kind.contains('-') {
                // GPT: sfdisk prints the type GUID in upper case.
                assert_eq!(
                    field(&json, *at, "type_guid"),
                    kind.to_lowercase(),
                    "{name}: partition {i} has a different type GUID than sfdisk reports",
                );
            } else {
                // MBR: sfdisk prints the type byte in hex, unprefixed.
                let byte = u8::from_str_radix(kind, 16).expect("an MBR type byte");
                assert_eq!(
                    field(&json, *at, "type_byte"),
                    byte.to_string(),
                    "{name}: partition {i} has a different type byte than sfdisk reports",
                );
            }
            if let Some(label) = label {
                assert_eq!(
                    field(&json, *at, "label"),
                    *label,
                    "{name}: partition {i} has a different name than sfdisk reports",
                );
            }
        }
    }
}
