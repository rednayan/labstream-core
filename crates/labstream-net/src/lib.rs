//! Sockets, timers, and interfaces for LSL.
//!
//! This is the only crate in the workspace that touches the operating system.
//! Every protocol rule lives in `labstream-proto` and `labstream-wire`, which hold no input
//! and no output.
//!
//! The split matters for testing. A protocol rule gets a unit test with a byte
//! slice. Only this crate needs a network, and only its tests are allowed to be
//! slow.

// One function in this crate needs a system call that the standard library
// does not expose. `clock()` must return the same value as `lsl_local_clock()`
// on the same machine, and that is `CLOCK_MONOTONIC`. See the note there.
#![deny(unsafe_code)]
#![deny(missing_docs)]

pub mod clock;
pub mod config;
pub mod desc;
pub mod info;
pub mod inlet;
pub mod outlet;
pub mod queue;
pub mod resolver;
pub mod xpath;

pub use clock::{ClockState, LiveSource};
pub use config::Config;
pub use desc::Node;
pub use info::StreamInfo;
pub use inlet::{build_property_query, read_fullinfo, resolve, Inlet};
pub use outlet::Outlet;
pub use queue::{Fanout, SampleQueue};
pub use resolver::ContinuousResolver;

/// The session that a stream belongs to, when no file names another.
///
/// `src/api_config.cpp:297` reads `lab.SessionID` and falls back to this word.
/// A resolver query always tests this field, so two machines with different
/// values never see each other. Read the live value from
/// [`config::get`], which honours the file.
pub const SESSION_ID: &str = "default";

/// The default base port. `src/api_config.cpp:170`.
pub const BASE_PORT: u16 = 16572;
/// The default port range. `src/api_config.cpp:171`.
pub const PORT_RANGE: u16 = 32;

/// The port that carries a multicast or broadcast query.
///
/// `src/api_config.cpp:169`. This port sits **outside** the unicast range that
/// starts at `BASE_PORT`. A resolver sends a multicast query here and a unicast
/// query to the range, so an outlet that listens only on the range never
/// answers a multicast query.
pub const MULTICAST_PORT: u16 = 16571;

