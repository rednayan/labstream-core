//! The background resolver must find a stream and then forget it.

use lsl_net::{ContinuousResolver, Outlet, StreamInfo};
use lsl_wire::Format;
use std::time::{Duration, Instant};

fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    done()
}

#[test]
fn a_background_resolver_finds_a_stream_and_forgets_it_after_it_stops() {
    // A short memory keeps the test short. liblsl uses the same value for the
    // same purpose in `GetAllStreams`.
    let resolver = ContinuousResolver::new("session_id='default' and name='ResolverLive'", 2.0);

    // Nothing is published yet.
    assert_eq!(resolver.results(10).len(), 0);

    let mut info = StreamInfo::new("ResolverLive", "EEG", 2, Format::Float32, 100.0);
    info.source_id = "resolver_live_src".into();
    let outlet = Outlet::new(info).expect("an outlet");
    let uid = outlet.info().uid.clone();

    assert!(
        wait_until(Duration::from_secs(15), || resolver.results(10).len() == 1),
        "the resolver never found the stream"
    );
    let found = resolver.results(10);
    assert_eq!(found[0].name, "ResolverLive");
    assert_eq!(found[0].uid, uid);
    assert_eq!(found[0].v4data_port, outlet.info().v4data_port);

    drop(outlet);

    // The stream stops answering, so it leaves the list after the memory runs
    // out. Nothing on the wire says that a stream went away. SPEC.md 10.
    assert!(
        wait_until(Duration::from_secs(20), || resolver.results(10).is_empty()),
        "the resolver kept a stream that stopped answering"
    );
}

#[test]
fn a_query_that_matches_nothing_returns_nothing() {
    let resolver = ContinuousResolver::new("session_id='default' and name='NoSuchStream'", 30.0);
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(resolver.results(10).len(), 0);
}
