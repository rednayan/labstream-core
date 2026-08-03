//! Test the transcripts.
//!
//! A transcript corpus that replays perfectly proves nothing on its own. This
//! file holds deliberate defects in the negotiation. Each one must break at
//! least one transcript.
//!
//! A defect that no transcript catches means the corpus has a hole. The fix is
//! then a new transcript and not a weaker assertion.
//!
//! The same shape as `crates/lsl-wire/tests/mutants.rs`, applied to a state
//! machine instead of a codec.

use lsl_proto::header::{parse_block, Headers, StatusLine};
use lsl_proto::negotiate::{
    can_convert_endian, Accepted, FeedRequest, Outcome, ServerStream, LITTLE,
};
use lsl_wire::Format;
use serde_json::Value as J;
use std::path::Path;

const SERVER_ENDIAN_PERFORMANCE: f64 = 1.0;

fn load() -> Vec<J> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/transcripts");
    let mut paths: Vec<_> = match std::fs::read_dir(&dir) {
        Ok(e) => e
            .filter_map(|x| x.ok())
            .map(|x| x.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect(),
        Err(_) => return Vec::new(),
    };
    paths.sort();
    paths
        .iter()
        .map(|p| serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap())
        .collect()
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
        o => panic!("unknown format {o}"),
    }
}

fn split_request(raw: &str) -> (i32, String, Headers) {
    let nl = raw.find('\n').expect("a request line");
    let line = raw[..nl].trim_end_matches('\r');
    let head = line
        .strip_prefix("LSL:streamfeed/")
        .expect("a feed request");
    let mut parts = head.splitn(2, ' ');
    let version: i32 = parts.next().unwrap().parse().unwrap();
    let uid = parts.next().unwrap_or("").trim().to_string();
    let (headers, _) = parse_block(&raw.as_bytes()[nl + 1..]).expect("a header block");
    (version, uid, headers)
}

/// A negotiation with a named defect.
///
/// Every mutant is a copy of the real function with one rule changed. Writing
/// them out in full keeps each defect visible in one place.
type Negotiator = fn(&FeedRequest, &ServerStream) -> Outcome;

struct Mutant {
    name: &'static str,
    why: &'static str,
    run: Negotiator,
}

fn refuse(version: i32, code: i32, message: &str) -> Outcome {
    Outcome::Refuse(StatusLine {
        version,
        code,
        message: message.to_string(),
    })
}

/// The shape that every mutant follows. The flags turn single rules off.
#[allow(clippy::too_many_arguments)]
fn negotiate_with(
    req: &FeedRequest,
    srv: &ServerStream,
    check_version: bool,
    check_uid: bool,
    version_pick_max: bool,
    downgrade_value_size: bool,
    downgrade_ieee: bool,
    swap_condition: fn(&FeedRequest, &ServerStream) -> bool,
    suppress_ignores_format: bool,
    version_code: i32,
    uid_code: i32,
) -> Outcome {
    if check_version && req.request_version / 100 > srv.configured_version / 100 {
        return refuse(
            srv.configured_version,
            version_code,
            "Version not supported",
        );
    }
    if check_uid && !req.request_uid.is_empty() && req.request_uid != srv.uid {
        return refuse(srv.configured_version, uid_code, "Not found");
    }
    if req.request_version < 110 {
        return Outcome::Accept(Accepted {
            data_protocol_version: 100,
            byte_order: srv.native_byte_order,
            reverse_byte_order: false,
            suppress_subnormals: false,
            max_buffer_length: req.max_buffer_length,
            max_chunk_length: req.max_chunk_length,
        });
    }

    let mut version = if version_pick_max {
        srv.configured_version.max(req.protocol_version)
    } else {
        srv.configured_version.min(req.protocol_version)
    };
    if downgrade_value_size && srv.format != Format::String && srv.channel_bytes != req.value_size {
        version = 100;
    }
    if downgrade_ieee && !req.has_ieee754_floats {
        version = 100;
    }

    let mut byte_order = srv.native_byte_order;
    let mut reverse = false;
    let mut suppress = false;
    if version >= 110 {
        if swap_condition(req, srv) {
            byte_order = req.native_byte_order;
            reverse = true;
        }
        suppress = (suppress_ignores_format || srv.format.is_float()) && !req.supports_subnormals;
    }

    Outcome::Accept(Accepted {
        data_protocol_version: version,
        byte_order,
        reverse_byte_order: reverse,
        suppress_subnormals: suppress,
        max_buffer_length: req.max_buffer_length,
        max_chunk_length: req.max_chunk_length,
    })
}

