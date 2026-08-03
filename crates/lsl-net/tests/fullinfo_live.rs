//! The description tree has to travel over a real socket.
//!
//! The golden test in `desc_golden.rs` compares the bytes. This one opens a
//! port, asks for the document the way an inlet asks, and reads the tree back.
//! A writer and a reader that agree with each other but not with the socket
//! pass the golden test. They fail here.

use lsl_net::{read_fullinfo, Outlet, StreamInfo};
use lsl_wire::Format;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

fn stream_with_channels(name: &str) -> StreamInfo {
    let mut i = StreamInfo::new(name, "EEG", 3, Format::Float32, 100.0);
    i.source_id = format!("{name}_src");
    let chns = i.desc.append_child("channels");
    for label in ["C3", "C4", "Cz"] {
        let c = chns.append_child("channel");
        c.append_child_value("label", label);
        c.append_child_value("unit", "microvolts");
        c.append_child_value("type", "EEG");
    }
    i.desc
        .append_child_value("manufacturer", "Acme & Co <test>");
    i
}

#[test]
fn a_data_port_answers_a_request_for_the_whole_description() {
    let outlet = Outlet::new(stream_with_channels("FullInfoLive")).expect("an outlet");
    std::thread::sleep(Duration::from_millis(200));

    let back = read_fullinfo(outlet.info(), Duration::from_secs(5)).expect("a description");

    assert_eq!(back.name, "FullInfoLive");
    assert_eq!(back.channel_count, 3);
    assert_eq!(back.format, Format::Float32);
    assert_eq!(back.uid, outlet.info().uid);
    assert_eq!(back.v4data_port, outlet.info().v4data_port);

    let channels = back.desc.child("channels").expect("a channel list");
    assert_eq!(channels.children.len(), 3);
    let labels: Vec<&str> = channels
        .children
        .iter()
        .map(|c| c.child_value_of("label"))
        .collect();
    assert_eq!(labels, ["C3", "C4", "Cz"]);
    assert_eq!(channels.children[0].child_value_of("unit"), "microvolts");
    // The escape has to survive the wire, not only the writer.
    assert_eq!(back.desc.child_value_of("manufacturer"), "Acme & Co <test>");
}

#[test]
fn the_document_that_the_port_writes_is_the_document_that_the_writer_writes() {
    let outlet = Outlet::new(stream_with_channels("FullInfoBytes")).expect("an outlet");
    std::thread::sleep(Duration::from_millis(200));

    let mut s = TcpStream::connect(("127.0.0.1", outlet.info().v4data_port)).expect("a connection");
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(b"LSL:fullinfo\r\n").unwrap();
    s.flush().unwrap();
    let mut got = String::new();
    s.read_to_string(&mut got).expect("a document");

    assert_eq!(got, outlet.info().to_fullinfo_xml());
    // The sender closes the connection. Nothing else marks the end.
    assert!(got.ends_with("</info>\n"));
}

#[test]
fn a_data_port_answers_a_short_description_request_only_for_a_query_that_matches() {
    let outlet = Outlet::new(stream_with_channels("FullInfoQuery")).expect("an outlet");
    std::thread::sleep(Duration::from_millis(200));
    let port = outlet.info().v4data_port;

    let ask = |query: &str| -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).expect("a connection");
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(format!("LSL:shortinfo\r\n{query}\r\n").as_bytes())
            .unwrap();
        s.flush().unwrap();
        let mut got = String::new();
        let _ = s.read_to_string(&mut got);
        got
    };

    let hit = ask("session_id='default' and name='FullInfoQuery'");
    assert!(hit.contains("<name>FullInfoQuery</name>"), "got {hit:?}");
    // The short form never carries the tree.
    assert!(hit.contains("<desc />"), "got {hit:?}");

    let miss = ask("session_id='default' and name='Nothing'");
    assert_eq!(miss, "", "a query that does not match must get no answer");
}
