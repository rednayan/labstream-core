//! The blocking outlet writes before the push returns, and loses nothing.
//!
//! `transp_sync_blocking` (`include/lsl/common.h:172`) removes the queue
//! between the caller and the socket. liblsl hands the socket to the blocking
//! writer only after the headers and the test pattern have gone out
//! (`src/tcp_server.cpp:733`), and the layout of a sample does not change, so
//! **a consumer cannot tell the two modes apart on the wire**.
//!
//! What changes is what happens when a consumer reads slowly. A queue drops
//! its oldest sample (SPEC.md 8.7). A blocking outlet holds up the caller
//! instead, and every sample arrives.

use labstream_net::{Inlet, Outlet, StreamInfo};
use labstream_wire::{Format, Sample, Value};
use std::time::Duration;

fn stream(name: &str, format: Format) -> StreamInfo {
    let mut i = StreamInfo::new(name, "Test", 1, format, 100.0);
    i.source_id = format!("{name}_src");
    i
}

#[test]
fn a_stream_of_strings_cannot_be_blocking() {
    // `src/stream_outlet_impl.cpp:34-38` throws for this, because a string has
    // no fixed width and the mode writes the memory of the caller directly.
    let err = match Outlet::new_blocking(stream("SyncStr", Format::String)) {
        Ok(_) => panic!("a stream of strings was accepted in the blocking mode"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("string"),
        "the message must name the reason, got {err}"
    );
}

#[test]
fn a_blocking_outlet_reports_its_consumers() {
    let outlet = Outlet::new_blocking(stream("SyncCount", Format::Float32)).expect("an outlet");
    assert_eq!(outlet.consumer_count(), 0);
    let _inlet = Inlet::open(outlet.info(), Duration::from_secs(5)).expect("an inlet");
    assert!(
        outlet.wait_for_consumers(Duration::from_secs(5)),
        "a blocking outlet must count the consumers of its writer"
    );
    assert_eq!(outlet.consumer_count(), 1);
}

#[test]
fn a_blocking_outlet_loses_nothing_to_a_slow_consumer() {
    // The buffer that an inlet asks for sizes **both** queues: the one in the
    // outlet and the one in the inlet. A small request therefore loses samples
    // in the inlet whatever the outlet does, and the two sides cannot be told
    // apart through the public calls. A first version of this test asked for
    // one sample and received three of two hundred, all lost in the inlet.
    //
    // What is left to test is the property that the mode promises: with a
    // consumer whose buffer is large enough, a blocking outlet loses nothing,
    // however slowly the application reads.
    let outlet = Outlet::new_blocking(stream("SyncKeep", Format::Float32)).expect("an outlet");
    let mut inlet =
        Inlet::open_with_buffer(outlet.info(), Duration::from_secs(5), 1000).expect("an inlet");
    assert!(outlet.wait_for_consumers(Duration::from_secs(5)));
    std::thread::sleep(Duration::from_millis(200));

    const COUNT: usize = 200;
    let sender = std::thread::spawn(move || {
        for k in 0..COUNT {
            outlet.push(&Sample {
                timestamp: 1.0 + k as f64,
                values: vec![Value::F32(k as f32)],
            });
        }
        outlet
    });

    let mut seen = Vec::new();
    for _ in 0..COUNT {
        // A slow reader, so the writer has to wait for it.
        std::thread::sleep(Duration::from_millis(1));
        match inlet.pull(Duration::from_secs(5)) {
            Ok(Some(s)) => seen.push(s.values[0].clone()),
            Ok(None) => break,
            Err(e) => panic!("pull failed: {e}"),
        }
    }
    let _outlet = sender.join().expect("the sender");

    assert_eq!(seen.len(), COUNT, "a blocking outlet must lose nothing");
    let want: Vec<Value> = (0..COUNT).map(|k| Value::F32(k as f32)).collect();
    assert_eq!(seen, want, "the samples must arrive in order and complete");
}
