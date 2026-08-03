//! Publish a stream that another machine can find and record.
//!
//! This uses `labstream-net` directly, so no C ABI is on the path. Run it on one
//! machine and open LabRecorder on another.
//!
//! ```text
//! cargo run --release -p labstream-net --example publish -- --name RustTest
//! ```
//!
//! The signal is built from the sample number, so a recording can be read back
//! and tested. Channel `k` carries a sine wave of `k + 1` Hz. The last channel
//! carries the sample number itself, which shows a gap at once.

use labstream_net::{Outlet, StreamInfo};
use labstream_wire::{Format, Sample, Value};
use std::time::{Duration, Instant};

struct Args {
    name: String,
    stream_type: String,
    channels: usize,
    rate: f64,
    seconds: f64,
    chunk: usize,
    markers: bool,
    /// A word that separates one publisher from another on the same network.
    tag: String,
}

fn parse() -> Args {
    let mut a = Args {
        name: "RustTest".to_string(),
        stream_type: "EEG".to_string(),
        channels: 8,
        rate: 100.0,
        seconds: 0.0,
        chunk: 1,
        markers: true,
        tag: String::new(),
    };
    let raw: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i + 1 < raw.len() {
        let (k, v) = (raw[i].as_str(), raw[i + 1].as_str());
        match k {
            "--name" => a.name = v.to_string(),
            "--type" => a.stream_type = v.to_string(),
            "--channels" => a.channels = v.parse().unwrap_or(8),
            "--rate" => a.rate = v.parse().unwrap_or(100.0),
            // Zero means no limit.
            "--seconds" => a.seconds = v.parse().unwrap_or(0.0),
            "--chunk" => a.chunk = v.parse().unwrap_or(1),
            "--markers" => a.markers = v != "0",
            "--tag" => a.tag = v.to_string(),
            other => {
                eprintln!("unknown option {other}");
                std::process::exit(2);
            }
        }
        i += 2;
    }
    a
}

/// The marker for one whole second of the signal, if there is one.
///
/// A marker stream carries no rate, so it holds an event and not a sample.
/// This sends one at each whole second, which is where the sine of channel one
/// crosses zero on its way up. A plot then shows each marker on a crossing,
/// and an alignment error is visible without any measurement.
fn marker_for(n: u64, rate: f64) -> Option<String> {
    let per_second = rate.max(1.0) as u64;
    if n % per_second != 0 {
        return None;
    }
    let second = n / per_second;
    Some(if second % 5 == 0 {
        format!("burst {second}")
    } else {
        format!("tick {second}")
    })
}

/// The label of one channel. LabRecorder writes these into the recording.
fn label(k: usize) -> String {
    const NAMES: [&str; 8] = ["Fp1", "Fp2", "C3", "C4", "P3", "P4", "O1", "O2"];
    match NAMES.get(k) {
        Some(n) => n.to_string(),
        None => format!("Ch{}", k + 1),
    }
}

/// Add the tag to a name, when there is one.
///
/// The tag separates one publisher from another. With it, the three modes of
/// `oracle/labrecorder.sh` can run at the same time, and a recorder shows
/// which library sent which stream.
fn tagged(base: &str, tag: &str) -> String {
    if tag.is_empty() {
        base.to_string()
    } else {
        format!("{base}-{tag}")
    }
}

