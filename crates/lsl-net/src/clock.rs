//! The clock offset that an inlet measures against its outlet. SPEC.md 3.
//!
//! A thread sends a burst of probes, keeps the estimate with the lowest
//! round-trip time, and publishes it. The post-processor reads that value
//! through [`OffsetSource`], so the arithmetic stays in `lsl-time` where a test
//! can drive it with fixed numbers.

use lsl_proto::timesync::{self, Burst};
use lsl_time::OffsetSource;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// The cadence below is the default. A configuration file changes it, and both
// libraries then move together. `crates/lsl-net/src/config.rs` reads the file.

/// How long between two bursts. `src/api_config.cpp:314`.
pub fn update_interval() -> Duration {
    Duration::from_secs_f64(crate::config::get().time_update_interval)
}
/// How many probes one burst sends. `src/api_config.cpp:316`.
pub fn probe_count() -> usize {
    crate::config::get().time_probe_count as usize
}
/// How long between two probes of one burst. `src/api_config.cpp:317`.
pub fn probe_interval() -> Duration {
    Duration::from_secs_f64(crate::config::get().time_probe_interval)
}
/// How long a burst waits for the last answer. `src/api_config.cpp:318`.
pub fn probe_max_rtt() -> Duration {
    Duration::from_secs_f64(crate::config::get().time_probe_max_rtt)
}
/// How many answers a burst needs before it publishes. `src/api_config.cpp:315`.
pub fn update_min_probes() -> usize {
    crate::config::get().time_update_min_probes as usize
}

/// The published result of the most recent burst.
#[derive(Debug)]
pub struct ClockState {
    /// The offset, as `f64` bits. `u64::MAX` means no measurement yet.
    offset_bits: AtomicU64,
    /// The round-trip time of the kept estimate, as `f64` bits.
    uncertainty_bits: AtomicU64,
    /// Set once when a burst first publishes a value.
    have: AtomicBool,
    /// Set when the connection reset. The post-processor clears it.
    reset: AtomicBool,
    /// Tells the thread to stop.
    stop: AtomicBool,
    /// How many bursts have published.
    updates: AtomicU64,
}

const NO_VALUE: u64 = u64::MAX;

impl Default for ClockState {
    fn default() -> Self {
        ClockState {
            offset_bits: AtomicU64::new(NO_VALUE),
            uncertainty_bits: AtomicU64::new(NO_VALUE),
            have: AtomicBool::new(false),
            reset: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            updates: AtomicU64::new(0),
        }
    }
}

impl ClockState {
    /// The most recent offset, or `None` before the first burst finishes.
    pub fn offset(&self) -> Option<f64> {
        match self.offset_bits.load(Ordering::SeqCst) {
            NO_VALUE => None,
            b => Some(f64::from_bits(b)),
        }
    }

    /// The round-trip time of the kept estimate. liblsl calls this the
    /// uncertainty (`src/time_receiver.cpp:204`).
    pub fn uncertainty(&self) -> Option<f64> {
        match self.uncertainty_bits.load(Ordering::SeqCst) {
            NO_VALUE => None,
            b => Some(f64::from_bits(b)),
        }
    }

    /// How many bursts have published a value.
    pub fn updates(&self) -> u64 {
        self.updates.load(Ordering::SeqCst)
    }

