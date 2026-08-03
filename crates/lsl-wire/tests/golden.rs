//! Golden vector tests for the sample codec.
//!
//! Every vector holds bytes that a real liblsl outlet wrote, and a record of
//! the values that the producer pushed. `oracle/makevectors.py` builds them.
//!
//! Each vector runs four checks. The four together close a hole that any one
//! of them leaves open.
//!
//! 1. **Sample count.** The stream holds two test pattern samples and then one
//!    sample for each pushed record.
//! 2. **Test pattern values.** Sample 0 and sample 1 must equal patterns that
//!    this crate builds from the specification. This checks the decoder against
//!    a value that no byte comparison produces.
//! 3. **Pushed values.** Every later sample must equal the record from the
//!    producer. The record never passed through this codec.
//! 4. **Re-encode.** The encoded bytes must equal the oracle bytes exactly.
//!
//! Check 4 alone passes when a decoder and an encoder hold the same error.
//! Check 2 and check 3 alone pass when an encoder is wrong. The set catches
//! both.

use lsl_wire::{ByteOrder, Codec, Format, Sample, Value, TEST_PATTERN_OFFSETS};
use serde_json::Value as J;
use std::path::{Path, PathBuf};

fn vector_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors")
}

fn load_all() -> Vec<(String, J)> {
    let dir = vector_dir();
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    let mut paths: Vec<_> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    for p in paths {
        let text = std::fs::read_to_string(&p).expect("read a vector file");
        let j: J = serde_json::from_str(&text).expect("parse a vector file");
        out.push((p.file_stem().unwrap().to_string_lossy().into_owned(), j));
    }
    out
}

fn format_of(name: &str) -> Format {
    match name {
        "float32" => Format::Float32,
        "double64" => Format::Double64,
        "string" => Format::String,
        "int32" => Format::Int32,
        "int16" => Format::Int16,
        "int8" => Format::Int8,
        "int64" => Format::Int64,
        other => panic!("unknown format {other}"),
    }
}

fn hex_to_bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex byte"))
        .collect()
}

fn bits_from_hex(s: &str) -> u64 {
    let t = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(t, 16).expect("hex bits")
}

/// Build the value that the producer recorded.
///
/// A float arrives as a bit pattern, so no text formatting takes place. An
/// integer arrives as a decimal string. A string arrives as hex bytes.
fn value_from_record(format: Format, raw: &str) -> Value {
    match format {
        Format::Float32 => Value::F32(f32::from_bits(bits_from_hex(raw) as u32)),
        Format::Double64 => Value::F64(f64::from_bits(bits_from_hex(raw))),
        Format::Int32 => Value::I32(raw.parse().expect("int32")),
        Format::Int16 => Value::I16(raw.parse().expect("int16")),
        Format::Int8 => Value::I8(raw.parse().expect("int8")),
        Format::Int64 => Value::I64(raw.parse().expect("int64")),
        Format::String => Value::Str(hex_to_bytes(raw)),
        Format::Undefined => panic!("undefined format"),
    }
}

/// Compare two values by bit pattern.
///
/// A direct comparison of floats fails for a value that is not a number, and
/// it wrongly passes for a negative zero against a positive zero.
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::F32(x), Value::F32(y)) => x.to_bits() == y.to_bits(),
        (Value::F64(x), Value::F64(y)) => x.to_bits() == y.to_bits(),
        _ => a == b,
    }
}

fn same_sample(a: &Sample, b: &Sample) -> bool {
    a.timestamp.to_bits() == b.timestamp.to_bits()
        && a.values.len() == b.values.len()
        && a.values
            .iter()
            .zip(&b.values)
            .all(|(x, y)| same_value(x, y))
}

struct Loaded {
    name: String,
    codec: Codec,
    bytes: Vec<u8>,
    pushed: Vec<Sample>,
}

fn parse(name: &str, j: &J) -> Loaded {
    let format = format_of(j["format"].as_str().expect("format"));
    let channels = j["channels"].as_u64().expect("channels") as usize;
    let order = match j["byte_order"].as_i64().expect("byte_order") {
        1234 => ByteOrder::Little,
        4321 => ByteOrder::Big,
        other => panic!("unknown byte order {other}"),
    };
    let suppress = j["suppress_subnormals"].as_bool().unwrap_or(false);
    let codec = Codec::new(format, channels, order).with_suppress_subnormals(suppress);

    let bytes = hex_to_bytes(j["bytes"].as_str().expect("bytes"));

    let pushed = j["pushed"]
        .as_array()
        .expect("pushed")
        .iter()
        .map(|s| {
            let t = f64::from_bits(bits_from_hex(s["t_bits"].as_str().expect("t_bits")));
            let values = s["values"]
                .as_array()
                .expect("values")
                .iter()
                .map(|v| value_from_record(format, v.as_str().expect("value")))
                .collect();
            Sample {
                timestamp: t,
                values,
            }
        })
        .collect();

    Loaded {
        name: name.to_string(),
        codec,
        bytes,
        pushed,
    }
}

