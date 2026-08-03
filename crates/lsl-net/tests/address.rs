//! An inlet must reach an outlet that is not on the loopback.
//!
//! An outlet leaves the address field empty, and the resolver of the peer fills
//! it in from the sender of the answer (`src/resolve_attempt_udp.cpp:135-140`).
//! An inlet then connects to that address.
//!
//! # Why this test exists
//!
//! Every connection used to name `127.0.0.1`. Every test in this project runs
//! on one machine, so nothing noticed. An inlet on a second machine could
//! never have reached a stream, and the direction was never tried: the live
//! tests all used our outlet with the inlet of liblsl.
//!
//! This test connects through an address of this machine that is not the
//! loopback, which is the same path a second machine takes.

use lsl_net::{resolve, Inlet, Outlet, StreamInfo};
use lsl_wire::{Format, Sample, Value};
use std::time::Duration;

fn stream(name: &str) -> StreamInfo {
    let mut i = StreamInfo::new(name, "Test", 2, Format::Float32, 100.0);
    i.source_id = format!("{name}_src");
    i
}

#[test]
fn a_resolved_description_carries_the_address_of_the_sender() {
    let _outlet = Outlet::new(stream("AddrStamp")).expect("an outlet");
    std::thread::sleep(Duration::from_millis(300));

    let found = resolve(
        "session_id='default' and name='AddrStamp'",
        1,
        Duration::from_secs(5),
    )
    .expect("a resolve");
    assert_eq!(found.len(), 1, "the stream was not found");
    let info = &found[0];
    assert!(
        !info.v4address.is_empty(),
        "the description carries no address, so an inlet has nowhere to connect"
    );
    assert!(
        info.v4address.parse::<std::net::Ipv4Addr>().is_ok(),
        "the address {:?} is not an address",
        info.v4address
    );
}

#[test]
fn an_inlet_uses_the_address_it_was_given() {
    // The test above passes even when the address is ignored, because the
    // loopback reaches the same outlet. This one cannot: the address names a
    // machine that does not answer, so a connection can only fail. An inlet
    // that falls back to the loopback would open and this test would fail.
    //
    // 192.0.2.0/24 is the range that RFC 5737 keeps for documentation, so no
    // real machine holds this address.
    let outlet = Outlet::new(stream("AddrHonour")).expect("an outlet");
    std::thread::sleep(Duration::from_millis(300));

    let mut info = outlet.info().clone();
    info.v4address = "192.0.2.1".to_string();

    let opened = Inlet::open(&info, Duration::from_secs(2));
    assert!(
        opened.is_err(),
        "the inlet reached a stream at an address that answers nothing, \
         so it ignored the address and used the loopback"
    );
}

#[test]
fn an_inlet_reaches_an_outlet_through_an_address_that_is_not_the_loopback() {
    let addresses: Vec<std::net::Ipv4Addr> = lsl_net::local_ipv4_addresses()
        .into_iter()
        .filter(|a| !a.is_loopback() && !a.is_unspecified() && a.octets()[3] != 0)
        .collect();
    let Some(addr) = addresses.first() else {
        eprintln!("this machine has no address outside the loopback, so nothing is tested");
        return;
    };
    println!("reaching the outlet through {addr}");

    let outlet = Outlet::new(stream("AddrReach")).expect("an outlet");
    std::thread::sleep(Duration::from_millis(300));

    // Name the address the way a second machine would, rather than the way a
    // program on this machine would.
    let mut info = outlet.info().clone();
    info.v4address = addr.to_string();

    let mut inlet = Inlet::open(&info, Duration::from_secs(5))
        .unwrap_or_else(|e| panic!("cannot reach {addr}: {e}"));
    assert!(outlet.wait_for_consumers(Duration::from_secs(5)));
    std::thread::sleep(Duration::from_millis(200));

    outlet.push(&Sample {
        timestamp: 7.125,
        values: vec![Value::F32(1.0), Value::F32(2.0)],
    });
    let got = inlet
        .pull(Duration::from_secs(5))
        .expect("a pull")
        .expect("a sample");
    assert_eq!(got.timestamp, 7.125);
    assert_eq!(got.values, vec![Value::F32(1.0), Value::F32(2.0)]);
}
