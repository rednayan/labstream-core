//! Transcript replay against the reference oracle.
//!
//! Each transcript holds one request that went to a real liblsl outlet, and
//! the answer that came back. `oracle/recordtranscripts.py` records them.
//!
//! The test drives the same request through this crate and compares the two
//! answers. The comparison runs after canonicalizing, because an answer
//! carries a UID that changes on every run. Every placeholder carries a
//! predicate, so an erased field still gets an assertion.
//!
//! A test that only checked the status code would pass while every negotiated
//! field was wrong. The comparison covers the whole block.

use labstream_proto::canon::{canonicalize, check, feed_rules, lint};
use labstream_proto::header::{parse_block, StatusLine};
use labstream_proto::negotiate::{negotiate, FeedRequest, Outcome, ServerStream, BIG, LITTLE};
use labstream_wire::Format;
use serde_json::Value as J;
use std::path::{Path, PathBuf};

/// The conversion speed that the replay gives the server.
///
/// A real value comes from a benchmark and it differs between machines. The
/// transcripts use only the two unambiguous extremes for the client: a value
/// of 0, where the outlet always wins, and 10^12, where it always loses. Any
/// positive number between those two reproduces both outcomes.
const SERVER_ENDIAN_PERFORMANCE: f64 = 1.0;

fn transcript_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/transcripts")
}