fn main() {
    let a = parse();
    let stream_name = tagged(&a.name, &a.tag);
    let marker_name = tagged(&format!("{}-Markers", a.name), &a.tag);
    let suffix = if a.tag.is_empty() {
        String::new()
    } else {
        format!("_{}", a.tag)
    };

    let mut info = StreamInfo::new(
        &stream_name,
        &a.stream_type,
        a.channels as u32,
        Format::Float32,
        a.rate,
    );
    // A source identifier lets a consumer find the stream again after a
    // restart. Without one, no recovery is possible. SPEC.md 10.1.
    //
    // The identifier and the tree below match `oracle/publish.c` exactly, so
    // the three publishers of `oracle/labrecorder.sh` describe one stream and
    // a plot of one can be compared with a plot of another.
    info.source_id = format!("{}_publish{suffix}", a.name);

    {
        let chns = info.desc.append_child("channels");
        for k in 0..a.channels {
            let c = chns.append_child("channel");
            c.append_child_value("label", &label(k));
            c.append_child_value("unit", "microvolts");
            c.append_child_value("type", &a.stream_type);
        }
    }
    info.desc
        .append_child_value("manufacturer", "conformance publisher");
    let acq = info.desc.append_child("acquisition");
    acq.append_child_value("model", "publish example");

    let outlet = match Outlet::new(info) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("cannot publish: {e}");
            std::process::exit(1);
        }
    };

    // A marker stream is a second stream: one channel of text, and no rate.
    // A recorder treats it as events rather than as a signal.
    let markers = if a.markers {
        let mut m = StreamInfo::new(&marker_name, "Markers", 1, Format::String, 0.0);
        m.source_id = format!("{}_markers{suffix}", a.name);
        match Outlet::new(m) {
            Ok(o) => Some(o),
            Err(e) => {
                eprintln!("cannot publish the markers: {e}");
                None
            }
        }
    } else {
        None
    };

    let published = outlet.info();
    println!("=== the stream is published");
    println!("  name          {}", published.name);
    println!("  type          {}", published.stream_type);
    println!("  channels      {} float32", published.channel_count);
    println!("  rate          {} Hz", published.nominal_srate);
    println!("  source id     {}", published.source_id);
    println!("  session       {}", published.session_id);
    println!("  host          {}", published.hostname);
    println!("  uid           {}", published.uid);
    let cfg = labstream_net::config::get();
    let port = |n: u16| {
        if n == 0 {
            "none".to_string()
        } else {
            n.to_string()
        }
    };
    println!(
        "  IPv4          data {}  service {}",
        port(published.v4data_port),
        port(published.v4service_port)
    );
    println!(
        "  IPv6          data {}  service {}",
        port(published.v6data_port),
        port(published.v6service_port)
    );
    let which = match (cfg.allow_ipv4, cfg.allow_ipv6) {
        (true, true) => "IPv4 and IPv6",
        (true, false) => "IPv4 only",
        (false, true) => "IPv6 only. A consumer has no other way to reach this stream.",
        (false, false) => "none. Nothing can reach this stream.",
    };
    println!("  protocols     {which}");
    match &markers {
        Some(m) => {
            println!(
                "  marker stream {}  ({})",
                m.info().name,
                m.info().source_id
            );
            println!("                one every second, on a zero crossing of channel one");
            if !m.holds_multicast_port() {
                println!("  WARNING: the marker stream does not hold the multicast port.");
            }
        }
        None => println!("  marker stream none"),
    }

    // A query from another machine arrives by multicast or by broadcast. The
    // outlet joins each group on each local address, so this list is the one
    // that decides whether another machine can find the stream at all.
    let addresses = labstream_net::local_ipv4_addresses();
    println!("\n=== the addresses that carry a query");
    if addresses.is_empty() {
        println!("  none found. Another machine will not see this stream.");
    }
    for addr in &addresses {
        println!("  {addr}");
    }
    println!("  multicast port {}", cfg.multicast_port);
    println!("  groups         {}", cfg.multicast_addresses.join(", "));
    println!("  configuration  {}", cfg.source);
    if outlet.holds_multicast_port() {
        println!("  multicast port held by this outlet");
    } else {
        println!(
            "  WARNING: port {} is held by another process on this machine.",
            cfg.multicast_port
        );
        println!("  A multicast query will not reach this outlet. Stop the other");
        println!("  program, or expect the other machine to find nothing.");
    }

    println!("\n=== open LabRecorder on the other machine and press Update");
    println!("  The count below rises when a recorder links the stream.\n");

    let period = if a.rate > 0.0 { 1.0 / a.rate } else { 0.01 };
    let start = Instant::now();
    let mut n: u64 = 0;
    let mut consumers = 0;
    let mut reported = Instant::now() - Duration::from_secs(10);
    let mut block: Vec<Sample> = Vec::new();

    loop {
        if a.seconds > 0.0 && start.elapsed().as_secs_f64() >= a.seconds {
            println!("\n{n} samples sent. Stopping.");
            return;
        }

        let now = outlet.consumer_count();
        if now != consumers {
            println!(
                "  consumers {consumers} -> {now}   after {:.1} s, {n} samples",
                start.elapsed().as_secs_f64()
            );
            consumers = now;
        }
        if reported.elapsed() >= Duration::from_secs(5) {
            println!(
                "  {:.0} s, {n} samples, {consumers} consumer(s)",
                start.elapsed().as_secs_f64()
            );
            reported = Instant::now();
        }

        // The value of a channel comes from the sample number, so a recording
        // can be tested later. The last channel holds the number itself.
        let t = n as f64 / a.rate.max(1.0);
        let values: Vec<Value> = (0..a.channels)
            .map(|k| {
                if k + 1 == a.channels {
                    Value::F32(n as f32)
                } else {
                    let hz = (k + 1) as f64;
                    Value::F32((100.0 * (2.0 * std::f64::consts::PI * hz * t).sin()) as f32)
                }
            })
            .collect();
        let sample = Sample {
            // A timestamp of zero means the current clock. SPEC.md 8.3.
            timestamp: 0.0,
            values,
        };

        if a.chunk > 1 {
            block.push(sample);
            if block.len() == a.chunk {
                // The timestamp names the last sample of the chunk. SPEC.md 9.2.
                outlet.push_chunk(&block, 0.0);
                block.clear();
            }
        } else {
            outlet.push(&sample);
        }

        // The marker carries the same rule for its timestamp as the sample: a
        // value of zero means the current clock. The two therefore sit within
        // a fraction of one period of each other. SPEC.md 8.3.
        if let (Some(m), Some(text)) = (&markers, marker_for(n, a.rate)) {
            m.push(&Sample {
                timestamp: 0.0,
                values: vec![Value::Str(text.into_bytes())],
            });
        }
        n += 1;

        std::thread::sleep(Duration::from_secs_f64(period));
    }
}