fn real_swap(req: &FeedRequest, srv: &ServerStream) -> bool {
    srv.native_byte_order != req.native_byte_order
        && can_convert_endian(req.native_byte_order, req.value_size)
        && req.value_size > 1
        && srv.endian_performance > req.endian_performance
}

const MUTANTS: &[Mutant] = &[
    Mutant {
        name: "version_picks_the_higher",
        why: "takes the larger of the two versions instead of the smaller",
        run: |r, s| {
            negotiate_with(
                r, s, true, true, true, true, true, real_swap, false, 505, 404,
            )
        },
    },
    Mutant {
        name: "no_value_size_downgrade",
        why: "keeps 1.10 when the client value size differs from the channel width",
        run: |r, s| {
            negotiate_with(
                r, s, true, true, false, false, true, real_swap, false, 505, 404,
            )
        },
    },
    Mutant {
        name: "no_ieee754_downgrade",
        why: "keeps 1.10 when the client reports no IEEE-754 floats",
        run: |r, s| {
            negotiate_with(
                r, s, true, true, false, true, false, real_swap, false, 505, 404,
            )
        },
    },
    Mutant {
        name: "swaps_when_slower",
        why: "reverses the speed contest, so the slower side converts",
        run: |r, s| {
            negotiate_with(
                r,
                s,
                true,
                true,
                false,
                true,
                true,
                |q, v| {
                    v.native_byte_order != q.native_byte_order
                        && can_convert_endian(q.native_byte_order, q.value_size)
                        && q.value_size > 1
                        && v.endian_performance < q.endian_performance
                },
                false,
                505,
                404,
            )
        },
    },
    Mutant {
        name: "swaps_a_one_byte_value",
        why: "drops condition 3, so a one-byte format swaps",
        run: |r, s| {
            negotiate_with(
                r,
                s,
                true,
                true,
                false,
                true,
                true,
                |q, v| {
                    v.native_byte_order != q.native_byte_order
                        && can_convert_endian(q.native_byte_order, q.value_size)
                        && v.endian_performance > q.endian_performance
                },
                false,
                505,
                404,
            )
        },
    },
    Mutant {
        name: "accepts_any_byte_order",
        why: "drops the conversion test, so an unknown order still swaps",
        run: |r, s| {
            negotiate_with(
                r,
                s,
                true,
                true,
                false,
                true,
                true,
                |q, v| {
                    v.native_byte_order != q.native_byte_order
                        && q.value_size > 1
                        && v.endian_performance > q.endian_performance
                },
                false,
                505,
                404,
            )
        },
    },
    Mutant {
        name: "suppresses_for_every_format",
        why: "clears subnormal values for an integer format, which holds none",
        run: |r, s| {
            negotiate_with(
                r, s, true, true, false, true, true, real_swap, true, 505, 404,
            )
        },
    },
    Mutant {
        name: "skips_the_version_test",
        why: "accepts a newer major version instead of answering 505",
        run: |r, s| {
            negotiate_with(
                r, s, false, true, false, true, true, real_swap, false, 505, 404,
            )
        },
    },
    Mutant {
        name: "skips_the_uid_test",
        why: "accepts a UID that names another stream instead of answering 404",
        run: |r, s| {
            negotiate_with(
                r, s, true, false, false, true, true, real_swap, false, 505, 404,
            )
        },
    },
    Mutant {
        name: "wrong_code_for_a_new_version",
        why: "answers 400 instead of 505",
        run: |r, s| {
            negotiate_with(
                r, s, true, true, false, true, true, real_swap, false, 400, 404,
            )
        },
    },
    Mutant {
        name: "wrong_code_for_a_bad_uid",
        why: "answers 400 instead of 404, which changes what the client does",
        run: |r, s| {
            negotiate_with(
                r, s, true, true, false, true, true, real_swap, false, 505, 400,
            )
        },
    },
];

/// A defect that lives in the header reader and not in the negotiation.
///
/// The server reads `Protocol-Version`. A reimplementation that reads
/// `Data-Protocol-Version` instead looks correct in the source of the client.
/// SPEC.md 4.4.
fn request_reading_the_dead_header(
    version: i32,
    uid: &str,
    h: &Headers,
    bytes: i32,
) -> FeedRequest {
    let mut r = FeedRequest::from_headers(version, uid, h, bytes);
    r.protocol_version = h.get_int("data-protocol-version").unwrap_or(version as i64) as i32;
    r
}