    /// Wait until a first measurement arrives.
    pub fn wait(&self, timeout: Duration) -> Option<f64> {
        let end = Instant::now() + timeout;
        while Instant::now() < end {
            if let Some(v) = self.offset() {
                return Some(v);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.offset()
    }

    /// Record that the connection reset, so the post-processor clears its fit.
    pub fn mark_reset(&self) {
        self.reset.store(true, Ordering::SeqCst);
    }

    /// Stop the probe thread.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    fn publish(&self, offset: f64, uncertainty: f64) {
        self.offset_bits.store(offset.to_bits(), Ordering::SeqCst);
        self.uncertainty_bits
            .store(uncertainty.to_bits(), Ordering::SeqCst);
        self.have.store(true, Ordering::SeqCst);
        self.updates.fetch_add(1, Ordering::SeqCst);
    }
}

/// Start a thread that keeps the offset up to date.
///
/// The thread sends one burst every [`update_interval()`]. A burst that collects
/// fewer than [`update_min_probes()`] answers publishes nothing, so a lost packet
/// leaves the previous value in place instead of replacing it with a guess.
pub fn start(target: std::net::SocketAddr, state: Arc<ClockState>, rng: Arc<Mutex<u64>>) {
    std::thread::spawn(move || {
        // The probe socket has to speak the protocol of the target.
        let any: std::net::IpAddr = if target.is_ipv6() {
            std::net::Ipv6Addr::UNSPECIFIED.into()
        } else {
            std::net::Ipv4Addr::UNSPECIFIED.into()
        };
        let sock = match UdpSocket::bind((any, 0)) {
            Ok(s) => s,
            Err(_) => return,
        };
        let _ = sock.set_read_timeout(Some(Duration::from_millis(40)));

        while !state.stop.load(Ordering::SeqCst) {
            // A new identifier for each burst, so a late answer from an earlier burst
            // is dropped. SPEC.md 3.4.
            let wave_id = {
                let mut r = rng.lock().unwrap();
                *r ^= *r << 13;
                *r ^= *r >> 7;
                *r ^= *r << 17;
                (*r as u32 as i32).abs()
            };
            let mut burst = Burst::new(wave_id);
            let burst_end =
                Instant::now() + probe_interval() * probe_count() as u32 + probe_max_rtt();

            for i in 0..probe_count() {
                if state.stop.load(Ordering::SeqCst) {
                    return;
                }
                let t0 = crate::clock();
                let msg = timesync::build_probe(wave_id, t0);
                let _ = sock.send_to(&msg, target);

                // Collect whatever arrives before the next probe goes out.
                let slot_end = Instant::now() + probe_interval();
                let mut buf = [0u8; 4096];
                while Instant::now() < slot_end {
                    match sock.recv_from(&mut buf) {
                        Ok((n, _)) => {
                            let t3 = crate::clock();
                            if let Some(a) = timesync::parse_answer(&buf[..n]) {
                                burst.accept(&a, t3);
                            }
                        }
                        Err(_) => break,
                    }
                }
                let _ = i;
            }

            // Drain the tail of the burst.
            let mut buf = [0u8; 4096];
            while Instant::now() < burst_end {
                match sock.recv_from(&mut buf) {
                    Ok((n, _)) => {
                        let t3 = crate::clock();
                        if let Some(a) = timesync::parse_answer(&buf[..n]) {
                            burst.accept(&a, t3);
                        }
                    }
                    Err(_) => break,
                }
            }

            if std::env::var_os("LSL_TRACE_CLOCK").is_some() {
                eprintln!(
                    "TRACE clock wave={wave_id} accepted={} offset={:?} rtt={:?}",
                    burst.accepted(),
                    burst.time_offset(),
                    burst.uncertainty()
                );
            }
            if burst.accepted() >= update_min_probes() {
                if let (Some(o), Some(u)) = (burst.time_offset(), burst.uncertainty()) {
                    state.publish(o, u);
                }
            }

            // Wait out the rest of the interval.
            let rest = update_interval()
                .saturating_sub(probe_interval() * probe_count() as u32 + probe_max_rtt());
            let wake = Instant::now() + rest;
            while Instant::now() < wake {
                if state.stop.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    });
}

/// Feeds the post-processor from a live connection.
pub struct LiveSource {
    state: Arc<ClockState>,
    srate: f64,
}

impl LiveSource {
    /// Build a source for a stream with the given nominal rate.
    pub fn new(state: Arc<ClockState>, srate: f64) -> Self {
        LiveSource { state, srate }
    }
}

impl OffsetSource for LiveSource {
    fn correction(&mut self) -> f64 {
        // Before the first burst finishes there is no measurement, and a
        // correction of zero leaves the timestamp alone. liblsl waits instead
        // (`src/time_receiver.cpp:57-76`), and a caller that needs the wait can
        // use `ClockState::wait`.
        self.state.offset().unwrap_or(0.0)
    }

    fn srate(&mut self) -> f64 {
        self.srate
    }

    fn was_reset(&mut self) -> bool {
        self.state.reset.swap(false, Ordering::SeqCst)
    }

    fn clock(&mut self) -> f64 {
        crate::clock()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_state_holds_no_measurement() {
        let s = ClockState::default();
        assert_eq!(s.offset(), None);
        assert_eq!(s.uncertainty(), None);
        assert_eq!(s.updates(), 0);
    }

    #[test]
    fn a_published_value_comes_back() {
        let s = ClockState::default();
        s.publish(-1.25, 0.004);
        assert_eq!(s.offset(), Some(-1.25));
        assert_eq!(s.uncertainty(), Some(0.004));
        assert_eq!(s.updates(), 1);
    }

    #[test]
    fn a_reset_reads_once() {
        let s = Arc::new(ClockState::default());
        let mut src = LiveSource::new(Arc::clone(&s), 100.0);
        assert!(!src.was_reset());
        s.mark_reset();
        assert!(src.was_reset(), "the first read reports the reset");
        assert!(!src.was_reset(), "a second read must not report it again");
    }

    #[test]
    fn a_missing_measurement_leaves_a_timestamp_alone() {
        let s = Arc::new(ClockState::default());
        let mut src = LiveSource::new(s, 100.0);
        assert_eq!(src.correction(), 0.0);
    }
}
