//! The description document must match the oracle byte for byte.
//!
//! The conformance workbench builds each tree below through real liblsl and
//! writes the document that `lsl_get_xml` returns. The files live in
//! `tests/desc/`. This test builds the same tree here and compares.
//!
//! The fields that carry a clock reading, a random identifier, or a port
//! change on every run. The capture tool replaces them with `PINNED`, and this
//! test applies the same replacement, so the comparison covers everything that
//! an implementation controls.

use labstream_net::desc::{self, Node};
use labstream_net::StreamInfo;
use labstream_wire::Format;

const PINNED: [&str; 9] = [
    "created_at",
    "uid",
    "hostname",
    "v4address",
    "v4data_port",
    "v4service_port",
    "v6address",
    "v6data_port",
    "v6service_port",
];

/// Replace the value of every field that changes between runs.
fn pin(mut xml: String) -> String {
    for f in PINNED {
        let open = format!("<{f}>");
        let close = format!("</{f}>");
        if let Some(a) = xml.find(&open) {
            if let Some(b) = xml[a..].find(&close).map(|k| k + a) {
                xml.replace_range(a + open.len()..b, "PINNED");
            }
        }
    }
    xml
}

fn base(channels: u32, format: Format, srate: f64) -> StreamInfo {
    let mut i = StreamInfo::new("Desc", "Interop", channels, format, srate);
    i.source_id = "desc_src".into();
    // A stream_info that no outlet published carries an empty session id.
    i.session_id = String::new();
    i
}

/// Put a readable value back where the capture tool wrote `PINNED`.
///
/// A port and a clock reading have to be numbers, and liblsl rejects a
/// description whose port is not one. The uid stays as it is, because any
/// text that is not empty passes.
fn unpin(mut xml: String) -> String {
    for (f, v) in [
        ("created_at", "1.000000000000000"),
        ("v4data_port", "16572"),
        ("v4service_port", "16572"),
        ("v6data_port", "0"),
        ("v6service_port", "0"),
    ] {
        xml = xml.replace(&format!("<{f}>PINNED</{f}>"), &format!("<{f}>{v}</{f}>"));
    }
    xml
}

fn golden(name: &str) -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/desc/");
    std::fs::read_to_string(format!("{path}{name}.xml"))
        .unwrap_or_else(|e| panic!("cannot read the capture {name}: {e}"))
}

fn same(name: &str, info: &StreamInfo) {
    let want = golden(name);
    let have = pin(info.to_fullinfo_xml());
    if want != have {
        for (k, (a, b)) in want.lines().zip(have.lines()).enumerate() {
            if a != b {
                panic!(
                    "{name} differs at line {}\n  oracle {a:?}\n  ours   {b:?}",
                    k + 1
                );
            }
        }
        panic!(
            "{name} differs in length: the oracle has {} lines, ours has {}",
            want.lines().count(),
            have.lines().count()
        );
    }
}

#[test]
fn an_empty_tree_matches_the_oracle() {
    same("desc_empty", &base(2, Format::Float32, 100.0));
}

#[test]
fn one_level_of_text_matches_the_oracle() {
    let mut i = base(2, Format::Float32, 100.0);
    i.desc.append_child_value("manufacturer", "Acme");
    i.desc.append_child_value("serial", "0042");
    same("desc_flat", &i);
}

#[test]
fn a_channel_list_matches_the_oracle() {
    let mut i = base(3, Format::Float32, 100.0);
    {
        let chns = i.desc.append_child("channels");
        for label in ["C3", "C4", "Cz"] {
            let c = chns.append_child("channel");
            c.append_child_value("label", label);
            c.append_child_value("unit", "microvolts");
            c.append_child_value("type", "EEG");
        }
    }
    i.desc.append_child_value("manufacturer", "Acme");
    same("desc_channels", &i);
}

#[test]
fn an_empty_value_and_an_empty_branch_match_the_oracle() {
    let mut i = base(1, Format::String, 0.0);
    i.desc.append_child_value("blank", "");
    i.desc.append_child("hollow");
    i.desc.append_child("outer").append_child("inner");
    same("desc_blank", &i);
}

#[test]
fn the_escapes_match_the_oracle() {
    let mut i = base(1, Format::Int8, 0.0);
    i.desc.append_child_value("amp", "a & b");
    i.desc.append_child_value("angles", "<tag> & </tag>");
    i.desc
        .append_child_value("quotes", "he said \"hi\" and 'bye'");
    i.desc.append_child_value("lines", "one\ntwo\rthree\tfour");
    i.desc.append_child_value("utf8", "\u{b5}V \u{2206}");
    same("desc_escapes", &i);
}

#[test]
fn deep_nesting_matches_the_oracle() {
    let mut i = base(1, Format::Int8, 0.0);
    let mut n = &mut i.desc;
    for k in 0..6 {
        n = n.append_child(&format!("level{k}"));
    }
    n.append_child_value("leaf", "bottom");
    same("desc_deep", &i);
}

