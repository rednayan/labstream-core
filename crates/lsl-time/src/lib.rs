//! Timestamp post-processing for LSL. SPEC.md 8.5.
//!
//! Three stages run in a fixed order: clock sync, then jitter removal, then
//! monotonic clamping (`src/time_postprocessor.cpp:60-105`).
//!
//! # Float discipline
//!
//! The jitter filter is a recursive least squares fit. Every operation here
//! keeps the order of the C++ source, because a different order gives a
//! different result in the last bits.
//!
//! Three rules hold for every line of the filter:
//!
//! 1. Do not reassociate an expression to make it read better.
//! 2. Do not use `mul_add`, because the source uses a separate multiply and add.
//! 3. Keep the integer baseline subtraction.
//!
//! liblsl builds with plain IEEE semantics, and no fast-math flag appears in
//! its build configuration. Bit-exact agreement is therefore a target. It is
//! not a promise across every platform.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

/// The default half-time of the jitter filter, in seconds.
///
/// `src/api_config.cpp:325` reads it as a `float`.
pub const DEFAULT_SMOOTHING_HALFTIME: f64 = 90.0;

/// How many samples pass between two clock offset queries.
///
/// `src/time_postprocessor.cpp:18`.
pub const SAMPLES_BETWEEN_CLOCKSYNCS: u8 = 50;

/// The shortest time between two clock offset queries, in seconds.
///
/// `src/time_postprocessor.cpp:80` adds this to the clock.
pub const MIN_CLOCKSYNC_INTERVAL: f64 = 0.5;

/// The post-processing stages, as a bit set.
///
/// The values match `include/lsl/common.h:103-126`.
pub mod flags {
    /// Apply no stage. This is the default.
    pub const NONE: u32 = 0;
    /// Add the measured clock offset.
    pub const CLOCKSYNC: u32 = 1;
    /// Remove jitter with a recursive least squares fit.
    pub const DEJITTER: u32 = 2;
    /// Never let a timestamp go backward.
    pub const MONOTONIZE: u32 = 4;
    /// Guard the state with a lock.
    pub const THREADSAFE: u32 = 8;
    /// Every stage.
    pub const ALL: u32 = 1 | 2 | 4 | 8;
}

/// The recursive least squares filter that removes jitter.
///
/// The filter fits `t = w0 + w1 * n` over the sample index `n`. SPEC.md 8.5.
#[derive(Debug, Clone, PartialEq)]
pub struct Dejitterer {
    /// The first timestamp, truncated to a whole number.
    ///
    /// liblsl stores this in an unsigned integer
    /// (`src/time_postprocessor.h:18`). The truncation is part of the
    /// behavior, because the value comes back at the end of every call.
    pub t0: u32,
    /// The number of samples since `t0`.
    pub samples_since_t0: u32,
    /// The intercept of the fit.
    pub w0: f64,
    /// The slope of the fit.
    pub w1: f64,
    /// The inverse covariance element P00.
    pub p00: f64,
    /// The inverse covariance element P11.
    pub p11: f64,
    /// The off-diagonal inverse covariance element.
    pub p01: f64,
    /// The forgetting factor.
    pub lam: f64,
}

impl Default for Dejitterer {
    /// The state before the first sample.
    ///
    /// `src/time_postprocessor.h:20-26` gives every initial value.
    fn default() -> Self {
        Dejitterer {
            t0: 0,
            samples_since_t0: 0,
            w0: 0.0,
            w1: 0.0,
            p00: 1e10,
            p11: 1e10,
            p01: 0.0,
            lam: 0.0,
        }
    }
}

impl Dejitterer {
    /// Build a filter for a stream.
    ///
    /// `src/time_postprocessor.cpp:104-110`. A rate of zero or less leaves the
    /// forgetting factor at zero, and the filter then returns every timestamp
    /// unchanged.
    pub fn new(t0: f64, srate: f64, halftime: f64) -> Self {
        let mut d = Dejitterer {
            t0: t0 as u32,
            ..Default::default()
        };
        if srate > 0.0 {
            d.w1 = 1. / srate;
            d.lam = 2f64.powf(-1. / (srate * halftime));
        }
        d
    }

    /// True once a first timestamp has set the baseline.
    ///
    /// liblsl tests the baseline against zero
    /// (`src/time_postprocessor.h:35`). A first timestamp below 1.0 therefore
    /// truncates to zero, and the filter reads as uninitialized.
    pub fn is_initialized(&self) -> bool {
        self.t0 != 0
    }

    /// True when the filter changes a timestamp.
    pub fn smoothing_applicable(&self) -> bool {
        self.lam > 0.0
    }