#[test]
fn vectors_exist() {
    let all = load_all();
    assert!(
        !all.is_empty(),
        "no vectors found in {}. Run oracle/makevectors.py first.",
        vector_dir().display()
    );
    println!("loaded {} vector files", all.len());
}

#[test]
fn every_vector_decodes_to_the_expected_samples() {
    let all = load_all();
    if all.is_empty() {
        eprintln!("no vectors; skipping");
        return;
    }
    let mut checked_samples = 0usize;

    for (name, j) in &all {
        let v = parse(name, j);
        let (got, used) = v
            .codec
            .decode_all(&v.bytes)
            .unwrap_or_else(|e| panic!("{}: decode failed: {e}", v.name));

        assert_eq!(
            used,
            v.bytes.len(),
            "{}: bytes left over after decode",
            v.name
        );

        // Check 1: the count.
        let want = 2 + v.pushed.len();
        assert_eq!(got.len(), want, "{}: wrong sample count", v.name);

        // Check 2: the two test pattern samples.
        for (i, off) in TEST_PATTERN_OFFSETS.iter().enumerate() {
            let expect = lsl_wire::test_pattern(v.codec.format, v.codec.channels, *off);
            assert!(
                same_sample(&got[i], &expect),
                "{}: test pattern {i} differs\n  expected {:?}\n  got      {:?}",
                v.name,
                expect,
                got[i]
            );
        }

        // Check 3: the pushed values, from the producer record.
        for (i, want_sample) in v.pushed.iter().enumerate() {
            let g = &got[2 + i];
            assert!(
                same_sample(g, want_sample),
                "{}: pushed sample {i} differs\n  expected {:?}\n  got      {:?}",
                v.name,
                want_sample,
                g
            );
        }

        checked_samples += got.len();
    }
    println!(
        "checked {checked_samples} samples across {} vectors",
        all.len()
    );
}

#[test]
fn every_vector_re_encodes_to_the_same_bytes() {
    let all = load_all();
    if all.is_empty() {
        eprintln!("no vectors; skipping");
        return;
    }
    let mut total = 0usize;

    for (name, j) in &all {
        let v = parse(name, j);
        let (got, _) = v.codec.decode_all(&v.bytes).expect("decode");

        let mut out = Vec::with_capacity(v.bytes.len());
        for s in &got {
            v.codec.encode(s, &mut out).expect("encode");
        }

        if out != v.bytes {
            let at = out
                .iter()
                .zip(&v.bytes)
                .position(|(a, b)| a != b)
                .unwrap_or(out.len().min(v.bytes.len()));
            panic!(
                "{}: re-encoded bytes differ at offset {at}\n  oracle {:02x?}\n  ours   {:02x?}",
                v.name,
                &v.bytes[at.saturating_sub(4)..(at + 8).min(v.bytes.len())],
                &out[at.saturating_sub(4)..(at + 8).min(out.len())]
            );
        }
        total += v.bytes.len();
    }
    println!("re-encoded {total} bytes across {} vectors", all.len());
}

#[test]
fn the_oracle_swapped_bytes_for_the_big_endian_vectors() {
    // The generator asks the oracle to swap. If the oracle never swapped, the
    // big-endian vectors carry no information beyond the little-endian ones.
    let all = load_all();
    if all.is_empty() {
        return;
    }
    let swapped: Vec<_> = all
        .iter()
        .filter(|(_, j)| j["byte_order"].as_i64() == Some(4321))
        .collect();
    assert!(
        !swapped.is_empty(),
        "no vector carries a swapped byte order. The negotiation did not work, \
         so the big-endian path is untested."
    );
    // A one-byte format never swaps, so it is not in this group.
    for (name, j) in &swapped {
        assert_ne!(
            j["format"].as_str(),
            Some("int8"),
            "{name}: a one-byte format must never swap"
        );
    }
    println!("{} vectors carry oracle-swapped bytes", swapped.len());
}
