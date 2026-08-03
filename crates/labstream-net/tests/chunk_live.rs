//! The block calls, measured against a live outlet.
//!
//! These tests open real sockets on the loopback interface, so they belong to
//! the reporting tier and not to the merge gate. A failure here can come from
//! the machine.
//!
//! `Outlet::push_chunk_fast` and `Inlet::pull_chunk` exist for speed. They must
//! deliver what the one-sample calls deliver. A unit test proves that for the
//! queue. This proves it over a socket, with the codec and the timestamp rules
//! in the path.

use labstream_net::{Inlet, Outlet, StreamInfo};
use labstream_wire::{Format, Sample, Value};
use std::time::Duration;

const RATE: f64 = 100.0;

fn stream(name: &str) -> StreamInfo {
    let mut i = StreamInfo::new(name, "ChunkTest", 2, Format::Float32, RATE);
    i.source_id = format!("{name}_src");
    i
}

fn block(n: usize) -> Vec<Sample> {
    (0..n)
        .map(|k| Sample {
            timestamp: 0.0,
            values: vec![Value::F32(k as f32), Value::F32(-(k as f32))],
        })
        .collect()
}

fn first_channel(s: &Sample) -> f32 {
    match s.values[0] {
        Value::F32(v) => v,
        _ => panic!("wrong format"),
    }
}

/// Open an outlet and an inlet against each other on this machine.
fn pair(name: &str) -> Option<(Outlet, Inlet)> {
    let outlet = Outlet::new(stream(name)).ok()?;
    let info = outlet.info().clone();
    let inlet = Inlet::open_with_buffer(&info, Duration::from_secs(5), 4096).ok()?;
    outlet.wait_for_consumers(Duration::from_secs(2));
    Some((outlet, inlet))
}

/// Read `want` samples with the block call, or give what arrived.
fn drain(inlet: &mut Inlet, want: usize) -> Vec<Sample> {
    let mut out = Vec::new();
    let end = std::time::Instant::now() + Duration::from_secs(5);
    while out.len() < want && std::time::Instant::now() < end {
        inlet
            .pull_chunk(&mut out, want, Duration::from_millis(100))
            .expect("a block read");
    }
    out
}

#[test]
fn a_block_write_delivers_every_sample_in_order() {
    let (outlet, mut inlet) = match pair("ChunkLive1") {
        Some(p) => p,
        None => {
            eprintln!("no local network; skipping");
            return;
        }
    };

    let sent = block(500);
    outlet.push_chunk_fast(&sent, labstream_net::clock());

    let got = drain(&mut inlet, sent.len());
    assert_eq!(got.len(), sent.len(), "every sample arrived");
    for (k, s) in got.iter().enumerate() {
        assert_eq!(first_channel(s), k as f32, "sample {k} is in place");
    }
}

#[test]
fn a_block_write_dates_its_samples_as_a_chunk_does() {
    // SPEC.md 9.2: the timestamp names the last sample, the first sample carries
    // the only stamp on the wire, and the rest carry the deduced tag. The two
    // calls must therefore give the same first stamp and the same spacing.
    let (outlet, mut inlet) = match pair("ChunkLive2") {
        Some(p) => p,
        None => {
            eprintln!("no local network; skipping");
            return;
        }
    };

    let sent = block(50);
    let at = labstream_net::clock();
    outlet.push_chunk(&sent, at);
    let slow = drain(&mut inlet, sent.len());

    let at2 = labstream_net::clock();
    outlet.push_chunk_fast(&sent, at2);
    let fast = drain(&mut inlet, sent.len());

    assert_eq!(slow.len(), sent.len());
    assert_eq!(fast.len(), sent.len());

    // The first stamp counts back from the value that the caller gave.
    let expect = |at: f64| at - (sent.len() - 1) as f64 / RATE;
    assert!(
        (slow[0].timestamp - expect(at)).abs() < 1e-9,
        "the one-sample path dates the block from its end"
    );
    assert!(
        (fast[0].timestamp - expect(at2)).abs() < 1e-9,
        "the block path dates the block the same way"
    );

    // Every later sample carries the deduced tag, so the reader spaces them.
    // The two blocks went out at different moments, so the test compares the
    // spacing inside each block and not the two absolute values.
    for (k, (a, b)) in slow.iter().zip(fast.iter()).enumerate() {
        let (da, db) = (
            a.timestamp - slow[0].timestamp,
            b.timestamp - fast[0].timestamp,
        );
        assert!(
            (da - db).abs() < 1e-9,
            "sample {k}: the one-sample path spaced it at {da} and the block path at {db}"
        );
    }
}

#[test]
fn a_block_read_gives_what_single_reads_give() {
    let (outlet, mut inlet) = match pair("ChunkLive3") {
        Some(p) => p,
        None => {
            eprintln!("no local network; skipping");
            return;
        }
    };

    let sent = block(120);
    outlet.push_chunk_fast(&sent, labstream_net::clock());

    // Read the first half one sample at a time, and the rest as a block.
    let mut one_at_a_time = Vec::new();
    while one_at_a_time.len() < 60 {
        if let Some(s) = inlet.pull(Duration::from_millis(200)).expect("a read") {
            one_at_a_time.push(s);
        }
    }
    let rest = drain(&mut inlet, 60);

    assert_eq!(rest.len(), 60);
    for (k, s) in one_at_a_time.iter().chain(rest.iter()).enumerate() {
        assert_eq!(first_channel(s), k as f32, "sample {k} is in place");
    }
}
