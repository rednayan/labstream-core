//! Test the sequences.
//!
//! The golden sequences agree with the oracle to the bit. That result means
//! nothing until the sequences are shown to notice a defect.
//!
//! This file holds deliberate changes to the filter. Most of them are the kind
//! that a careful reader would make while tidying the code: reassociating an
//! expression, folding a multiply and an add together, or moving a line.
//!
//! Every one must break at least one sequence.

use serde_json::Value as J;
use std::path::Path;

/// A mutant counts as caught when any value differs from the oracle by a
/// single bit.
///
/// A looser bar measured badly. Reassociating one sum changes about one value
/// in nine by roughly one unit in the last place, and a tolerance of 1e-12
/// reported that change as uncaught. The sequences did discriminate it. The
/// comparison did not.
fn same(got: f64, want: f64) -> bool {
    got.to_bits() == want.to_bits()
}

fn load() -> Vec<(String, J)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sequences");
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
    f64::from_bits(u64::from_str_radix(s, 16).unwrap())
}

/// A copy of the filter state that a mutant can change.
#[derive(Clone)]
struct State {
    t0: u32,
    n: u32,
    w0: f64,
    w1: f64,
    p00: f64,
    p11: f64,
    p01: f64,
    lam: f64,
}

impl State {
    fn new(t0: f64, srate: f64, halftime: f64) -> Self {
        let mut s = State {
            t0: t0 as u32,
            n: 0,
            w0: 0.0,
            w1: 0.0,
            p00: 1e10,
            p11: 1e10,
            p01: 0.0,
            lam: 0.0,
        };
        if srate > 0.0 {
            s.w1 = 1. / srate;
            s.lam = 2f64.powf(-1. / (srate * halftime));
        }
        s
    }
}

struct Mutant {
    name: &'static str,
    why: &'static str,
    /// Build the state. A mutant can change an initial value.
    init: fn(f64, f64, f64) -> State,
    /// Take one timestamp.
    step: fn(&mut State, f64) -> f64,
}

fn real_step(s: &mut State, t: f64) -> f64 {
    let t = t - s.t0 as f64;
    let u1 = s.n as f64;
    s.n += 1;
    let pi0 = s.p00 + u1 * s.p01;
    let pi1 = s.p01 + u1 * s.p11;
    let al = t - (s.w0 + u1 * s.w1);
    let g_inv = 1. / (s.lam + pi0 + pi1 * u1);
    let il_ = 1. / s.lam;
    s.p00 = il_ * (s.p00 - pi0 * pi0 * g_inv);
    s.p01 = il_ * (s.p01 - pi0 * pi1 * g_inv);
    s.p11 = il_ * (s.p11 - pi1 * pi1 * g_inv);
    s.w0 += al * (s.p00 + s.p01 * u1);
    s.w1 += al * (s.p01 + s.p11 * u1);
    s.w0 + u1 * s.w1 + s.t0 as f64
}

