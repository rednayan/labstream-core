//! Test the tests.
//!
//! A vector set that passes everything proves nothing. This file holds a list
//! of deliberate encoder defects. Each one must break at least one vector.
//!
//! A defect that no vector catches means the vector set has a hole. The test
//! then names the defect, and the fix is a new vector and not a new assertion.
//!
//! Every mutant writes bytes from samples that the real codec decoded from
//! oracle bytes. A mutant that still matches the oracle bytes went unnoticed.

use lsl_wire::{ByteOrder, Codec, Format, Sample, Value};
use serde_json::Value as J;
use std::path::Path;

// ---------------------------------------------------------------------------
// loading
// ---------------------------------------------------------------------------

struct Vector {
    name: String,
    codec: Codec,
    bytes: Vec<u8>,
    samples: Vec<Sample>,
}

fn hex_to_bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn load() -> Vec<Vector> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors");
    let mut paths: Vec<_> = match std::fs::read_dir(&dir) {
        Ok(e) => e
            .filter_map(|x| x.ok())
            .map(|x| x.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect(),
        Err(_) => return Vec::new(),
    };
    paths.sort();

    let mut out = Vec::new();
    for p in paths {
        let j: J = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let format = match j["format"].as_str().unwrap() {
            "float32" => Format::Float32,
            "double64" => Format::Double64,
            "string" => Format::String,
            "int32" => Format::Int32,
            "int16" => Format::Int16,
            "int8" => Format::Int8,
            "int64" => Format::Int64,
            o => panic!("unknown format {o}"),
        };
        let channels = j["channels"].as_u64().unwrap() as usize;
        let order = match j["byte_order"].as_i64().unwrap() {
            1234 => ByteOrder::Little,
            4321 => ByteOrder::Big,
            o => panic!("unknown order {o}"),
        };
        let codec = Codec::new(format, channels, order)
            .with_suppress_subnormals(j["suppress_subnormals"].as_bool().unwrap_or(false));
        let bytes = hex_to_bytes(j["bytes"].as_str().unwrap());
        let samples = match codec.decode_all(&bytes) {
            Ok((s, _)) => s,
            Err(e) => panic!("the real codec cannot decode {p:?}: {e}"),
        };
        out.push(Vector {
            name: p.file_stem().unwrap().to_string_lossy().into_owned(),
            codec,
            bytes,
            samples,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// the mutants
// ---------------------------------------------------------------------------

/// Whether a golden vector can catch a defect at all.
#[derive(PartialEq)]
enum Reach {
    /// The oracle can produce a stream that shows the defect.
    Oracle,
    /// The oracle cannot produce such a stream. The reason names the rule that
    /// blocks it. A direct property test covers the defect instead.
    Blocked(&'static str),
}

/// A deliberate defect. It writes bytes the way a wrong implementation does.
struct Mutant {
    name: &'static str,
    why: &'static str,
    reach: Reach,
    write: fn(&Codec, &Sample, &mut Vec<u8>),
}

fn native_swap(c: &Codec) -> bool {
    c.order != ByteOrder::native()
}

fn put_ts(out: &mut Vec<u8>, t: f64, swap: bool) {
    let b = if swap {
        t.to_bits().to_be_bytes()
    } else {
        t.to_bits().to_le_bytes()
    };
    out.extend_from_slice(&b);
}

macro_rules! push_num {
    ($out:expr, $v:expr, $swap:expr) => {{
        let b = if $swap {
            $v.to_be_bytes()
        } else {
            $v.to_le_bytes()
        };
        $out.extend_from_slice(&b);
    }};
}

/// Write the channel values the correct way. Mutants reuse this.
fn correct_values(_c: &Codec, s: &Sample, out: &mut Vec<u8>, swap: bool) {
    for v in &s.values {
        match v {
            Value::F32(x) => push_num!(out, x.to_bits(), swap),
            Value::F64(x) => push_num!(out, x.to_bits(), swap),
            Value::I32(x) => push_num!(out, x, swap),
            Value::I16(x) => push_num!(out, x, swap),
            Value::I8(x) => out.push(*x as u8),
            Value::I64(x) => push_num!(out, x, swap),
            Value::Str(b) => {
                if b.len() <= 0xFF {
                    out.push(1);
                    out.push(b.len() as u8);
                } else if b.len() <= 0xFFFF_FFFF {
                    out.push(4);
                    push_num!(out, (b.len() as u32), swap);
                } else {
                    out.push(8);
                    push_num!(out, (b.len() as u64), swap);
                }
                out.extend_from_slice(b);
            }
        }
    }
}

fn correct_header(c: &Codec, s: &Sample, out: &mut Vec<u8>) {
    if s.timestamp == lsl_wire::DEDUCED_TIMESTAMP {
        out.push(1);
    } else {
        out.push(2);
        put_ts(out, s.timestamp, native_swap(c));
    }
}

const MUTANTS: &[Mutant] = &[
    Mutant {
        name: "tags_swapped",
        reach: Reach::Oracle,
        why: "writes tag 2 for a deduced timestamp and tag 1 for a transmitted one",
        write: |c, s, out| {
            if s.timestamp == lsl_wire::DEDUCED_TIMESTAMP {
                out.push(2);
                put_ts(out, s.timestamp, native_swap(c));
            } else {
                out.push(1);
            }
            correct_values(c, s, out, c.order != ByteOrder::native());
        },
    },
    Mutant {
        name: "always_writes_timestamp",
        reach: Reach::Oracle,
        why: "writes an f64 even for a deduced timestamp, so every sample grows by 8 bytes",
        write: |c, s, out| {
            out.push(if s.timestamp == lsl_wire::DEDUCED_TIMESTAMP {
                1
            } else {
                2
            });
            put_ts(out, s.timestamp, native_swap(c));
            correct_values(c, s, out, c.order != ByteOrder::native());
        },
    },
    Mutant {
        name: "timestamp_never_swaps",
        reach: Reach::Oracle,
        why: "keeps the timestamp in native order when the connection swaps",
        write: |c, s, out| {
            if s.timestamp == lsl_wire::DEDUCED_TIMESTAMP {
                out.push(1);
            } else {
                out.push(2);
                put_ts(out, s.timestamp, false);
            }
            correct_values(c, s, out, c.order != ByteOrder::native());
        },
    },
    Mutant {
        name: "values_never_swap",
        reach: Reach::Oracle,
        why: "keeps channel values in native order when the connection swaps",
        write: |c, s, out| {
            correct_header(c, s, out);
            correct_values(c, s, out, false);
        },
    },
    Mutant {
        name: "values_always_swap",
        reach: Reach::Oracle,
        why: "swaps channel values even when the connection uses the native order",
        write: |c, s, out| {
            correct_header(c, s, out);
            correct_values(c, s, out, true);
        },
    },
    Mutant {
        name: "channels_reversed",
        reach: Reach::Oracle,
        why: "writes the channels back to front",
        write: |c, s, out| {
            correct_header(c, s, out);
            let flipped = Sample {
                timestamp: s.timestamp,
                values: s.values.iter().rev().cloned().collect(),
            };
            correct_values(c, &flipped, out, c.order != ByteOrder::native());
        },
    },
    Mutant {
        name: "string_len_always_u32",
        reach: Reach::Oracle,
        why: "writes the width 4 for every string, including a short one",
        write: |c, s, out| {
            correct_header(c, s, out);
            let swap = c.order != ByteOrder::native();
            for v in &s.values {
                match v {
                    Value::Str(b) => {
                        out.push(4);
                        push_num!(out, (b.len() as u32), swap);
                        out.extend_from_slice(b);
                    }
                    other => correct_values(
                        c,
                        &Sample {
                            timestamp: 0.0,
                            values: vec![other.clone()],
                        },
                        out,
                        swap,
                    ),
                }
            }
        },
    },
    Mutant {
        name: "string_len_u16_when_it_fits",
        reach: Reach::Oracle,
        why: "picks the width 2 for a length of 256 to 65535, which reads correctly and \
              which no liblsl outlet ever writes",
        write: |c, s, out| {
            correct_header(c, s, out);
            let swap = c.order != ByteOrder::native();
            for v in &s.values {
                match v {
                    Value::Str(b) => {
                        if b.len() <= 0xFF {
                            out.push(1);
                            out.push(b.len() as u8);
                        } else if b.len() <= 0xFFFF {
                            out.push(2);
                            push_num!(out, (b.len() as u16), swap);
                        } else {
                            out.push(4);
                            push_num!(out, (b.len() as u32), swap);
                        }
                        out.extend_from_slice(b);
                    }
                    other => correct_values(
                        c,
                        &Sample {
                            timestamp: 0.0,
                            values: vec![other.clone()],
                        },
                        out,
                        swap,
                    ),
                }
            }
        },
    },
    Mutant {
        name: "string_width_byte_swapped",
        reach: Reach::Oracle,
        why: "swaps the width byte, which must never swap",
        write: |c, s, out| {
            correct_header(c, s, out);
            let swap = c.order != ByteOrder::native();
            for v in &s.values {
                match v {
                    Value::Str(b) => {
                        // A wrong implementation treats the width as part of
                        // the swapped length field.
                        if b.len() <= 0xFF {
                            out.push(1);
                            out.push(b.len() as u8);
                        } else {
                            out.push(4);
                            push_num!(out, (b.len() as u32), !swap);
                        }
                        out.extend_from_slice(b);
                    }
                    other => correct_values(
                        c,
                        &Sample {
                            timestamp: 0.0,
                            values: vec![other.clone()],
                        },
                        out,
                        swap,
                    ),
                }
            }
        },
    },
    Mutant {
        name: "string_len_off_by_one",
        reach: Reach::Oracle,
        why: "writes a length one byte short",
        write: |c, s, out| {
            correct_header(c, s, out);
            let swap = c.order != ByteOrder::native();
            for v in &s.values {
                match v {
                    Value::Str(b) => {
                        let n = b.len().saturating_sub(1);
                        if n <= 0xFF {
                            out.push(1);
                            out.push(n as u8);
                        } else {
                            out.push(4);
                            push_num!(out, (n as u32), swap);
                        }
                        out.extend_from_slice(b);
                    }
                    other => correct_values(
                        c,
                        &Sample {
                            timestamp: 0.0,
                            values: vec![other.clone()],
                        },
                        out,
                        swap,
                    ),
                }
            }
        },
    },
    Mutant {
        name: "int8_swapped_with_the_connection",
        reach: Reach::Blocked(
            "the oracle never swaps a one-byte format. Negotiation condition 3 needs a \
             value size above 1 (SPEC.md 5.2, src/tcp_server.cpp:659), so no capture can \
             hold a swapped one-byte stream.",
        ),
        why: "flips the bits of a one-byte value, which must never swap",
        write: |c, s, out| {
            correct_header(c, s, out);
            let swap = c.order != ByteOrder::native();
            for v in &s.values {
                match v {
                    Value::I8(x) if swap => out.push((*x as u8).reverse_bits()),
                    other => correct_values(
                        c,
                        &Sample {
                            timestamp: 0.0,
                            values: vec![other.clone()],
                        },
                        out,
                        swap,
                    ),
                }
            }
        },
    },
    Mutant {
        name: "drops_the_last_channel",
        reach: Reach::Oracle,
        why: "writes one value too few",
        write: |c, s, out| {
            correct_header(c, s, out);
            let short = Sample {
                timestamp: s.timestamp,
                values: s.values[..s.values.len().saturating_sub(1)].to_vec(),
            };
            correct_values(c, &short, out, c.order != ByteOrder::native());
        },
    },
    Mutant {
        name: "negative_zero_becomes_zero",
        reach: Reach::Oracle,
        why: "loses the sign of a negative zero, which a value comparison misses",
        write: |c, s, out| {
            correct_header(c, s, out);
            let fixed = Sample {
                timestamp: s.timestamp,
                values: s
                    .values
                    .iter()
                    .map(|v| match v {
                        Value::F32(x) if x.to_bits() == 0x8000_0000 => Value::F32(0.0),
                        Value::F64(x) if x.to_bits() == 0x8000_0000_0000_0000 => Value::F64(0.0),
                        o => o.clone(),
                    })
                    .collect(),
            };
            correct_values(c, &fixed, out, c.order != ByteOrder::native());
        },
    },
    Mutant {
        name: "subnormals_flushed_on_write",
        reach: Reach::Oracle,
        why: "clears a subnormal value when writing, which the connection never asked for",
        write: |c, s, out| {
            correct_header(c, s, out);
            let fixed = Sample {
                timestamp: s.timestamp,
                values: s
                    .values
                    .iter()
                    .map(|v| match v {
                        Value::F32(x) => {
                            let b = x.to_bits();
                            if b != 0 && (b & 0x7fff_ffff) <= 0x007f_ffff {
                                Value::F32(f32::from_bits(b & 0x8000_0000))
                            } else {
                                v.clone()
                            }
                        }
                        Value::F64(x) => {
                            let b = x.to_bits();
                            if b != 0 && (b & 0x7fff_ffff_ffff_ffff) <= 0x000f_ffff_ffff_ffff {
                                Value::F64(f64::from_bits(b & 0x8000_0000_0000_0000))
                            } else {
                                v.clone()
                            }
                        }
                        o => o.clone(),
                    })
                    .collect(),
            };
            correct_values(c, &fixed, out, c.order != ByteOrder::native());
        },
    },
    Mutant {
        name: "quiet_nan_becomes_a_different_nan",
        reach: Reach::Oracle,
        why: "rebuilds a value that is not a number instead of copying its bits",
        write: |c, s, out| {
            correct_header(c, s, out);
            let fixed = Sample {
                timestamp: s.timestamp,
                values: s
                    .values
                    .iter()
                    .map(|v| match v {
                        Value::F32(x) if x.is_nan() => Value::F32(f32::from_bits(0x7fc0_0001)),
                        Value::F64(x) if x.is_nan() => {
                            Value::F64(f64::from_bits(0x7ff8_0000_0000_0001))
                        }
                        o => o.clone(),
                    })
                    .collect(),
            };
            correct_values(c, &fixed, out, c.order != ByteOrder::native());
        },
    },
];

// ---------------------------------------------------------------------------
// the test
// ---------------------------------------------------------------------------

#[test]
fn every_mutant_is_caught_by_at_least_one_vector() {
    let vectors = load();
    if vectors.is_empty() {
        eprintln!("no vectors; skipping. Run oracle/makevectors.py first.");
        return;
    }

    let mut uncaught = Vec::new();
    let mut blocked = Vec::new();
    let mut report = Vec::new();

    for m in MUTANTS {
        let mut caught_by = 0usize;
        let mut first: Option<&str> = None;

        for v in &vectors {
            let mut out = Vec::with_capacity(v.bytes.len());
            for s in &v.samples {
                (m.write)(&v.codec, s, &mut out);
            }
            if out != v.bytes {
                caught_by += 1;
                if first.is_none() {
                    first = Some(&v.name);
                }
            }
        }

        match &m.reach {
            Reach::Oracle => {
                if caught_by == 0 {
                    uncaught.push(format!("{}: {}", m.name, m.why));
                }
                report.push(format!(
                    "  {:34} caught by {:3}/{} vectors  first: {}",
                    m.name,
                    caught_by,
                    vectors.len(),
                    first.unwrap_or("-")
                ));
            }
            Reach::Blocked(reason) => {
                // A blocked defect that a vector still catches means the
                // reason is wrong. Say so instead of passing in silence.
                assert_eq!(
                    caught_by, 0,
                    "{}: this defect is marked as blocked, and {caught_by} vectors caught \
                     it. The reason is wrong: {reason}",
                    m.name
                );
                report.push(format!("  {:34} BLOCKED, a direct test covers it", m.name));
                blocked.push(format!("{}: {}", m.name, reason));
            }
        }
    }

    println!("mutation report over {} vectors:", vectors.len());
    for line in &report {
        println!("{line}");
    }
    if !blocked.is_empty() {
        println!(
            "\n{} defect(s) that no golden vector can reach:",
            blocked.len()
        );
        for b in &blocked {
            println!("  {b}");
        }
    }

    assert!(
        uncaught.is_empty(),
        "these defects slipped past every vector. The vector set has a hole, \
         and the fix is a new vector.\n  {}",
        uncaught.join("\n  ")
    );
}

/// The control. The correct encoder must match every vector.
///
/// Without this, a mutation report of "everything caught" also appears when
/// the loader is broken and every comparison differs.
#[test]
fn the_real_encoder_is_not_caught() {
    let vectors = load();
    if vectors.is_empty() {
        return;
    }
    for v in &vectors {
        let mut out = Vec::with_capacity(v.bytes.len());
        for s in &v.samples {
            v.codec.encode(s, &mut out).expect("encode");
        }
        assert_eq!(
            out, v.bytes,
            "{}: the real encoder differs from the oracle",
            v.name
        );
    }
    println!("the real encoder matches all {} vectors", vectors.len());
}

/// Cover the defect that no golden vector can reach.
///
/// The oracle never swaps a one-byte format, so no capture holds a swapped
/// one-byte stream. The rule still belongs in the codec, and this test states
/// it directly: a one-byte value carries the same bytes in either order.
#[test]
fn a_one_byte_value_never_swaps() {
    let little = Codec::new(Format::Int8, 8, ByteOrder::Little);
    let big = Codec::new(Format::Int8, 8, ByteOrder::Big);
    let s = Sample {
        timestamp: 1.5,
        values: (0..8).map(|k| Value::I8(k as i8 - 4)).collect(),
    };

    let a = little.encode_to_vec(&s).expect("encode little");
    let b = big.encode_to_vec(&s).expect("encode big");

    // The timestamp still swaps, because it is eight bytes wide. Only the
    // channel bytes must match.
    assert_eq!(a[9..], b[9..], "a one-byte channel value must not swap");
    assert_ne!(a[1..9], b[1..9], "the timestamp must still swap");

    // Both orders read back to the same values.
    assert_eq!(little.decode(&a).expect("decode little").0.values, s.values);
    assert_eq!(big.decode(&b).expect("decode big").0.values, s.values);
}
