//! The LSL protocol state machines.
//!
//! This crate holds no input and no output. It takes bytes and events, and it
//! returns bytes and decisions. Every rule comes from `SPEC.md`, which cites
//! the pinned oracle.
//!
//! The layering matters. liblsl parses a request inside a socket completion
//! handler (`src/tcp_server.cpp:495-500`), so a test of that parser needs a
//! network. Here a test calls a function with a byte slice.
//!
//! # Modules
//!
//! | Module | Holds |
//! |---|---|
//! | [`header`] | The header block parser and the status line |
//! | [`negotiate`](negotiate::negotiate) | The server decision, as a pure function |
//! | [`feed`] | The data feed state machine, client side |
//! | [`discovery`] | The shortinfo query and answer |
//! | [`timesync`] | The time probe and the offset estimator |
//! | [`canon`] | The canonicalizer for transcript comparison |

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod canon;
pub mod discovery;
pub mod feed;
pub mod header;
pub mod negotiate;
pub mod timesync;

pub use header::{Error, Headers, StatusAction, StatusLine};
pub use negotiate::{negotiate, Accepted, FeedRequest, Outcome, ServerStream};