fn answer_of(o: Outcome, srv: &ServerStream) -> String {
    match o {
        Outcome::Refuse(s) => s.to_wire(),
        Outcome::Accept(a) => a.to_wire(srv.configured_version, &srv.uid),
    }
}

fn server_for(t: &J) -> ServerStream {
    let format = format_of(t["format"].as_str().unwrap());
    let mut srv = ServerStream::new(
        t["stream_uid"].as_str().unwrap(),
        format,
        LITTLE,
        SERVER_ENDIAN_PERFORMANCE,
    );
    if format == Format::String {
        srv.channel_bytes = 0;
    }
    srv
}

#[test]
fn every_mutant_is_caught_by_at_least_one_transcript() {
    let all = load();
    if all.is_empty() {
        eprintln!("no transcripts; skipping. Run oracle/recordtranscripts.py first.");
        return;
    }

    let mut uncaught = Vec::new();
    let mut report = Vec::new();

    for m in MUTANTS {
        let mut caught = 0usize;
        let mut first = None;
        for t in &all {
            let srv = server_for(t);
            let (version, uid, headers) = split_request(t["request"].as_str().unwrap());
            let req = FeedRequest::from_headers(version, &uid, &headers, srv.channel_bytes);
            let ours = answer_of((m.run)(&req, &srv), &srv);
            let oracle = t["answer"].as_str().unwrap();
            if ours.trim() != oracle.trim() {
                caught += 1;
                if first.is_none() {
                    first = t["case"].as_str();
                }
            }
        }
        if caught == 0 {
            uncaught.push(format!("{}: {}", m.name, m.why));
        }
        report.push(format!(
            "  {:32} caught by {:2}/{} transcripts  first: {}",
            m.name,
            caught,
            all.len(),
            first.unwrap_or("-")
        ));
    }

    // The dead header defect lives in the reader, so it needs its own pass.
    {
        let mut caught = 0usize;
        let mut first = None;
        for t in &all {
            let srv = server_for(t);
            let (version, uid, headers) = split_request(t["request"].as_str().unwrap());
            let req = request_reading_the_dead_header(version, &uid, &headers, srv.channel_bytes);
            let ours = answer_of(lsl_proto::negotiate(&req, &srv), &srv);
            if ours.trim() != t["answer"].as_str().unwrap().trim() {
                caught += 1;
                if first.is_none() {
                    first = t["case"].as_str();
                }
            }
        }
        if caught == 0 {
            uncaught.push(
                "reads_data_protocol_version: reads the header that liblsl ignores".to_string(),
            );
        }
        report.push(format!(
            "  {:32} caught by {:2}/{} transcripts  first: {}",
            "reads_data_protocol_version",
            caught,
            all.len(),
            first.unwrap_or("-")
        ));
    }

    println!("mutation report over {} transcripts:", all.len());
    for line in &report {
        println!("{line}");
    }

    assert!(
        uncaught.is_empty(),
        "these defects slipped past every transcript. The corpus has a hole, \
         and the fix is a new transcript.\n  {}",
        uncaught.join("\n  ")
    );
}

/// The control. The real negotiation must match every transcript.
///
/// Without this, a report of "everything caught" also appears when the loader
/// is broken and every comparison differs.
#[test]
fn the_real_negotiation_is_not_caught() {
    let all = load();
    if all.is_empty() {
        return;
    }
    for t in &all {
        let srv = server_for(t);
        let (version, uid, headers) = split_request(t["request"].as_str().unwrap());
        let req = FeedRequest::from_headers(version, &uid, &headers, srv.channel_bytes);
        let ours = answer_of(lsl_proto::negotiate(&req, &srv), &srv);
        let oracle = t["answer"].as_str().unwrap();
        // The UID differs per run, and the answer carries it. Compare the
        // lines that hold no UID.
        let strip = |s: &str| {
            s.split("\r\n")
                .filter(|l| !l.to_ascii_lowercase().starts_with("uid:"))
                .collect::<Vec<_>>()
                .join("\r\n")
        };
        assert_eq!(
            strip(&ours).trim(),
            strip(oracle).trim(),
            "{}: the real negotiation differs from the oracle",
            t["case"].as_str().unwrap()
        );
    }
    println!("the real negotiation matches all {} transcripts", all.len());
}