    /// Take one timestamp and return the fitted value.
    ///
    /// The operation order matches `src/time_postprocessor.cpp:112-131` line
    /// for line. Do not tidy this function.
    pub fn dejitter(&mut self, t: f64) -> f64 {
        if !self.smoothing_applicable() {
            return t;
        }

        // Remove the baseline for numerical accuracy.
        let t = t - self.t0 as f64;

        let u1 = self.samples_since_t0 as f64;
        self.samples_since_t0 = self.samples_since_t0.wrapping_add(1);

        let pi0 = self.p00 + u1 * self.p01;
        let pi1 = self.p01 + u1 * self.p11;
        let al = t - (self.w0 + u1 * self.w1);
        let g_inv = 1. / (self.lam + pi0 + pi1 * u1);
        let il_ = 1. / self.lam;

        self.p00 = il_ * (self.p00 - pi0 * pi0 * g_inv);
        self.p01 = il_ * (self.p01 - pi0 * pi1 * g_inv);
        self.p11 = il_ * (self.p11 - pi1 * pi1 * g_inv);
        self.w0 += al * (self.p00 + self.p01 * u1);
        self.w1 += al * (self.p01 + self.p11 * u1);

        self.w0 + u1 * self.w1 + self.t0 as f64
    }

    /// Move the sample counter forward for samples that never arrived.
    ///
    /// `src/time_postprocessor.cpp:133-135`.
    pub fn skip_samples(&mut self, skipped: u32) {
        self.samples_since_t0 = self.samples_since_t0.wrapping_add(skipped);
    }
}

/// Reads the clock offset that the time sync channel measured.
///
/// The post-processor asks for a value instead of reading a clock, so a test
/// drives it with fixed numbers.
pub trait OffsetSource {
    /// The current offset between the two clocks.
    fn correction(&mut self) -> f64;
    /// The nominal rate of the stream.
    fn srate(&mut self) -> f64;
    /// True when the connection reset since the last call.
    fn was_reset(&mut self) -> bool;
    /// The local clock, in seconds.
    fn clock(&mut self) -> f64;
}

/// Applies the three stages to every timestamp. SPEC.md 8.5.
pub struct PostProcessor {
    options: u32,
    halftime: f64,
    dejitter: Dejitterer,
    samples_since_last_clocksync: u8,
    next_query_time: f64,
    last_offset: f64,
    last_value: f64,
}

impl PostProcessor {
    /// Build a post-processor with no stage enabled.
    ///
    /// `src/time_postprocessor.cpp:21-27` starts the last value at the lowest
    /// possible number, so the first sample always passes the clamp.
    pub fn new() -> Self {
        PostProcessor {
            options: flags::NONE,
            halftime: DEFAULT_SMOOTHING_HALFTIME,
            dejitter: Dejitterer::default(),
            samples_since_last_clocksync: SAMPLES_BETWEEN_CLOCKSYNCS,
            next_query_time: 0.0,
            last_offset: 0.0,
            last_value: f64::MIN,
        }
    }

    /// Set the half-time of the jitter filter.
    pub fn set_halftime(&mut self, halftime: f64) {
        self.halftime = halftime;
    }

    /// The current options.
    pub fn options(&self) -> u32 {
        self.options
    }

    /// The state of the jitter filter.
    pub fn dejitterer(&self) -> &Dejitterer {
        &self.dejitter
    }

    /// Choose the stages.
    ///
    /// A change to a stage clears the state of that stage
    /// (`src/time_postprocessor.cpp:45-58`).
    pub fn set_options(&mut self, options: u32) {
        let changed = self.options ^ options;
        if changed & flags::DEJITTER != 0 {
            self.dejitter = Dejitterer::default();
        }
        if changed & flags::MONOTONIZE != 0 {
            self.last_value = f64::MIN;
        }
        self.options = options;
    }