const MUTANTS: &[Mutant] = &[
    Mutant {
        name: "no_baseline_subtraction",
        why: "drops the integer baseline, which the comment calls a numerical aid",
        init: State::new,
        step: |s, t| {
            let u1 = s.n as f64;
            s.n += 1;
            let pi0 = s.p00 + u1 * s.p01;
            let pi1 = s.p01 + u1 * s.p11;
            let al = t - (s.w0 + u1 * s.w1);
            let g_inv = 1. / (s.lam + pi0 + pi1 * u1);
            let il_ = 1. / s.lam;
            s.p00 = il_ * (s.p00 - pi0 * pi0 * g_inv);
            s.p01 = il_ * (s.p01 - pi0 * pi1 * g_inv);
            s.p11 = il_ * (s.p11 - pi1 * pi1 * g_inv);
            s.w0 += al * (s.p00 + s.p01 * u1);
            s.w1 += al * (s.p01 + s.p11 * u1);
            s.w0 + u1 * s.w1
        },
    },
    Mutant {
        name: "fused_multiply_add",
        why: "folds a multiply and an add together, which changes the rounding",
        init: State::new,
        step: |s, t| {
            let t = t - s.t0 as f64;
            let u1 = s.n as f64;
            s.n += 1;
            let pi0 = u1.mul_add(s.p01, s.p00);
            let pi1 = u1.mul_add(s.p11, s.p01);
            let al = t - u1.mul_add(s.w1, s.w0);
            let g_inv = 1. / (s.lam + pi0 + pi1 * u1);
            let il_ = 1. / s.lam;
            s.p00 = il_ * (-(pi0 * pi0)).mul_add(g_inv, s.p00);
            s.p01 = il_ * (-(pi0 * pi1)).mul_add(g_inv, s.p01);
            s.p11 = il_ * (-(pi1 * pi1)).mul_add(g_inv, s.p11);
            s.w0 = al.mul_add(s.p01.mul_add(u1, s.p00), s.w0);
            s.w1 = al.mul_add(s.p11.mul_add(u1, s.p01), s.w1);
            u1.mul_add(s.w1, s.w0) + s.t0 as f64
        },
    },
    Mutant {
        name: "reassociated_gain",
        why: "rewrites the gain denominator in a different order",
        init: State::new,
        step: |s, t| {
            let t = t - s.t0 as f64;
            let u1 = s.n as f64;
            s.n += 1;
            let pi0 = s.p00 + u1 * s.p01;
            let pi1 = s.p01 + u1 * s.p11;
            let al = t - (s.w0 + u1 * s.w1);
            // The order of the three terms changed.
            let g_inv = 1. / (pi1 * u1 + pi0 + s.lam);
            let il_ = 1. / s.lam;
            s.p00 = il_ * (s.p00 - pi0 * pi0 * g_inv);
            s.p01 = il_ * (s.p01 - pi0 * pi1 * g_inv);
            s.p11 = il_ * (s.p11 - pi1 * pi1 * g_inv);
            s.w0 += al * (s.p00 + s.p01 * u1);
            s.w1 += al * (s.p01 + s.p11 * u1);
            s.w0 + u1 * s.w1 + s.t0 as f64
        },
    },
    Mutant {
        name: "weights_before_covariance",
        why: "updates the weights before the covariance, so it uses the old values",
        init: State::new,
        step: |s, t| {
            let t = t - s.t0 as f64;
            let u1 = s.n as f64;
            s.n += 1;
            let pi0 = s.p00 + u1 * s.p01;
            let pi1 = s.p01 + u1 * s.p11;
            let al = t - (s.w0 + u1 * s.w1);
            let g_inv = 1. / (s.lam + pi0 + pi1 * u1);
            let il_ = 1. / s.lam;
            s.w0 += al * (s.p00 + s.p01 * u1);
            s.w1 += al * (s.p01 + s.p11 * u1);
            s.p00 = il_ * (s.p00 - pi0 * pi0 * g_inv);
            s.p01 = il_ * (s.p01 - pi0 * pi1 * g_inv);
            s.p11 = il_ * (s.p11 - pi1 * pi1 * g_inv);
            s.w0 + u1 * s.w1 + s.t0 as f64
        },
    },
    Mutant {
        name: "counter_increments_after_use",
        why: "reads the counter after the increment instead of before",
        init: State::new,
        step: |s, t| {
            let t = t - s.t0 as f64;
            s.n += 1;
            let u1 = s.n as f64;
            let pi0 = s.p00 + u1 * s.p01;
            let pi1 = s.p01 + u1 * s.p11;
            let al = t - (s.w0 + u1 * s.w1);
            let g_inv = 1. / (s.lam + pi0 + pi1 * u1);
            let il_ = 1. / s.lam;
            s.p00 = il_ * (s.p00 - pi0 * pi0 * g_inv);
            s.p01 = il_ * (s.p01 - pi0 * pi1 * g_inv);
            s.p11 = il_ * (s.p11 - pi1 * pi1 * g_inv);
            s.w0 += al * (s.p00 + s.p01 * u1);
            s.w1 += al * (s.p01 + s.p11 * u1);
            s.w0 + u1 * s.w1 + s.t0 as f64
        },
    },
    Mutant {
        name: "baseline_not_truncated",
        why: "keeps the fraction of the first timestamp instead of truncating it",
        init: |t0, srate, halftime| {
            let mut s = State::new(t0, srate, halftime);
            // A real implementation would carry the whole value. liblsl holds
            // an unsigned integer, so the fraction is lost.
            s.t0 = t0.round() as u32;
            s
        },
        step: real_step,
    },
    Mutant {
        name: "wrong_initial_covariance",
        why: "starts the covariance at 1e9 instead of 1e10",
        init: |t0, srate, halftime| {
            let mut s = State::new(t0, srate, halftime);
            s.p00 = 1e9;
            s.p11 = 1e9;
            s
        },
        step: real_step,
    },
    Mutant {
        name: "slope_starts_at_zero",
        why: "leaves the slope at zero instead of one over the rate",
        init: |t0, srate, halftime| {
            let mut s = State::new(t0, srate, halftime);
            s.w1 = 0.0;
            s
        },
        step: real_step,
    },
    Mutant {
        name: "forgetting_factor_uses_e",
        why: "uses a natural exponent instead of a power of two",
        init: |t0, srate, halftime| {
            let mut s = State::new(t0, srate, halftime);
            if srate > 0.0 {
                s.lam = (-1. / (srate * halftime)).exp();
            }
            s
        },
        step: real_step,
    },
    Mutant {
        name: "baseline_not_added_back",
        why: "leaves the baseline out of the returned value",
        init: State::new,
        step: |s, t| {
            let r = real_step(s, t);
            r - s.t0 as f64
        },
    },
];

