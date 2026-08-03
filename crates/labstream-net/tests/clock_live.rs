//! The clock offset, measured against a live outlet.
//!
//! These tests open real sockets on the loopback interface, so they belong to
//! the reporting tier and not to the merge gate. A failure here can come from
//! the machine.
//!
//! The value they check is one that a unit test cannot reach: the offset that
//! two processes measure between their clocks. `labstream-time` already proves the
//! arithmetic against golden sequences. This proves the wiring.

use labstream_net::{Inlet, Outlet, StreamInfo};
use labstream_wire::{Format, Sample, Value};
use std::time::Duration;

fn stream(name: &str) -> StreamInfo {
    let mut i = StreamInfo::new(name, "ClockTest", 2, Format::Float32, 100.0);
    i.source_id = format!("{name}_src");
    i
}

/// Open an outlet and an inlet against each other on this machine.
fn pair(name: &str) -> Option<(Outlet, Inlet)> {
    let outlet = Outlet::new(stream(name)).ok()?;
    let info = outlet.info().clone();
    let inlet = Inlet::open(&info, Duration::from_secs(5)).ok()?;
    Some((outlet, inlet))
}

#[test]
fn the_offset_between_two_local_clocks_is_small() {
    // Both processes read the same clock, so the measured offset is the
    // asymmetry of the loopback path and nothing more.
    //
    // A wrong clock origin shows up here at once. An earlier version measured
    // from the start of the process, and the offset was then the age of the
    // machine: 31,426 seconds on the first live run.
    let (_outlet, inlet) = match pair("ClkLive1") {
        Some(p) => p,
        None => {
            eprintln!("no local network; skipping");
            return;
        }
    };

    let got = inlet.wait_for_time_correction(Duration::from_secs(8));
    let offset = match got {
        Some(v) => v,
        None => {
            eprintln!("no measurement inside the timeout; skipping");
            return;
        }
    };
    let rtt = inlet.time_uncertainty().unwrap_or(f64::MAX);

    println!("offset {offset:+.9} s, round-trip {:.6} ms", rtt * 1000.0);

    // One second is enormous for two processes on one machine, and far below
    // the age of any machine. This bound catches a wrong origin without
    // failing on a slow loopback.
    assert!(
        offset.abs() < 1.0,
        "the offset is {offset} s. A value near the age of the machine means \
         the clock does not share its origin with liblsl."
    );
    // The error of the kept estimate is at most half its round-trip time.
    assert!(
        offset.abs() <= rtt / 2.0 + 0.001,
        "the offset {offset} passed the bound from its round-trip time {rtt}"
    );
}

#[test]
fn clock_sync_moves_a_timestamp_by_the_measured_offset() {
    let (outlet, mut inlet) = match pair("ClkLive2") {
        Some(p) => p,
        None => {
            eprintln!("no local network; skipping");
            return;
        }
    };
    if inlet
        .wait_for_time_correction(Duration::from_secs(8))
        .is_none()
    {
        eprintln!("no measurement inside the timeout; skipping");
        return;
    }
    let offset = inlet.time_correction().expect("an offset");

    // Push one sample with a timestamp that no clock produced, so the change
    // is visible.
    let sample = Sample {
        timestamp: 1000.0,
        values: vec![Value::F32(1.0), Value::F32(2.0)],
    };
    outlet.push(&sample);

    inlet.set_postprocessing(labstream_time::flags::CLOCKSYNC);
    let got = inlet
        .pull(Duration::from_secs(5))
        .expect("a pull")
        .expect("a sample");

    let moved = got.timestamp - 1000.0;
    println!("offset {offset:+.9}, the timestamp moved {moved:+.9}");
    assert!(
        (moved - offset).abs() < 1e-6,
        "the stage moved the timestamp by {moved} and the offset is {offset}"
    );
}

#[test]
fn no_stage_leaves_a_timestamp_alone() {
    let (outlet, mut inlet) = match pair("ClkLive3") {
        Some(p) => p,
        None => {
            eprintln!("no local network; skipping");
            return;
        }
    };
    let sample = Sample {
        timestamp: 1234.5,
        values: vec![Value::F32(1.0), Value::F32(2.0)],
    };
    outlet.push(&sample);

    let got = inlet
        .pull(Duration::from_secs(5))
        .expect("a pull")
        .expect("a sample");
    // The default applies no stage, so the value is the one that went out.
    assert_eq!(got.timestamp.to_bits(), 1234.5f64.to_bits());
}

/// A chunk arrives with only its first timestamp on the wire.
///
/// SPEC.md 8.1 and 9.2. Every later sample carries the deduced tag, and the
/// reader rebuilds the value from the rate.
///
/// This case had no test until the C ABI produced one. Every earlier test
/// pushed samples one at a time, and a single push always carries a timestamp,
/// so the reconstruction never ran.
#[test]
fn a_deduced_timestamp_is_rebuilt_from_the_rate() {
    let srate = 100.0;
    let mut info = StreamInfo::new("DeducedTest", "T", 2, Format::Float32, srate);
    info.source_id = "deduced_src".into();
    let outlet = match Outlet::new(info) {
        Ok(o) => o,
        Err(_) => {
            eprintln!("no local network; skipping");
            return;
        }
    };
    let mut inlet = match Inlet::open(outlet.info(), Duration::from_secs(5)) {
        Ok(i) => i,
        Err(_) => {
            eprintln!("could not open; skipping");
            return;
        }
    };

    // The first sample carries a timestamp. The rest ask the reader to derive
    // one, exactly as a chunk does.
    let first = 5000.0;
    outlet.push(&Sample {
        timestamp: first,
        values: vec![Value::F32(0.0), Value::F32(0.0)],
    });
    for k in 1..5 {
        outlet.push(&Sample {
            timestamp: labstream_wire::DEDUCED_TIMESTAMP,
            values: vec![Value::F32(k as f32), Value::F32(0.0)],
        });
    }

    let mut got = Vec::new();
    for _ in 0..5 {
        match inlet.pull(Duration::from_secs(3)) {
            Ok(Some(s)) => got.push(s.timestamp),
            _ => break,
        }
    }
    assert_eq!(got.len(), 5, "not every sample arrived");
    for (k, t) in got.iter().enumerate() {
        let want = first + k as f64 / srate;
        assert!(
            (t - want).abs() < 1e-9,
            "sample {k} carries {t} and the rate gives {want}"
        );
    }
    println!("rebuilt {got:?}");
}