/// The clock that LSL timestamps use.
///
/// liblsl returns `steady_clock::now().time_since_epoch()` in seconds
/// (`src/common.cpp:20`). On Linux that is `CLOCK_MONOTONIC`, whose origin is
/// the moment the machine booted.
///
/// # Why this is not `Instant`
///
/// The origin has to match. Two liblsl processes on one machine read the same
/// clock, and an application can push a timestamp from `lsl_local_clock()`
/// that a consumer on the same machine reads directly. No post-processing runs
/// by default (`include/lsl/common.h:103`), so nothing corrects a difference.
///
/// A first version of this function measured from the start of the process.
/// Every value was then wrong by the age of the machine. A live test against a
/// real liblsl outlet measured the error at 31,426 seconds.
///
/// `std::time::Instant` uses this same clock and keeps its value private, and
/// `/proc/uptime` counts a different one: it includes time spent suspended, so
/// it read 66,567 on a machine where `CLOCK_MONOTONIC` read 31,492. The system
/// call is the only source that agrees.
///
/// Each platform reads its own clock. `machine_clock` gives the note for the
/// platform.
pub fn clock() -> f64 {
    if let Some(seconds) = machine_clock() {
        return seconds;
    }
    // A platform with no clock above falls back to the process clock.
    // Timestamps then carry the wrong origin, and only a peer that applies the
    // clock correction reads them correctly.
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// The clock of the machine, in seconds, or `None` where there is none.
///
/// Unix reads `CLOCK_MONOTONIC`, which is what `steady_clock` reads there.
#[cfg(unix)]
fn machine_clock() -> Option<f64> {
    #[allow(unsafe_code)]
    {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `clock_gettime` writes a `timespec` through the pointer and
        // reads nothing else. The value is stack-allocated and initialized.
        let rc = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
        if rc == 0 {
            return Some(ts.tv_sec as f64 + ts.tv_nsec as f64 * 1e-9);
        }
    }
    None
}

/// The clock of the machine, in seconds, or `None` where there is none.
///
/// # Why Windows needs its own arithmetic
///
/// Windows has no `clock_gettime`. MSVC builds `steady_clock` on the
/// performance counter, and liblsl reads `steady_clock` (`src/common.cpp:20`),
/// so this function reads the same counter. The counter starts when the
/// machine starts, and every process on the machine reads one value.
///
/// MSVC divides before it multiplies, which keeps a large counter inside an
/// `i64`:
///
/// ```text
/// whole = (counter / frequency) * 1_000_000_000
/// part  = (counter % frequency) * 1_000_000_000 / frequency
/// ```
///
/// This function keeps that order. It holds the whole seconds and the
/// nanoseconds apart, because `f64` carries 53 bits and a machine that ran for
/// 104 days has more nanoseconds than that. liblsl divides the same way and
/// gives the reason (`src/common.cpp:46-48`).
///
/// The multiplication uses `i128`. A counter frequency above 9.2 GHz overflows
/// an `i64` here, and an overflow stops a program in a debug build. No counter
/// runs that fast. The wider type costs nothing and removes the case.
#[cfg(windows)]
fn machine_clock() -> Option<f64> {
    #[allow(unsafe_code)]
    {
        let mut frequency: i64 = 0;
        let mut counter: i64 = 0;
        // SAFETY: each call writes one `i64` through the pointer and reads
        // nothing else. Both values are stack-allocated and initialized.
        let ok = unsafe {
            QueryPerformanceFrequency(&mut frequency) != 0
                && QueryPerformanceCounter(&mut counter) != 0
        };
        // Windows XP and every later version always answer. A zero frequency
        // would divide by zero, so this checks it.
        if ok && frequency > 0 {
            return Some(counter_seconds(counter, frequency));
        }
    }
    None
}

/// Seconds from a performance counter and its frequency.
///
/// Only Windows calls this. Every platform compiles it, so a test of the
/// arithmetic runs everywhere. Windows is the one platform that this
/// repository does not measure against liblsl.
///
/// The caller must give a frequency above zero.
#[cfg_attr(not(windows), allow(dead_code))]
fn counter_seconds(counter: i64, frequency: i64) -> f64 {
    let seconds = counter / frequency;
    let rest = (counter % frequency) as i128;
    let nanoseconds = (rest * 1_000_000_000 / frequency as i128) as f64;
    seconds as f64 + nanoseconds * 1e-9
}

/// The clock of the machine, in seconds, or `None` where there is none.
#[cfg(not(any(unix, windows)))]
fn machine_clock() -> Option<f64> {
    None
}

// `QueryPerformanceCounter` counts from the moment the machine started, and
// `QueryPerformanceFrequency` gives the counts in one second. The frequency
// does not change while the machine runs.
#[cfg(windows)]
#[allow(unsafe_code)]
#[link(name = "kernel32")]
extern "system" {
    fn QueryPerformanceCounter(count: *mut i64) -> i32;
    fn QueryPerformanceFrequency(frequency: *mut i64) -> i32;
}

/// Build a stream instance identifier.
///
/// liblsl writes a UUID. The value is random and it changes on every restart,
/// so no test compares it. SPEC.md 12.
pub fn make_uid() -> String {
    let b = random_bytes();
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

/// Build an opaque discovery token.
///
/// liblsl builds this from `std::hash`, whose value differs between standard
/// libraries. Any value works, because the token only travels out and back.
/// SPEC.md 2.3.
pub fn make_query_id() -> String {
    let b = random_bytes();
    let mut s = String::with_capacity(16);
    for x in b.iter().take(8) {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

fn random_bytes() -> [u8; 16] {
    // The operating system supplies the bytes. This value never needs to be
    // reproducible, and it must not repeat across two processes that start at
    // the same moment.
    //
    // Read an exact count. A read to the end of this file never returns,
    // because the file never ends.
    use std::io::Read;
    let mut out = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        if f.read_exact(&mut out).is_ok() {
            return out;
        }
    }
    // A fallback that mixes the clock with the process identifier.
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let p = std::process::id() as u64;
    let mut x = t ^ (p << 32) ^ 0x9E3779B97F4A7C15;
    for b in out.iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = (x & 0xFF) as u8;
    }
    out
}

/// Every local IPv4 address on this machine.
///
/// A multicast join names an interface by its address. Joining with the
/// unspecified address lets the kernel pick one, and it can pick the loopback,
/// which carries no multicast. An outlet is then invisible to a resolver that
/// sends on a real interface.
///
/// liblsl reads the interface list with `getifaddrs`
/// (`src/netinterfaces.cpp`). This reads the routing table instead, which needs
/// no unsafe code. A machine with no readable table returns an empty list, and
/// the caller falls back to the unspecified address.
pub fn local_ipv4_addresses() -> Vec<std::net::Ipv4Addr> {
    let mut out = Vec::new();
    let text = match std::fs::read_to_string("/proc/net/fib_trie") {
        Ok(t) => t,
        Err(_) => return out,
    };
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        // A local address appears as a `/32 host LOCAL` entry, one line below
        // the address itself.
        if !line.contains("host LOCAL") {
            continue;
        }
        if i == 0 {
            continue;
        }
        let prev = lines[i - 1];
        let addr = prev.rsplit(&[' ', '|', '-'][..]).find(|t| {
            t.split('.').count() == 4 && t.chars().all(|c| c.is_ascii_digit() || c == '.')
        });
        if let Some(a) = addr.and_then(|a| a.parse::<std::net::Ipv4Addr>().ok()) {
            if !a.is_broadcast() && !out.contains(&a) {
                out.push(a);
            }
        }
    }
    out
}

/// The index of every interface on this machine, with zero at the front.
///
/// An IPv6 multicast join names an interface by index rather than by address
/// (`src/udp_server.cpp:78`, which reads `scope_id`). Index zero lets the
/// operating system pick one, and that choice can be the loopback, which
/// carries no multicast. Every real index is therefore tried as well.
///
/// The list comes from `/proc/net/if_inet6`, whose fourth column is the index
/// in hexadecimal. A machine with no readable file returns only zero.
pub fn local_interface_indexes() -> Vec<u32> {
    let mut out = vec![0u32];
    if let Ok(text) = std::fs::read_to_string("/proc/net/if_inet6") {
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            // address, index, prefix length, scope, flags, name
            let index = parts.nth(1).and_then(|v| u32::from_str_radix(v, 16).ok());
            if let Some(i) = index {
                if i != 0 && !out.contains(&i) {
                    out.push(i);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interface_list_holds_the_choice_of_the_system() {
        let idx = local_interface_indexes();
        assert_eq!(idx[0], 0, "index zero must come first");
        println!("interface indexes: {idx:?}");
    }

    #[test]
    fn a_uid_has_the_shape_that_the_predicate_wants() {
        let u = make_uid();
        let parts: Vec<&str> = u.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(
            [
                parts[0].len(),
                parts[1].len(),
                parts[2].len(),
                parts[3].len(),
                parts[4].len()
            ],
            [8, 4, 4, 4, 12]
        );
        assert!(u.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
    }

    #[test]
    fn two_uids_differ() {
        assert_ne!(make_uid(), make_uid());
    }

    #[test]
    fn the_clock_moves_forward() {
        let a = clock();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(clock() > a);
    }
}

/// The arithmetic of the Windows clock. SPEC.md 3.0.
///
/// Only Windows runs `counter_seconds`, and no measurement compares this
/// library against liblsl on Windows. These tests are therefore the only check
/// of that arithmetic, and they run on every platform.
#[cfg(test)]
mod counter {
    use super::counter_seconds;

    /// 10 MHz is the frequency of most machines. MSVC holds a separate branch
    /// for it that multiplies the counter by 100 to get nanoseconds. The two
    /// branches must give one answer.
    const TEN_MHZ: i64 = 10_000_000;

    #[test]
    fn a_whole_number_of_seconds_carries_no_remainder() {
        assert_eq!(counter_seconds(TEN_MHZ * 7, TEN_MHZ), 7.0);
        assert_eq!(counter_seconds(0, TEN_MHZ), 0.0);
    }

    #[test]
    fn a_part_of_a_second_reads_as_a_fraction() {
        // A quarter of a second at 10 MHz is 2,500,000 counts.
        assert_eq!(counter_seconds(2_500_000, TEN_MHZ), 0.25);
        // One microsecond is 10 counts.
        let one_microsecond = counter_seconds(10, TEN_MHZ);
        assert!((one_microsecond - 1e-6).abs() < 1e-15);
    }

    #[test]
    fn the_ten_megahertz_branch_of_the_source_agrees() {
        // MSVC returns `counter * 100` nanoseconds at this frequency. The
        // general form must give the same seconds.
        for counter in [1_i64, 999, 12_345_678, 8_985_600_000_000] {
            let theirs = (counter * 100) as f64 * 1e-9;
            let ours = counter_seconds(counter, TEN_MHZ);
            assert!(
                (ours - theirs).abs() < 1e-9,
                "counter {counter}: {ours} against {theirs}"
            );
        }
    }

    /// A machine that ran for 104 days holds more nanoseconds than an `f64`
    /// carries. liblsl keeps the whole seconds apart from the rest for that
    /// reason (`src/common.cpp:46-48`), and this function does the same.
    ///
    /// The whole second must stay exact, and the rest must stay inside a
    /// nanosecond. A later change that divides a whole nanosecond count by 1e9
    /// loses the second one first.
    #[test]
    fn a_long_uptime_keeps_the_microseconds() {
        let days_104 = 104 * 24 * 60 * 60; // 8,985,600 seconds
        let counter = TEN_MHZ * days_104 + 25; // and 2.5 microseconds
        let seconds = counter_seconds(counter, TEN_MHZ);
        assert_eq!(seconds.trunc(), days_104 as f64);
        // An `f64` at 8.9 million holds about 2 nanoseconds in its last bit,
        // so this is the resolution that remains after 104 days.
        let rest = seconds - days_104 as f64;
        assert!(
            (rest - 2.5e-6).abs() < 1e-8,
            "the rest reads {rest}, and 2.5 microseconds was pushed"
        );
    }

    /// A frequency that is not 10 MHz has to work as well. 3,579,545 Hz is the
    /// frequency of an older machine.
    #[test]
    fn an_odd_frequency_gives_the_right_second() {
        let frequency = 3_579_545;
        assert_eq!(counter_seconds(frequency * 3, frequency), 3.0);
        let half = counter_seconds(frequency / 2, frequency);
        assert!((half - 0.5).abs() < 1e-6, "half a second read {half}");
    }
}

#[cfg(test)]
mod clock_origin {
    /// The clock must share its origin with liblsl.
    ///
    /// liblsl reads `CLOCK_MONOTONIC` (`src/common.cpp:20`). A test that only
    /// checks the value moves forward would pass with any origin, and an origin
    /// that differs makes every timestamp wrong for a peer that applies no
    /// correction.
    #[test]
    fn the_clock_agrees_with_the_system_monotonic_clock() {
        let ours = super::clock();
        let uptime = std::fs::read_to_string("/proc/uptime").unwrap_or_default();
        let boottime: f64 = uptime
            .split_whitespace()
            .next()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);
        // The two differ by the time the machine spent suspended, so this only
        // checks the order of magnitude. The value must be an age of the
        // machine and not an age of this process.
        assert!(
            ours > 1.0,
            "the clock reads {ours}, which looks like a process age"
        );
        if boottime > 0.0 {
            assert!(
                ours <= boottime + 1.0,
                "the clock reads {ours}, past the boot clock {boottime}"
            );
        }
        println!("clock {ours:.3}, boot clock {boottime:.3}");
    }
}