    /// Take one timestamp and return the processed value.
    ///
    /// The order is clock sync, then jitter removal, then the clamp
    /// (`src/time_postprocessor.cpp:60-105`).
    pub fn process(&mut self, value: f64, src: &mut impl OffsetSource) -> f64 {
        let mut value = value;

        if self.options & flags::CLOCKSYNC != 0 {
            // The offset refreshes every 50 samples, and never more than twice
            // per second.
            self.samples_since_last_clocksync = self.samples_since_last_clocksync.saturating_add(1);
            if self.samples_since_last_clocksync > SAMPLES_BETWEEN_CLOCKSYNCS
                && src.clock() > self.next_query_time
            {
                self.last_offset = src.correction();
                self.samples_since_last_clocksync = 0;
                if src.was_reset() {
                    self.last_offset = src.correction();
                    self.last_value = f64::MIN;
                    self.dejitter = Dejitterer::default();
                }
                self.next_query_time = src.clock() + MIN_CLOCKSYNC_INTERVAL;
            }
            value += self.last_offset;
        }

        if self.options & flags::DEJITTER != 0 {
            if !self.dejitter.is_initialized() {
                let srate = src.srate();
                self.dejitter = Dejitterer::new(value, srate, self.halftime);
            }
            value = self.dejitter.dejitter(value);
        }

        if self.options & flags::MONOTONIZE != 0 {
            if value < self.last_value {
                value = self.last_value;
            } else {
                self.last_value = value;
            }
        }

        value
    }

    /// Tell the filter that samples were skipped.
    pub fn skip_samples(&mut self, skipped: u32) {
        if self.options & flags::DEJITTER != 0 && self.dejitter.smoothing_applicable() {
            self.dejitter.skip_samples(skipped);
        }
    }
}

impl Default for PostProcessor {
    fn default() -> Self {
        Self::new()
    }
}

/// A source with values that a test sets by hand.
#[derive(Debug, Clone)]
pub struct FixedSource {
    /// The offset to return.
    pub offset: f64,
    /// The rate to return.
    pub srate: f64,
    /// The reset flag to return.
    pub reset: bool,
    /// The clock to return. A test moves it forward by hand.
    pub now: f64,
}

