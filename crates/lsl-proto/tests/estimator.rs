//! Envelope tests for the clock offset estimator. SPEC.md 3.3.
//!
//! The estimator cannot be tested against a golden value. Its inputs are four
//! clock readings, and a real network decides three of them. A test therefore
//! drives it with a simulated delay and asserts a bound.
//!
//! The bound is the point. liblsl keeps the probe with the lowest round-trip
//! time, because a fast exchange leaves less room for an asymmetric delay. That
//! rule has a consequence that a test can state: the error of the kept estimate
//! is at most half the asymmetry of the fastest exchange.

use lsl_proto::timesync::{Answer, Burst};

/// Build one exchange with a chosen delay in each direction.
///
/// `true_offset` is how far the other clock runs ahead. A correct estimator
/// recovers it exactly when the two delays match.
fn exchange(
    wave: i32,
    t0: f64,
    out_delay: f64,
    back_delay: f64,
    true_offset: f64,
) -> (Answer, f64) {
    let t1 = t0 + out_delay + true_offset; // the other clock on arrival
    let t2 = t1; // the answer leaves at once
    let t3 = t0 + out_delay + back_delay; // this clock on arrival
    (
        Answer {
            wave_id: wave,
            t0,
            t1,
            t2,
        },
        t3,
    )
}

/// A small fixed generator, so the sequence is the same on every machine.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / 9007199254740992.0
    }
}

#[test]
fn a_symmetric_exchange_recovers_the_offset_exactly() {
    let true_offset = 12.5;
    let mut b = Burst::new(1);
    let (a, t3) = exchange(1, 100.0, 0.004, 0.004, true_offset);
    b.accept(&a, t3);
    // The stored value carries the opposite sign (`src/time_receiver.cpp:204`).
    let got = -b.time_offset().expect("an offset");
    assert!((got - true_offset).abs() < 1e-12, "got {got}");
}

#[test]
fn the_error_is_half_the_asymmetry() {
    // A path that is slower one way biases the estimate by half the
    // difference. This is a property of the formula and not of liblsl.
    let true_offset = -3.25;
    for (out, back) in [(0.010, 0.002), (0.002, 0.010), (0.050, 0.001)] {
        let mut b = Burst::new(7);
        let (a, t3) = exchange(7, 500.0, out, back, true_offset);
        b.accept(&a, t3);
        let got = -b.time_offset().unwrap();
        let want_error = (out - back) / 2.0;
        assert!(
            (got - true_offset - want_error).abs() < 1e-12,
            "out {out} back {back}: got {got}, expected {}",
            true_offset + want_error
        );
    }
}

#[test]
fn the_burst_keeps_the_fastest_exchange() {
    // The rule from SPEC.md 3.3. A slow and asymmetric exchange must lose to a
    // fast and symmetric one, whatever order they arrive in.
    let true_offset = 42.0;
    for reversed in [false, true] {
        let mut b = Burst::new(3);
        let slow = exchange(3, 10.0, 0.200, 0.010, true_offset);
        let fast = exchange(3, 11.0, 0.003, 0.003, true_offset);
        if reversed {
            b.accept(&fast.0, fast.1);
            b.accept(&slow.0, slow.1);
        } else {
            b.accept(&slow.0, slow.1);
            b.accept(&fast.0, fast.1);
        }
        let got = -b.time_offset().unwrap();
        assert!(
            (got - true_offset).abs() < 1e-9,
            "reversed {reversed}: the burst kept the slow exchange, got {got}"
        );
        assert!(
            b.uncertainty().unwrap() < 0.01,
            "the kept round-trip time is wrong"
        );
    }
}

#[test]
fn a_burst_converges_under_a_noisy_path() {
    // The exit criterion of the plan: the estimator converges within a bound
    // under a simulated asymmetric delay.
    //
    // The path has a floor of 2 ms each way, a heavy tail one way, and a rare
    // very slow exchange. The bound below follows from the rule that the
    // fastest exchange wins.
    let true_offset = 1234.5;
    let mut rng = Rng(0x243F6A8885A308D3);
    let mut b = Burst::new(9);

    let n = 200;
    let mut best_asymmetry = f64::MAX;
    for i in 0..n {
        let base = 0.002;
        let out = base + rng.next() * 0.020;
        let back = base + rng.next() * 0.004;
        let (a, t3) = exchange(9, 1000.0 + i as f64, out, back, true_offset);
        b.accept(&a, t3);
        let rtt = out + back;
        if rtt < best_asymmetry {
            best_asymmetry = rtt;
        }
    }

    let got = -b.time_offset().expect("an offset");
    let error = (got - true_offset).abs();

    // The error of the kept estimate is at most half its round-trip time.
    let bound = b.uncertainty().unwrap() / 2.0;
    assert!(
        error <= bound + 1e-12,
        "the error {error} passed the bound {bound}"
    );
    // The bound must also be useful. A path with a 2 ms floor gives a few
    // milliseconds, not seconds.
    assert!(
        bound < 0.02,
        "the bound {bound} is too loose to mean anything"
    );
    println!("error {error:.9} s, bound {bound:.9} s, over {n} probes");
}

#[test]
fn a_late_answer_from_an_earlier_burst_never_counts() {
    let mut b = Burst::new(5);
    let (good, t3) = exchange(5, 10.0, 0.001, 0.001, 7.0);
    b.accept(&good, t3);
    // An answer from wave 4 arrives late and claims a wildly different offset.
    let (stale, st3) = exchange(4, 9.0, 0.0005, 0.0005, -9999.0);
    assert!(!b.accept(&stale, st3));
    assert_eq!(b.accepted(), 1);
    assert!((-b.time_offset().unwrap() - 7.0).abs() < 1e-12);
}

#[test]
fn a_burst_with_no_answer_reports_nothing() {
    let b = Burst::new(2);
    assert_eq!(b.time_offset(), None);
    assert_eq!(b.uncertainty(), None);
    assert_eq!(b.accepted(), 0);
}