fn run(name: &str, m: &Mutant) -> usize {
    let all = load();
    let mut caught = 0;
    for (_, j) in &all {
        let arr = j["samples"].as_array().unwrap();
        let srate = bits(j["srate_bits"].as_str().unwrap());
        let halftime = bits(j["halftime_bits"].as_str().unwrap());
        let first = bits(arr[0].as_array().unwrap()[0].as_str().unwrap());
        let mut st = (m.init)(first, srate, halftime);

        let mut differs = false;
        for pair in arr {
            let p = pair.as_array().unwrap();
            let t = bits(p[0].as_str().unwrap());
            let want = bits(p[1].as_str().unwrap());
            let got = (m.step)(&mut st, t);
            if !same(got, want) {
                differs = true;
                break;
            }
        }
        if differs {
            caught += 1;
        }
    }
    let _ = name;
    caught
}

#[test]
fn every_mutant_is_caught_by_at_least_one_sequence() {
    let all = load();
    if all.is_empty() {
        eprintln!("no sequences; skipping");
        return;
    }
    let mut uncaught = Vec::new();
    println!("mutation report over {} sequences:", all.len());
    for m in MUTANTS {
        let caught = run(m.name, m);
        println!(
            "  {:32} caught by {}/{} sequences",
            m.name,
            caught,
            all.len()
        );
        if caught == 0 {
            uncaught.push(format!("{}: {}", m.name, m.why));
        }
    }
    assert!(
        uncaught.is_empty(),
        "these changes slipped past every sequence. The set has a hole, and the \
         fix is a new sequence.\n  {}",
        uncaught.join("\n  ")
    );
}

/// The control. The real filter must match every sequence.
#[test]
fn the_real_filter_is_not_caught() {
    let all = load();
    if all.is_empty() {
        return;
    }
    let real = Mutant {
        name: "real",
        why: "",
        init: State::new,
        step: real_step,
    };
    assert_eq!(
        run("real", &real),
        0,
        "the reference copy of the filter differs from the oracle, so the \
         mutation report means nothing"
    );
    println!("the reference copy matches all {} sequences", all.len());
}