impl OffsetSource for FixedSource {
    fn correction(&mut self) -> f64 {
        self.offset
    }
    fn srate(&mut self) -> f64 {
        self.srate
    }
    fn was_reset(&mut self) -> bool {
        self.reset
    }
    fn clock(&mut self) -> f64 {
        self.now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(srate: f64) -> FixedSource {
        FixedSource {
            offset: 0.0,
            srate,
            reset: false,
            now: 1e9,
        }
    }

    #[test]
    fn no_stage_changes_a_timestamp() {
        let mut p = PostProcessor::new();
        let mut s = src(1.0);
        for t in [2.0, 3.1, 3.0, 5.0, 5.9, 7.1] {
            assert_eq!(p.process(t, &mut s), t);
        }
    }

    #[test]
    fn clock_sync_adds_the_offset() {
        // The upstream test uses these values (`testing/int/postproc.cpp`).
        let mut p = PostProcessor::new();
        p.set_options(flags::CLOCKSYNC);
        let mut s = src(1.0);
        s.offset = -50.0;
        for t in [2.0, 3.1, 3.0, 5.0, 5.9, 7.1] {
            assert!((p.process(t, &mut s) - (t - 50.0)).abs() < 1e-12);
        }
    }

    #[test]
    fn the_clamp_never_lets_a_timestamp_go_backward() {
        // The upstream test expects this exact list.
        let mut p = PostProcessor::new();
        p.set_options(flags::MONOTONIZE);
        let mut s = src(1.0);
        let input = [2.0, 3.1, 3.0, 5.0, 5.9, 7.1];
        let want = [2.0, 3.1, 3.1, 5.0, 5.9, 7.1];
        for (i, t) in input.iter().enumerate() {
            assert!(
                (p.process(*t, &mut s) - want[i]).abs() < 1e-12,
                "sample {i}"
            );
        }
    }

    #[test]
    fn a_stage_change_clears_that_state() {
        let mut p = PostProcessor::new();
        p.set_options(flags::MONOTONIZE);
        let mut s = src(1.0);
        p.process(100.0, &mut s);
        // Turning the clamp off and on again clears the running maximum.
        p.set_options(flags::NONE);
        p.set_options(flags::MONOTONIZE);
        assert_eq!(p.process(2.0, &mut s), 2.0);
    }

    #[test]
    fn a_rate_of_zero_leaves_the_filter_off() {
        let d = Dejitterer::new(1000.0, 0.0, 90.0);
        assert!(!d.smoothing_applicable());
        let mut d = d;
        assert_eq!(d.dejitter(1234.5), 1234.5);
    }

    #[test]
    fn the_baseline_truncates_to_a_whole_number() {
        // `src/time_postprocessor.h:18` holds an unsigned integer.
        let d = Dejitterer::new(1000.75, 100.0, 90.0);
        assert_eq!(d.t0, 1000);
        assert!(d.is_initialized());
    }

    #[test]
    fn a_first_timestamp_below_one_reads_as_uninitialized() {
        // A real trap. The baseline truncates to zero, and the test for an
        // initialized filter compares against zero.
        let d = Dejitterer::new(0.5, 100.0, 90.0);
        assert_eq!(d.t0, 0);
        assert!(!d.is_initialized());
    }

    #[test]
    fn the_filter_converges_on_a_clean_ramp() {
        let srate = 100.0;
        let t0 = 5000.0;
        let mut d = Dejitterer::new(t0, srate, 90.0);
        d.dejitter(t0);
        for i in 0..2000 {
            let t = t0 + i as f64 / srate;
            d.dejitter(t);
        }
        // The upstream test asserts the same two bounds.
        assert!((d.w1 - 1.0 / srate).abs() < 1e-6, "slope {}", d.w1);
        assert!(d.w0.abs() < 0.1, "intercept {}", d.w0);
    }

    #[test]
    fn the_filter_removes_a_constant_latency_from_the_slope() {
        // A constant offset moves the intercept and leaves the slope alone.
        let srate = 100.0;
        let t0 = 5000.0;
        let latency = 0.05;
        let mut d = Dejitterer::new(t0, srate, 90.0);
        d.dejitter(t0);
        for i in 0..5000 {
            d.dejitter(t0 + i as f64 / srate + latency);
        }
        assert!((d.w1 - 1.0 / srate).abs() < 1e-6);
        assert!((d.w0 - latency).abs() < 0.1, "intercept {}", d.w0);
    }

    #[test]
    fn skipped_samples_move_the_counter() {
        let mut d = Dejitterer::new(1000.0, 100.0, 90.0);
        d.dejitter(1000.0);
        let before = d.samples_since_t0;
        d.skip_samples(7);
        assert_eq!(d.samples_since_t0, before + 7);
    }

    #[test]
    fn clock_sync_refreshes_at_most_twice_per_second() {
        let mut p = PostProcessor::new();
        p.set_options(flags::CLOCKSYNC);
        let mut s = src(1.0);
        s.offset = 1.0;
        s.now = 100.0;
        // The first sample queries, because the counter starts at the limit.
        p.process(0.0, &mut s);
        s.offset = 99.0;
        // The next 50 samples must not query again.
        for _ in 0..50 {
            let got = p.process(0.0, &mut s);
            assert!((got - 1.0).abs() < 1e-12, "the offset changed too early");
        }
        // The counter is ready, but the clock has not moved.
        assert!((p.process(0.0, &mut s) - 1.0).abs() < 1e-12);
        // Move the clock past the interval.
        s.now = 101.0;
        assert!((p.process(0.0, &mut s) - 99.0).abs() < 1e-12);
    }

    #[test]
    fn a_reset_gives_the_filter_a_new_baseline() {
        // A reset means the source restarted, so the old fit describes a
        // process that no longer exists. The filter takes a new baseline from
        // the first timestamp after the reset.
        let mut p = PostProcessor::new();
        p.set_options(flags::CLOCKSYNC | flags::DEJITTER);
        let mut s = src(100.0);
        s.now = 100.0;

        p.process(5000.0, &mut s);
        assert_eq!(p.dejitterer().t0, 5000);

        // Move the counter past the limit while the clock stays put, so no
        // query fires yet.
        for i in 1..60 {
            p.process(5000.0 + i as f64 / 100.0, &mut s);
        }
        assert_eq!(p.dejitterer().t0, 5000, "no query fired yet");

        // Now the clock passes the interval and the source reports a reset.
        s.reset = true;
        s.now = 101.0;
        p.process(9000.0, &mut s);
        assert_eq!(p.dejitterer().t0, 9000, "the reset gives a new baseline");
    }

    #[test]
    fn the_clamp_state_clears_on_a_reset() {
        // The clearing is visible only when the resetting sample itself
        // carries a lower value. Any later sample sets the maximum again.
        //
        // The clock must stay put while the counter fills, so that the query
        // fires on the call that carries the low value and on no earlier one.
        let mut p = PostProcessor::new();
        p.set_options(flags::CLOCKSYNC | flags::MONOTONIZE);
        let mut s = src(100.0);
        s.now = 100.0;

        p.process(500.0, &mut s);
        for _ in 0..55 {
            p.process(500.0, &mut s);
        }
        // The counter is ready and the clock has not moved, so no query fired.
        // A lower value is still clamped.
        assert!((p.process(10.0, &mut s) - 500.0).abs() < 1e-12);

        // Move the clock past the interval and report a reset. The query now
        // fires on this call, and the clamp state clears before the clamp runs.
        s.reset = true;
        s.now = 101.0;
        assert!((p.process(10.0, &mut s) - 10.0).abs() < 1e-12);
    }
}