fn load() -> Vec<J> {
    let dir = transcript_dir();
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

/// Split a recorded request into its request line and its header block.
fn split_request(raw: &str) -> (i32, String, labstream_proto::Headers) {
    let nl = raw.find('\n').expect("a request line");
    let line = raw[..nl].trim_end_matches('\r');
    let rest = &raw[nl + 1..];

    // `LSL:streamfeed/<version> <uid>`
    let head = line
        .strip_prefix("LSL:streamfeed/")
        .expect("a feed request");
    let mut parts = head.splitn(2, ' ');
    let version: i32 = parts.next().unwrap().parse().expect("a version");
    let uid = parts.next().unwrap_or("").trim().to_string();

    let (headers, _) = parse_block(rest.as_bytes()).expect("a header block");
    (version, uid, headers)
}

/// Run our negotiation over a recorded request and write the answer.
fn our_answer(t: &J) -> String {
    let format = format_of(t["format"].as_str().unwrap());
    let stream_uid = t["stream_uid"].as_str().unwrap();
    let raw = t["request"].as_str().unwrap();
    let (version, req_uid, headers) = split_request(raw);

    let mut srv = ServerStream::new(stream_uid, format, LITTLE, SERVER_ENDIAN_PERFORMANCE);
    // A string stream carries no fixed channel width.
    if format == Format::String {
        srv.channel_bytes = 0;
    }

    let req = FeedRequest::from_headers(version, &req_uid, &headers, srv.channel_bytes);
    match negotiate(&req, &srv) {
        Outcome::Refuse(s) => s.to_wire(),
        Outcome::Accept(a) => a.to_wire(srv.configured_version, stream_uid),
    }
}

#[test]
fn transcripts_exist() {
    let all = load();
    assert!(
        !all.is_empty(),
        "no transcripts in {}. Run oracle/recordtranscripts.py first.",
        transcript_dir().display()
    );
    println!("loaded {} transcripts", all.len());
}

#[test]
fn the_canonical_rules_pass_the_lint() {
    // Risk R3 of the conformance plan: a placeholder with no predicate hides a
    // difference. This gate runs before any comparison uses the rules.
    let failures = lint(&feed_rules());
    assert!(
        failures.is_empty(),
        "the canonical rules fail the lint: {failures:?}"
    );
}

#[test]
fn every_transcript_replays() {
    let all = load();
    if all.is_empty() {
        eprintln!("no transcripts; skipping");
        return;
    }
    let rules = feed_rules();
    let mut checked = 0usize;
    let mut mismatches = Vec::new();

    for t in &all {
        let case = t["case"].as_str().unwrap();
        let oracle_raw = t["answer"].as_str().unwrap();

        let ours = our_answer(t);
        let a = canonicalize(oracle_raw, &rules);
        let b = canonicalize(&ours, &rules);

        // Every erased value still gets an assertion.
        for v in check(&a, &rules) {
            mismatches.push(format!(
                "{case}: the oracle answer failed a predicate: {v:?}"
            ));
        }
        for v in check(&b, &rules) {
            mismatches.push(format!("{case}: our answer failed a predicate: {v:?}"));
        }

        if a.text.trim() != b.text.trim() {
            mismatches.push(format!(
                "{case}: the answers differ\n  oracle: {:?}\n  ours:   {:?}",
                a.text.trim(),
                b.text.trim()
            ));
        }
        checked += 1;
    }

    assert!(
        mismatches.is_empty(),
        "{} of {checked} transcripts differ:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    println!("{checked} transcripts replay with the same answer");
}

#[test]
fn the_recorded_status_codes_match_the_specification() {
    let all = load();
    if all.is_empty() {
        return;
    }
    let mut seen = std::collections::BTreeSet::new();
    for t in &all {
        let case = t["case"].as_str().unwrap();
        let answer = t["answer"].as_str().unwrap();
        let line = answer.split("\r\n").next().unwrap();
        let s = StatusLine::parse(line).unwrap_or_else(|e| panic!("{case}: {e}"));
        seen.insert(s.code);

        match case {
            "version_too_new" => assert_eq!(s.code, 505, "{case}"),
            "wrong_uid" => assert_eq!(s.code, 404, "{case}"),
            _ => assert_eq!(s.code, 200, "{case}"),
        }
    }
    // The corpus must cover every code that SPEC.md 4.5 names.
    assert!(
        seen.contains(&200) && seen.contains(&404) && seen.contains(&505),
        "the corpus misses a status code. Found {seen:?}"
    );
    println!("status codes covered: {seen:?}");
}

/// The negotiation outcomes that the corpus must show.
///
/// A transcript set that only held accepted requests would replay perfectly
/// while every branch stayed untested. This names the branches.
#[test]
fn the_corpus_covers_every_negotiation_branch() {
    let all = load();
    if all.is_empty() {
        return;
    }
    let find = |case: &str| {
        all.iter()
            .find(|t| t["case"].as_str() == Some(case))
            .unwrap_or_else(|| panic!("the corpus misses the case {case}"))
    };
    let field = |t: &J, name: &str| -> String {
        let ans = t["answer"].as_str().unwrap();
        for line in ans.split("\r\n") {
            if let Some(c) = line.find(':') {
                if line[..c].trim().eq_ignore_ascii_case(name) {
                    return line[c + 1..].trim().to_string();
                }
            }
        }
        String::new()
    };

    // The outlet swaps only when it wins the speed contest.
    assert_eq!(
        field(find("swap_slow_client"), "Byte-Order"),
        BIG.to_string()
    );
    assert_eq!(
        field(find("noswap_fast_client"), "Byte-Order"),
        LITTLE.to_string()
    );

    // A one-byte format never swaps, whatever the request asks for.
    assert_eq!(
        field(find("onebyte_never_swaps"), "Byte-Order"),
        LITTLE.to_string()
    );

    // Two conditions force protocol 1.00.
    assert_eq!(
        field(find("downgrade_value_size"), "Data-Protocol-Version"),
        "100"
    );
    assert_eq!(
        field(find("downgrade_no_ieee754"), "Data-Protocol-Version"),
        "100"
    );

    // Suppression follows the client, and only for a floating point format.
    assert_eq!(
        field(find("suppress_subnormals"), "Suppress-Subnormals"),
        "1"
    );
    assert_eq!(
        field(find("subnormals_int_format"), "Suppress-Subnormals"),
        "0"
    );

    // SPEC.md 4.4: the server reads `Protocol-Version` and never
    // `Data-Protocol-Version`.
    assert_eq!(
        field(find("live_protocol_version"), "Data-Protocol-Version"),
        "100",
        "Protocol-Version must change the negotiated version"
    );
    assert_eq!(
        field(find("dead_header_only"), "Data-Protocol-Version"),
        "110",
        "Data-Protocol-Version in a request must change nothing"
    );

    println!("every negotiation branch appears in the corpus");
}
