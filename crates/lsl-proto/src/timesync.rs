//! Time synchronization over UDP. SPEC.md 3.
//!
//! The estimator is a pure function of four timestamps, so a test drives it
//! with fixed numbers and no socket.

/// Build a time probe. SPEC.md 3.1, `src/time_receiver.cpp:136`.
///
/// liblsl writes the values with 16 digits of precision.
pub fn build_probe(wave_id: i32, t0: f64) -> Vec<u8> {
    format!("LSL:timedata\r\n{wave_id} {t0:.16}\r\n").into_bytes()
}

/// A parsed probe, as an outlet reads it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Probe {
    /// The identifier of the current burst.
    pub wave_id: i32,
    /// The sender clock at the moment of the request.
    pub t0: f64,
}

/// Read a time probe.
pub fn parse_probe(buf: &[u8]) -> Option<Probe> {
    let text = String::from_utf8_lossy(buf);
    let mut lines = text.split('\n');
    if lines.next()?.trim_end_matches('\r').trim() != "LSL:timedata" {
        return None;
    }
    let mut parts = lines.next()?.split_whitespace();
    Some(Probe {
        wave_id: parts.next()?.parse().ok()?,
        t0: parts.next()?.parse().ok()?,
    })
}

/// Build the answer to a probe. SPEC.md 3.2.
///
/// The answer starts with a space. `t1` is the clock on arrival, and `t2` is
/// the clock at the moment of the answer.
pub fn build_answer(wave_id: i32, t0: f64, t1: f64, t2: f64) -> Vec<u8> {
    format!(" {wave_id} {t0:.16} {t1:.16} {t2:.16}").into_bytes()
}

/// A parsed answer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Answer {
    /// The burst identifier that came back.
    pub wave_id: i32,
    /// The sender clock at the moment of the request.
    pub t0: f64,
    /// The other clock on arrival.
    pub t1: f64,
    /// The other clock at the moment of the answer.
    pub t2: f64,
}

/// Read an answer to a probe.
pub fn parse_answer(buf: &[u8]) -> Option<Answer> {
    let text = String::from_utf8_lossy(buf);
    let mut parts = text.split_whitespace();
    Some(Answer {
        wave_id: parts.next()?.parse().ok()?,
        t0: parts.next()?.parse().ok()?,
        t1: parts.next()?.parse().ok()?,
        t2: parts.next()?.parse().ok()?,
    })
}

/// One round-trip measurement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimate {
    /// The round-trip time.
    pub rtt: f64,
    /// The clock offset, before the sign change.
    pub offset: f64,
}

/// Compute one estimate from four timestamps. SPEC.md 3.3.
///
/// `src/time_receiver.cpp:170-178`.
pub fn estimate(a: &Answer, t3: f64) -> Estimate {
    Estimate {
        rtt: (t3 - a.t0) - (a.t2 - a.t1),
        offset: ((a.t1 - a.t0) + (a.t2 - t3)) / 2.0,
    }
}

/// Collects the estimates of one burst and keeps the best one.
///
/// liblsl keeps the estimate with the lowest round-trip time and discards the
/// others (`src/time_receiver.cpp:190-205`). A fast exchange leaves less room
/// for an asymmetric delay.
#[derive(Debug, Clone)]
pub struct Burst {
    wave_id: i32,
    best: Option<Estimate>,
    seen: usize,
}

impl Burst {
    /// Start a burst with an identifier.
    pub fn new(wave_id: i32) -> Self {
        Burst {
            wave_id,
            best: None,
            seen: 0,
        }
    }

    /// The identifier of this burst.
    pub fn wave_id(&self) -> i32 {
        self.wave_id
    }

    /// The number of answers that this burst accepted.
    pub fn accepted(&self) -> usize {
        self.seen
    }

