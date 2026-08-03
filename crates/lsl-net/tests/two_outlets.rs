//! Two outlets in one process must both be reachable.
//!
//! A program that publishes a data stream and a marker stream builds two
//! outlets. Both need the port that carries a multicast query, so that port has
//! to be shared. liblsl shares it with `reuse_address`
//! (`src/udp_server.cpp:60`).
//!
//! Without the option the second outlet answers a unicast query and a
//! broadcast query, and never sees a multicast query. On one machine every test
//! still passes, because a unicast query reaches the whole port range. Another
//! machine sends a multicast query and finds only one of the two streams.

use lsl_net::{resolve, Outlet, StreamInfo};
use lsl_wire::Format;
use std::time::Duration;

#[test]
fn both_outlets_hold_the_multicast_port() {
    let mut a = StreamInfo::new("TwoA", "EEG", 2, Format::Float32, 100.0);
    a.source_id = "two_a".into();
    let mut b = StreamInfo::new("TwoB", "Markers", 1, Format::String, 0.0);
    b.source_id = "two_b".into();

    let first = Outlet::new(a).expect("the first outlet");
    let second = Outlet::new(b).expect("the second outlet");

    assert!(
        first.holds_multicast_port(),
        "the first outlet must hold the port"
    );
    assert!(
        second.holds_multicast_port(),
        "the second outlet must hold the port as well, or another machine sees only one stream"
    );
    // The two must not share a data port.
    assert_ne!(first.info().v4data_port, second.info().v4data_port);
}

#[test]
fn a_resolver_finds_both_streams() {
    let mut a = StreamInfo::new("BothA", "EEG", 2, Format::Float32, 100.0);
    a.source_id = "both_a".into();
    let mut b = StreamInfo::new("BothB", "Markers", 1, Format::String, 0.0);
    b.source_id = "both_b".into();
    let _first = Outlet::new(a).expect("the first outlet");
    let _second = Outlet::new(b).expect("the second outlet");
    std::thread::sleep(Duration::from_millis(300));

    let found = resolve("session_id='default'", 2, Duration::from_secs(5)).expect("a resolve");
    let names: Vec<&str> = found.iter().map(|i| i.name.as_str()).collect();
    assert!(names.contains(&"BothA"), "got {names:?}");
    assert!(names.contains(&"BothB"), "got {names:?}");
}
