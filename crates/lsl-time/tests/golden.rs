//! Golden sequence tests for the jitter filter.
//!
//! Each sequence holds an input timestamp and the value that the real liblsl
//! filter returned. `oracle/gentime.cpp` builds them, linked against the object
//! files of the pinned oracle build, so the numbers come from the same code
//! that a real inlet runs.
//!
//! The filter is deterministic for a given input. That is what makes an exact
//! comparison possible here, where M4 and M5 could only compare a stream that
//! both sides agreed on.
//!
//! Every number crosses the boundary as a bit pattern, so no text formatting of
//! a float takes place. The comparison then sees the exact value, including a
//! negative zero and a value that is not a number.

use lsl_time::Dejitterer;
use serde_json::Value as J;
use std::path::{Path, PathBuf};

/// The filter must agree with the oracle to the bit.
///
/// This bar is measured and not hoped for. Every one of the 96,008 samples in
/// this repository agrees exactly, and `report_worst` prints the count.
///
/// A looser bar would be wrong here. Reassociating one sum inside the filter
/// changes about one value in nine by roughly one unit in the last place, and a
/// tolerance of 1e-12 hides every one of those. The mutation test measured
/// that, so the tolerance went away.
///
/// A failure on another processor is worth reading and not worth relaxing. A
/// difference near one unit in the last place means the operation order
/// differs. A larger one means the algorithm differs.
fn same(got: f64, want: f64) -> bool {
    got.to_bits() == want.to_bits()
}

fn sequence_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sequences")
}

fn load() -> Vec<(String, J)> {
    let dir = sequence_dir();
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
        .map(|p| {
            let j: J = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
            (p.file_stem().unwrap().to_string_lossy().into_owned(), j)
        })
        .collect()
}

fn bits(s: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(s, 16).expect("a bit pattern"))
}

struct Sequence {
    name: String,
    srate: f64,
    halftime: f64,
    input: Vec<f64>,
    expected: Vec<f64>,
    final_state: J,
}

fn parse(name: &str, j: &J) -> Sequence {
    let arr = j["samples"].as_array().expect("samples");
    let mut input = Vec::with_capacity(arr.len());
    let mut expected = Vec::with_capacity(arr.len());
    for pair in arr {
        let p = pair.as_array().expect("a pair");
        input.push(bits(p[0].as_str().unwrap()));
        expected.push(bits(p[1].as_str().unwrap()));
    }
    Sequence {
        name: name.to_string(),
        srate: bits(j["srate_bits"].as_str().unwrap()),
        halftime: bits(j["halftime_bits"].as_str().unwrap()),
        input,
        expected,
        final_state: j["final"].clone(),
    }
}

#[test]
fn sequences_exist() {
    let all = load();
    assert!(
        !all.is_empty(),
        "no sequences in {}. Build oracle/gentime.cpp and run it first.",
        sequence_dir().display()
    );
    let total: usize = all
        .iter()
        .map(|(_, j)| j["samples"].as_array().unwrap().len())
        .sum();
    println!("loaded {} sequences, {total} samples", all.len());
    assert!(
        total >= 10_000,
        "the plan asks for sequences of 10,000 samples or more. Found {total}."
    );
}

#[test]
fn every_sequence_matches_the_oracle() {
    let all = load();
    if all.is_empty() {
        eprintln!("no sequences; skipping");
        return;
    }
    let mut checked = 0usize;

    for (name, j) in &all {
        let s = parse(name, j);
        let mut d = Dejitterer::new(s.input[0], s.srate, s.halftime);

        for (i, t) in s.input.iter().enumerate() {
            let got = d.dejitter(*t);
            let want = s.expected[i];
            if same(got, want) {
                continue;
            }
            let scale = want.abs().max(1.0);
            let diff = (got - want).abs() / scale;
            let ulps = diff / f64::EPSILON;
            panic!(
                "{}: sample {i} differs\n  input    {t:?}\n  \
                 expected {want:?} ({:016x})\n  got      {got:?} ({:016x})\n  \
                 relative difference {diff:e}, about {ulps:.1} units in the last place\n  \
                 A difference near one unit means the operation order differs. \
                 Read the float discipline note in the crate documentation.",
                s.name,
                want.to_bits(),
                got.to_bits()
            );
        }
        checked += s.input.len();
    }
    println!("{checked} samples match across {} sequences", all.len());
}

#[test]
fn the_final_state_matches_the_oracle() {
    // The returned values can agree while the internal state drifts. This
    // compares the state that carries into every later sample.
    let all = load();
    if all.is_empty() {
        return;
    }
    for (name, j) in &all {
        let s = parse(name, j);
        let mut d = Dejitterer::new(s.input[0], s.srate, s.halftime);
        for t in &s.input {
            d.dejitter(*t);
        }
        let f = &s.final_state;
        let cmp = |field: &str, got: f64| {
            let want = bits(f[field].as_str().unwrap());
            assert!(
                same(got, want),
                "{}: the field {field} differs\n  expected {want:?} ({:016x})\n  \
                 got      {got:?} ({:016x})",
                s.name,
                want.to_bits(),
                got.to_bits()
            );
        };
        cmp("w0", d.w0);
        cmp("w1", d.w1);
        cmp("p00", d.p00);
        cmp("p01", d.p01);
        cmp("p11", d.p11);
        cmp("lam", d.lam);
        assert_eq!(d.t0 as u64, f["t0"].as_u64().unwrap(), "{}: t0", s.name);
        assert_eq!(
            d.samples_since_t0 as u64,
            f["samples_since_t0"].as_u64().unwrap(),
            "{}: the sample counter",
            s.name
        );
    }
    println!("the final state matches for all {} sequences", all.len());
}

/// Report how close the agreement really is.
///
/// The tolerance above allows a small difference. This test prints the largest
/// one that actually occurs, so a claim of bit-exact agreement rests on a
/// number and not on a hope.
#[test]
fn report_worst() {
    let all = load();
    if all.is_empty() {
        return;
    }
    let mut worst = 0f64;
    let mut worst_name = String::new();
    let mut exact = 0usize;
    let mut total = 0usize;

    for (name, j) in &all {
        let s = parse(name, j);
        let mut d = Dejitterer::new(s.input[0], s.srate, s.halftime);
        for (i, t) in s.input.iter().enumerate() {
            let got = d.dejitter(*t);
            let want = s.expected[i];
            total += 1;
            if got.to_bits() == want.to_bits() {
                exact += 1;
                continue;
            }
            let scale = want.abs().max(1.0);
            let diff = (got - want).abs() / scale;
            if diff > worst {
                worst = diff;
                worst_name = format!("{}:{i}", s.name);
            }
        }
    }
    println!(
        "bit-exact on {exact} of {total} samples ({:.4}%)",
        100.0 * exact as f64 / total as f64
    );
    if worst > 0.0 {
        println!("largest relative difference {worst:e} at {worst_name}");
    } else {
        println!("every sample agrees to the bit");
    }
}