    /// Take one answer.
    ///
    /// An answer from another burst is dropped. SPEC.md 3.4,
    /// `src/time_receiver.cpp:166`.
    pub fn accept(&mut self, a: &Answer, t3: f64) -> bool {
        if a.wave_id != self.wave_id {
            return false;
        }
        let e = estimate(a, t3);
        self.seen += 1;
        match self.best {
            Some(b) if b.rtt <= e.rtt => {}
            _ => self.best = Some(e),
        }
        true
    }

    /// The offset that this burst produced.
    ///
    /// liblsl stores the negative of the computed offset
    /// (`src/time_receiver.cpp:204`).
    pub fn time_offset(&self) -> Option<f64> {
        self.best.map(|e| -e.offset)
    }

    /// The round-trip time of the best estimate. liblsl calls it the
    /// uncertainty (`src/time_receiver.cpp:204`).
    pub fn uncertainty(&self) -> Option<f64> {
        self.best.map(|e| e.rtt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_probe_round_trips() {
        let bytes = build_probe(12345, 1.5);
        let p = parse_probe(&bytes).expect("a probe");
        assert_eq!(p.wave_id, 12345);
        assert!((p.t0 - 1.5).abs() < 1e-12);
    }

    #[test]
    fn an_answer_starts_with_a_space() {
        let bytes = build_answer(7, 1.0, 2.0, 3.0);
        assert_eq!(bytes[0], b' ');
        let a = parse_answer(&bytes).expect("an answer");
        assert_eq!(a.wave_id, 7);
    }

    #[test]
    fn the_captured_answer_parses() {
        // From captures/timeprobe.json.
        let raw = b" 12345 7958.920835802 7958.921048403 7958.921142721";
        let a = parse_answer(raw).expect("an answer");
        assert_eq!(a.wave_id, 12345);
        assert!((a.t1 - 7958.921048403).abs() < 1e-9);
    }

    #[test]
    fn a_symmetric_exchange_shows_the_offset() {
        // The other clock runs 10 seconds ahead. The path takes 1 second each
        // way, so the estimator must find the offset with no bias.
        let a = Answer {
            wave_id: 1,
            t0: 0.0,
            t1: 11.0,
            t2: 11.0,
        };
        let e = estimate(&a, 2.0);
        assert!((e.rtt - 2.0).abs() < 1e-12);
        assert!((e.offset - 10.0).abs() < 1e-12);
    }

    #[test]
    fn the_burst_keeps_the_lowest_round_trip_time() {
        let mut b = Burst::new(9);
        // A slow exchange first, then a fast one.
        b.accept(
            &Answer {
                wave_id: 9,
                t0: 0.0,
                t1: 15.0,
                t2: 15.0,
            },
            10.0,
        );
        b.accept(
            &Answer {
                wave_id: 9,
                t0: 0.0,
                t1: 10.5,
                t2: 10.5,
            },
            1.0,
        );
        assert_eq!(b.accepted(), 2);
        assert!((b.uncertainty().unwrap() - 1.0).abs() < 1e-12);
        assert!((b.time_offset().unwrap() + 10.0).abs() < 1e-12);
    }

    #[test]
    fn a_late_answer_from_an_earlier_burst_is_dropped() {
        let mut b = Burst::new(9);
        assert!(!b.accept(
            &Answer {
                wave_id: 8,
                t0: 0.0,
                t1: 1.0,
                t2: 1.0
            },
            2.0
        ));
        assert_eq!(b.accepted(), 0);
        assert_eq!(b.time_offset(), None);
    }

    #[test]
    fn the_stored_offset_carries_the_opposite_sign() {
        let mut b = Burst::new(1);
        b.accept(
            &Answer {
                wave_id: 1,
                t0: 0.0,
                t1: 5.0,
                t2: 5.0,
            },
            0.0,
        );
        // The computed offset is 5. liblsl stores minus 5.
        assert!((b.time_offset().unwrap() + 5.0).abs() < 1e-12);
    }
}