#[test]
fn repeated_names_match_the_oracle() {
    let mut i = base(1, Format::Int8, 0.0);
    i.desc.append_child_value("item", "one");
    i.desc.append_child_value("item", "two");
    i.desc.append_child_value("dotted.name_2-x", "value");
    same("desc_repeat", &i);
}

// ===========================================================================
// the reader
// ===========================================================================

#[test]
fn the_reader_accepts_every_capture() {
    for name in [
        "desc_empty",
        "desc_flat",
        "desc_channels",
        "desc_blank",
        "desc_escapes",
        "desc_deep",
        "desc_repeat",
    ] {
        // The capture holds `PINNED` where a uid belongs, which is not empty,
        // so the reader accepts it.
        let info = StreamInfo::from_fullinfo_xml(&unpin(golden(name)))
            .unwrap_or_else(|e| panic!("{name} was rejected: {e}"));
        assert_eq!(info.name, "Desc");
        assert_eq!(info.source_id, "desc_src");
    }
}

#[test]
fn a_document_with_an_empty_uid_is_rejected_with_the_text_that_liblsl_throws() {
    // `tests/desc/desc_nouid.name` holds
    // `(invalid: The UID of the given stream info is empty.)`.
    let want = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/desc/desc_nouid.name"
    ))
    .expect("the capture");
    let mut i = base(2, Format::Float32, 100.0);
    i.uid = String::new();
    let err = StreamInfo::from_fullinfo_xml(&i.to_fullinfo_xml()).expect_err("a rejection");
    assert_eq!(format!("(invalid: {err})"), want.trim());
}

#[test]
fn a_round_trip_keeps_the_tree() {
    // `tests/desc/desc_roundtrip.xml` shows that the tree survives.
    let text = unpin(golden("desc_roundtrip"));
    let info = StreamInfo::from_fullinfo_xml(&text).expect("a description");
    let label = info
        .desc
        .child("channels")
        .and_then(|c| c.child("channel"))
        .map(|c| c.child_value_of("label"))
        .unwrap_or("");
    assert_eq!(label, "C3");
    assert_eq!(info.channel_count, 2);
    assert_eq!(info.format, Format::Float32);
}

#[test]
fn a_document_that_came_from_a_reader_writes_an_empty_field_as_one_tag() {
    // `tests/desc/desc_roundtrip.xml` is what the oracle writes after it
    // reads a document. Every empty field there is `<x />`, because the reader
    // drops a text node that holds nothing. A description that a program built
    // writes `<x></x>` for the same field.
    let text = unpin(golden("desc_roundtrip"));
    let info = StreamInfo::from_fullinfo_xml(&text).expect("a description");
    let back = pin(info.to_fullinfo_xml());
    let want = golden("desc_roundtrip");
    for (k, (a, b)) in want.lines().zip(back.lines()).enumerate() {
        assert_eq!(a, b, "line {} differs after a read and a write", k + 1);
    }
    assert_eq!(want.lines().count(), back.lines().count());
}

#[test]
fn a_description_that_a_program_built_writes_an_empty_field_as_a_pair_of_tags() {
    let i = base(2, Format::Float32, 100.0);
    let xml = i.to_fullinfo_xml();
    assert!(xml.contains("<v4address></v4address>"), "got {xml}");
    assert!(xml.contains("<session_id></session_id>"), "got {xml}");
}

#[test]
fn a_reparse_loses_a_carriage_return_the_way_the_oracle_loses_one() {
    // `tests/desc/desc_reparse.hex` holds the exact bytes.
    let hex = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/desc/desc_reparse.hex"
    ))
    .expect("the capture");

    let mut i = base(1, Format::Int8, 0.0);
    i.uid = "abc123".into();
    i.desc.append_child_value("amp", "a & b");
    i.desc.append_child_value("angles", "<tag> & </tag>");
    i.desc
        .append_child_value("quotes", "he said \"hi\" and 'bye'");
    i.desc.append_child_value("lines", "one\ntwo\rthree\tfour");
    i.desc.append_child_value("utf8", "\u{b5}V \u{2206}");
    let back = StreamInfo::from_fullinfo_xml(&i.to_fullinfo_xml()).expect("a description");

    for line in hex.lines().filter(|l| !l.trim().is_empty()) {
        let (key, want) = line.split_once(' ').expect("a key and a value");
        let have: String = back
            .desc
            .child_value_of(key)
            .bytes()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(have, want, "the value of <{key}> differs");
    }
}

#[test]
fn a_tree_survives_a_write_and_a_read() {
    let mut root = Node::element("desc");
    root.append_child_value("a", "1");
    root.append_child("b").append_child_value("c", "2");
    let mut text = String::new();
    root.write(&mut text, 0);
    let back = desc::parse(&text).expect("a tree");
    assert_eq!(back, root);
}
