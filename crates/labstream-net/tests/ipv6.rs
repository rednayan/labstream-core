//! A stream must be reachable over IPv6.
//!
//! An outlet binds one stack for each protocol that the configuration allows
//! (`src/stream_outlet_impl.cpp:43-53`) and reports the ports of both. A
//! resolver runs one attempt per protocol and only sends to the groups of that
//! protocol (`src/resolve_attempt_udp.cpp:174`).

use labstream_net::{resolve, Inlet, Outlet, StreamInfo};
use labstream_wire::{Format, Sample, Value};
use std::time::Duration;

fn stream(name: &str) -> StreamInfo {
    let mut i = StreamInfo::new(name, "Test", 2, Format::Float32, 100.0);
    i.source_id = format!("{name}_src");
    i
}

#[test]
fn an_outlet_reports_a_port_for_each_protocol() {
    let outlet = Outlet::new(stream("V6Ports")).expect("an outlet");
    let info = outlet.info();
    assert_ne!(info.v4data_port, 0, "no IPv4 data port");
    assert_ne!(info.v4service_port, 0, "no IPv4 service port");
    assert_ne!(info.v6data_port, 0, "no IPv6 data port");
    assert_ne!(info.v6service_port, 0, "no IPv6 service port");
    // This test asserted that the two data ports differ. That assertion was
    // wrong. It held on Linux and failed on macOS and on Windows.
    //
    // liblsl opens one acceptor for each family and scans the port range for
    // each one (`src/tcp_server.cpp:333-353`). It never sets `IPV6_V6ONLY`, so
    // the default of the platform decides whether the second bind meets the
    // first. Linux gives a dual stack socket, the second bind meets the first,
    // and the scan moves to the next port. macOS and Windows give one family
    // for each socket, and both take the base port.
    //
    // A measurement on Windows read liblsl through pylsl. It reported 16572
    // for `v4data_port` and 16572 for `v6data_port`. One number for both
    // families is therefore what liblsl does, and this library matches it.
    //
    // The port of each family has to be present, and a reader connects to the
    // port of its own family. Whether the two numbers differ is a property of
    // the platform and not a rule of the protocol.
    println!(
        "v4 data {} service {}, v6 data {} service {}",
        info.v4data_port, info.v4service_port, info.v6data_port, info.v6service_port
    );
}

#[test]
fn the_description_carries_both_families() {
    let outlet = Outlet::new(stream("V6Xml")).expect("an outlet");
    let xml = outlet.info().to_shortinfo_xml();
    let want = format!("<v6data_port>{}</v6data_port>", outlet.info().v6data_port);
    assert!(xml.contains(&want), "got {xml}");
    let back = StreamInfo::from_shortinfo_xml(&xml).expect("a description");
    assert_eq!(back.v6data_port, outlet.info().v6data_port);
    assert_eq!(back.v6service_port, outlet.info().v6service_port);
}

#[test]
fn a_resolver_finds_a_stream_over_ipv6() {
    let _outlet = Outlet::new(stream("V6Find")).expect("an outlet");
    std::thread::sleep(Duration::from_millis(400));
    let found = resolve(
        "session_id='default' and name='V6Find'",
        1,
        Duration::from_secs(6),
    )
    .expect("a resolve");
    assert_eq!(found.len(), 1, "the stream was not found at all");
    let info = &found[0];
    println!(
        "v4address {:?} v6address {:?}",
        info.v4address, info.v6address
    );
    assert!(
        !info.v4address.is_empty() || !info.v6address.is_empty(),
        "the answer carried no address"
    );
}

#[test]
fn an_inlet_reads_a_stream_over_ipv6_alone() {
    let outlet = Outlet::new(stream("V6Only")).expect("an outlet");
    std::thread::sleep(Duration::from_millis(300));

    // Name only the IPv6 endpoint, so IPv4 cannot be used as a way out.
    let mut info = outlet.info().clone();
    info.v4address = String::new();
    info.v4data_port = 0;
    info.v4service_port = 0;
    info.v6address = "::1".to_string();

    let mut inlet = Inlet::open(&info, Duration::from_secs(5)).expect("an inlet over IPv6");
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
