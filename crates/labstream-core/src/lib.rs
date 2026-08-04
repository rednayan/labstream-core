//! The LSL core library, as one crate.
//!
//! This crate holds no code of its own. It names the four crates that carry
//! the library, so a program takes one dependency and not four:
//!
//! ```toml
//! [dependencies]
//! labstream-core = "0.1"
//! ```
//!
//! Each crate stays separate below this one. A program that wants one part
//! alone can still name that part alone.
//!
//! # The parts
//!
//! | Module | Crate | Touches the operating system |
//! |---|---|---|
//! | [`wire`] | `labstream-wire` | no |
//! | [`proto`] | `labstream-proto` | no |
//! | [`time`] | `labstream-time` | no |
//! | [`net`] | `labstream-net` | yes |
//!
//! # The C ABI is not here
//!
//! `labstream-capi` builds `liblsl.so` for a C program. It gives a Rust
//! program nothing, so this crate does not name it. `README.md` gives the way
//! to build it.
//!
//! # Features
//!
//! `net` is on by default. It carries the one crate that opens a socket. A
//! program that reads bytes from somewhere else can turn it off:
//!
//! ```toml
//! [dependencies]
//! labstream-core = { version = "0.1", default-features = false }
//! ```
//!
//! `wire`, `proto`, and `time` hold no input and no output, so they cost a
//! program nothing and stay present.
//!
//! # An example
//!
//! ```no_run
//! use labstream_core::net::{clock, Outlet, StreamInfo};
//! use labstream_core::wire::{Format, Sample, Value};
//!
//! # fn main() -> std::io::Result<()> {
//! let info = StreamInfo::new("BioSemi", "EEG", 8, Format::Float32, 100.0);
//! let outlet = Outlet::new(info)?;
//! outlet.push(&Sample {
//!     timestamp: clock(),
//!     values: (0..8).map(|k| Value::F32(k as f32)).collect(),
//! });
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![deny(missing_docs)]

/// The handshake, the discovery messages, and the time sync. No input and no
/// output.
pub use labstream_proto as proto;
/// The timestamp filter. The result matches the C++ filter bit for bit.
pub use labstream_time as time;
/// The sample codec for protocol 1.10. No input and no output.
pub use labstream_wire as wire;

/// Sockets, outlet, inlet, resolver, configuration, and XPath.
///
/// This is the one part that touches the operating system. The `net` feature
/// carries it, and that feature is on by default.
#[cfg(feature = "net")]
pub use labstream_net as net;
