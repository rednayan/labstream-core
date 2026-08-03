//! A pushed timestamp of zero must become the current clock.
//!
//! SPEC.md 8.3. liblsl applies the rule inside the outlet
//! (`src/stream_outlet_impl.cpp:170`), so every caller gets it.
//!
//! # Why this test exists
//!
//! The rule lived only at the C boundary. Every earlier test either read a
//! timestamp out of a file or went through the C ABI, so nothing pushed a zero
//! through `Outlet::push`. An application that used this crate directly sent a
//! timestamp of zero on the wire, and **every consumer reads that as "no
//! sample"**. The values arrived and no recorder could plot them.
//!
//! A two-machine test with LabRecorder found it. This test now covers it.

use lsl_net::{Inlet, Outlet, StreamInfo};
use lsl_wire::{Format, Sample, Value};
use std::time::Duration;

fn stream(name: &str) -> StreamInfo {
    let mut i = StreamInfo::new(name, "Test", 2, Format::Float32, 100.0);
    i.source_id = format!("{name}_src");
    i
}

#[test]
fn a_pushed_zero_becomes_the_current_clock() {
    let outlet = Outlet::new(stream("ZeroStamp")).expect("an outlet");
    let mut inlet = Inlet::open(outlet.info(), Duration::from_secs(5)).expect("an inlet");
    assert!(outlet.wait_for_consumers(Duration::from_secs(5)));
    std::thread::sleep(Duration::from_millis(200));

    let before = lsl_net::clock();
    outlet.push(&Sample {
        timestamp: 0.0,
        values: vec![Value::F32(1.0), Value::F32(2.0)],
    });
    let after = lsl_net::clock();

    let got = inlet
        .pull(Duration::from_secs(5))
        .expect("a pull")
        .expect("a sample");
    assert_eq!(got.values, vec![Value::F32(1.0), Value::F32(2.0)]);
    assert!(
        got.timestamp != 0.0,
        "a timestamp of zero reaches the consumer as no sample"
    );
    assert!(
        got.timestamp >= before && got.timestamp <= after,
        "the timestamp {} is outside the moment of the push, {before} to {after}",
        got.timestamp
    );
}

#[test]
fn a_pushed_value_that_is_not_zero_travels_unchanged() {
    let outlet = Outlet::new(stream("KeptStamp")).expect("an outlet");
    let mut inlet = Inlet::open(outlet.info(), Duration::from_secs(5)).expect("an inlet");
    assert!(outlet.wait_for_consumers(Duration::from_secs(5)));
    std::thread::sleep(Duration::from_millis(200));

    outlet.push(&Sample {
        timestamp: 7.125,
        values: vec![Value::F32(3.0), Value::F32(4.0)],
    });
    let got = inlet
        .pull(Duration::from_secs(5))
        .expect("a pull")
        .expect("a sample");
    assert_eq!(got.timestamp, 7.125);
}

#[test]
fn a_chunk_pushed_with_zero_is_dated_from_the_current_clock() {
    let outlet = Outlet::new(stream("ZeroChunk")).expect("an outlet");
    let mut inlet = Inlet::open(outlet.info(), Duration::from_secs(5)).expect("an inlet");
    assert!(outlet.wait_for_consumers(Duration::from_secs(5)));
    std::thread::sleep(Duration::from_millis(200));

    let block: Vec<Sample> = (0..4)
        .map(|k| Sample {
            timestamp: 0.0,
            values: vec![Value::F32(k as f32), Value::F32(0.0)],
        })
        .collect();
    let before = lsl_net::clock();
    outlet.push_chunk(&block, 0.0);

    let mut stamps = Vec::new();
    for _ in 0..4 {
        let s = inlet
            .pull(Duration::from_secs(5))
            .expect("a pull")
            .expect("a sample");
        stamps.push(s.timestamp);
    }
    // The value names the last sample, so the first sits three periods before
    // the moment of the push. SPEC.md 9.2.
    assert!(stamps.iter().all(|t| *t != 0.0), "got {stamps:?}");
    assert!(stamps[0] >= before - 0.05, "got {stamps:?}");
    for k in 1..4 {
        let step = stamps[k] - stamps[k - 1];
        assert!((step - 0.01).abs() < 1e-9, "step {step} at {k}, {stamps:?}");
    }
}
