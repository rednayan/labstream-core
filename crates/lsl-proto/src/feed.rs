//! The data feed state machine, client side. SPEC.md 4 and 6.
//!
//! The machine takes bytes and returns bytes. It opens no socket and reads no
//! clock, so a test drives every state with a byte slice.

use crate::header::{parse_block, Error, Headers, StatusLine};
use crate::negotiate::{BIG, LITTLE};
use lsl_wire::{ByteOrder, Codec, Format, Sample, TEST_PATTERN_OFFSETS};

/// What the client asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedParams {
    /// The version to ask for.
    pub version: i32,
    /// The UID of the stream to open.
    pub uid: String,
    /// The byte order of this machine.
    pub native_byte_order: i32,
    /// How fast this machine converts a byte order.
    ///
    /// A measured value. A test injects a fixed number, because no conformance
    /// test can compare it. SPEC.md 5.2.
    pub endian_performance: f64,
    /// True when this machine holds IEEE-754 floats.
    pub has_ieee754_floats: bool,
    /// True when this machine holds subnormal values.
    pub supports_subnormals: bool,
    /// The width of one channel value. Zero for a string stream.
    pub value_size: i32,
    /// The buffer length to ask for.
    pub max_buffer_length: i32,
    /// The chunk length to ask for. Zero means no limit.
    pub max_chunk_length: i32,
    /// The host name of this machine.
    pub hostname: String,
    /// The source identifier of the stream.
    pub source_id: String,
    /// The session identifier.
    pub session_id: String,
}

impl FeedParams {
    /// Build the usual parameters for a stream.
    pub fn new(uid: &str, format: Format) -> Self {
        FeedParams {
            version: crate::negotiate::LSL_PROTOCOL_VERSION,
            uid: uid.to_string(),
            native_byte_order: if cfg!(target_endian = "little") {
                LITTLE
            } else {
                BIG
            },
            endian_performance: 0.0,
            has_ieee754_floats: true,
            supports_subnormals: true,
            value_size: format.width().unwrap_or(0) as i32,
            max_buffer_length: 360,
            max_chunk_length: 0,
            hostname: String::new(),
            source_id: String::new(),
            session_id: "default".to_string(),
        }
    }

    /// Write the request line and the header block.
    ///
    /// The field order matches `src/data_receiver.cpp:170-190`. A real liblsl
    /// server reads the names and not the order, but a transcript comparison
    /// needs the same order.
    pub fn to_wire(&self) -> Vec<u8> {
        let mut s = format!("LSL:streamfeed/{} {}\r\n", self.version, self.uid);
        s.push_str(&format!(
            "Native-Byte-Order: {}\r\n",
            self.native_byte_order
        ));
        s.push_str(&format!(
            "Endian-Performance: {}\r\n",
            self.endian_performance.floor()
        ));
        s.push_str(&format!(
            "Has-IEEE754-Floats: {}\r\n",
            self.has_ieee754_floats as i32
        ));
        s.push_str(&format!(
            "Supports-Subnormals: {}\r\n",
            self.supports_subnormals as i32
        ));
        s.push_str(&format!("Value-Size: {}\r\n", self.value_size));
        // liblsl writes this name, and the server never reads it. SPEC.md 4.4.
        s.push_str(&format!("Data-Protocol-Version: {}\r\n", self.version));
        s.push_str(&format!(
            "Max-Buffer-Length: {}\r\n",
            self.max_buffer_length
        ));
        s.push_str(&format!("Max-Chunk-Length: {}\r\n", self.max_chunk_length));
        s.push_str(&format!("Hostname: {}\r\n", self.hostname));
        s.push_str(&format!("Source-Id: {}\r\n", self.source_id));
        s.push_str(&format!("Session-Id: {}\r\n", self.session_id));
        s.push_str("\r\n");
        s.into_bytes()
    }
}

/// The parameters that the answer settled.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedAgreement {
    /// The negotiated data protocol version.
    pub data_protocol_version: i32,
    /// The byte order for the sample stream.
    pub order: ByteOrder,
    /// True when the reader clears subnormal values.
    pub suppress_subnormals: bool,
    /// The UID that the answer named.
    pub uid: String,
}

/// Where the client is in the handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// The request went out. The client waits for the answer headers.
    AwaitingHeaders,
    /// The headers arrived. The client waits for two test pattern samples.
    AwaitingTestPattern,
    /// The handshake finished. Every later byte is a sample.
    Streaming,
    /// The handshake stopped.
    Failed,
}

/// The client side of a data feed.
pub struct FeedClient {
    params: FeedParams,
    format: Format,
    channels: usize,
    state: State,
    agreement: Option<FeedAgreement>,
    codec: Option<Codec>,
}

impl FeedClient {
    /// Build a client and return the bytes to send.
    pub fn start(params: FeedParams, format: Format, channels: usize) -> (Self, Vec<u8>) {
        let bytes = params.to_wire();
        (
            FeedClient {
                params,
                format,
                channels,
                state: State::AwaitingHeaders,
                agreement: None,
                codec: None,
            },
            bytes,
        )
    }

    /// The current state.
    pub fn state(&self) -> State {
        self.state
    }

    /// What the answer settled. Available once the headers arrive.
    pub fn agreement(&self) -> Option<&FeedAgreement> {
        self.agreement.as_ref()
    }

    /// The codec for the sample stream. Available once the headers arrive.
    pub fn codec(&self) -> Option<&Codec> {
        self.codec.as_ref()
    }

    /// Read the answer headers.
    ///
    /// Returns the number of bytes used. A buffer with no complete block
    /// returns `Error::Incomplete`, and the caller reads more bytes.
    pub fn on_headers(&mut self, buf: &[u8]) -> Result<usize, Error> {
        let nl = match buf.iter().position(|&b| b == b'\n') {
            Some(n) => n,
            None => return Err(Error::Incomplete),
        };
        let status = StatusLine::parse(&String::from_utf8_lossy(&buf[..nl]))?;

        // A newer major version stops the feed.
        // `src/data_receiver.cpp:200-203`.
        if status.version / 100 > self.params.version / 100 {
            self.state = State::Failed;
            return Err(Error::VersionTooNew {
                theirs: status.version,
                ours: self.params.version,
            });
        }

        if status.action() != crate::header::StatusAction::Proceed {
            self.state = State::Failed;
            return Err(Error::Status(status));
        }

        let (headers, used) = match parse_block(&buf[nl + 1..]) {
            Some(x) => x,
            None => return Err(Error::Incomplete),
        };

        let agreement = self.read_answer(&headers)?;
        let order = agreement.order;
        self.codec = Some(
            Codec::new(self.format, self.channels, order)
                .with_suppress_subnormals(agreement.suppress_subnormals),
        );
        self.agreement = Some(agreement);
        self.state = State::AwaitingTestPattern;
        Ok(nl + 1 + used)
    }

    fn read_answer(&mut self, h: &Headers) -> Result<FeedAgreement, Error> {
        // The UID must match. `src/data_receiver.cpp:242`.
        let uid = h.get("uid").unwrap_or("").to_string();
        if !uid.is_empty() && uid != self.params.uid {
            self.state = State::Failed;
            return Err(Error::UidMismatch {
                expected: self.params.uid.clone(),
                actual: uid,
            });
        }

        // A value of 0 means the native order of the reader. This case exists
        // for liblsl 1.13 and earlier. `src/data_receiver.cpp:231`.
        let raw = h.get_int("byte-order").unwrap_or(LITTLE as i64) as i32;
        let order_value = if raw == 0 {
            self.params.native_byte_order
        } else {
            raw
        };
        let order = match ByteOrder::from_wire(order_value) {
            Some(o) => o,
            None => {
                self.state = State::Failed;
                return Err(Error::ByteOrderNotSupported(order_value));
            }
        };

        let version = h
            .get_int("data-protocol-version")
            .unwrap_or(self.params.version as i64) as i32;
        if version > self.params.version {
            self.state = State::Failed;
            return Err(Error::VersionTooNew {
                theirs: version,
                ours: self.params.version,
            });
        }
        // A server can agree to a version below the one that was asked for.
        // `src/tcp_server.cpp:576` only refuses a different major version, so
        // a 1.10 request against a server set to 1.00 is answered with 1.00.
        //
        // Protocol 1.00 carries every sample in a Boost archive
        // (`src/sample.cpp:330`), which this implementation does not read. It
        // has to stop here. Without this test the bytes of the archive are
        // decoded as 1.10 samples, and the first error is a wrong tag rather
        // than a wrong version.
        if version < 110 {
            self.state = State::Failed;
            return Err(Error::VersionTooOld { theirs: version });
        }

        Ok(FeedAgreement {
            data_protocol_version: version,
            order,
            suppress_subnormals: h.get_bool("suppress-subnormals").unwrap_or(false),
            uid,
        })
    }

    /// Read the two test pattern samples and compare them. SPEC.md 6.
    ///
    /// The client builds the expected samples itself. A mismatch means the
    /// formats differ, and liblsl stops the feed with a named error.
    ///
    /// Returns the number of bytes used.
    pub fn on_test_pattern(&mut self, buf: &[u8]) -> Result<usize, TestPatternError> {
        let codec = self.codec.as_ref().ok_or(TestPatternError::NotReady)?;
        let mut off = 0usize;
        for (i, pattern) in TEST_PATTERN_OFFSETS.iter().enumerate() {
            let (got, n) = match codec.decode(&buf[off..]) {
                Ok(x) => x,
                Err(lsl_wire::Error::Truncated) => return Err(TestPatternError::Incomplete),
                Err(e) => {
                    self.state = State::Failed;
                    return Err(TestPatternError::Decode(e));
                }
            };
            let want = lsl_wire::test_pattern(self.format, self.channels, *pattern);
            if !same_sample(&got, &want) {
                self.state = State::Failed;
                return Err(TestPatternError::Mismatch {
                    index: i,
                    expected: Box::new(want),
                    actual: Box::new(got),
                });
            }
            off += n;
        }
        self.state = State::Streaming;
        Ok(off)
    }

    /// Read samples from the stream.
    pub fn on_samples(&self, buf: &[u8]) -> Result<(Vec<Sample>, usize), lsl_wire::Error> {
        let codec = self.codec.as_ref().expect("the handshake set the codec");
        codec.decode_all(buf)
    }
}

/// Compare two samples by bit pattern.
///
/// liblsl compares samples with `operator!=`. A direct comparison of floats
/// fails for a value that is not a number, so this compares bits.
fn same_sample(a: &Sample, b: &Sample) -> bool {
    use lsl_wire::Value;
    a.timestamp.to_bits() == b.timestamp.to_bits()
        && a.values.len() == b.values.len()
        && a.values.iter().zip(&b.values).all(|(x, y)| match (x, y) {
            (Value::F32(p), Value::F32(q)) => p.to_bits() == q.to_bits(),
            (Value::F64(p), Value::F64(q)) => p.to_bits() == q.to_bits(),
            _ => x == y,
        })
}

/// A failure while reading the test pattern.
#[derive(Debug, Clone, PartialEq)]
pub enum TestPatternError {
    /// The buffer holds no complete sample yet.
    Incomplete,
    /// The handshake has not reached this stage.
    NotReady,
    /// The codec refused the bytes.
    Decode(lsl_wire::Error),
    /// A sample did not match the pattern.
    ///
    /// liblsl reports `The received test-pattern samples do not match the
    /// specification. The protocol formats are likely incompatible.`
    /// (`src/data_receiver.cpp:294`).
    Mismatch {
        /// Which of the two samples differed.
        index: usize,
        /// The sample that the specification produces.
        expected: Box<Sample>,
        /// The sample that arrived.
        actual: Box<Sample>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::parse_block;
    use crate::negotiate::{negotiate, FeedRequest, Outcome, ServerStream};

    fn handshake(
        format: Format,
        channels: usize,
        client_order: i32,
        client_perf: f64,
    ) -> FeedClient {
        let mut p = FeedParams::new("uid-1", format);
        p.native_byte_order = client_order;
        p.endian_performance = client_perf;
        let (mut client, request) = FeedClient::start(p, format, channels);

        // Drive a server built from the same specification.
        let text = String::from_utf8_lossy(&request);
        let head = text.split("\r\n").next().unwrap().to_string();
        let uid = head.split(' ').nth(1).unwrap_or("").to_string();
        let block_start = request.iter().position(|&b| b == b'\n').unwrap() + 1;
        let (headers, _) = parse_block(&request[block_start..]).expect("a block");

        let srv = ServerStream::new("uid-1", format, LITTLE, 1000.0);
        let req = FeedRequest::from_headers(110, &uid, &headers, srv.channel_bytes);
        let accepted = match negotiate(&req, &srv) {
            Outcome::Accept(a) => a,
            Outcome::Refuse(s) => panic!("refused: {} {}", s.code, s.message),
        };

        let answer = accepted.to_wire(110, "uid-1");
        client.on_headers(answer.as_bytes()).expect("headers");

        // The server then sends the two test pattern samples.
        let codec = *client.codec().expect("a codec");
        let mut bytes = Vec::new();
        for off in TEST_PATTERN_OFFSETS {
            let s = lsl_wire::test_pattern(format, channels, off);
            codec.encode(&s, &mut bytes).expect("encode");
        }
        let used = client.on_test_pattern(&bytes).expect("the test pattern");
        assert_eq!(used, bytes.len());
        client
    }

    #[test]
    fn the_request_carries_every_header_that_liblsl_writes() {
        let p = FeedParams::new("abc", Format::Float32);
        let text = String::from_utf8(p.to_wire()).unwrap();
        for name in [
            "Native-Byte-Order",
            "Endian-Performance",
            "Has-IEEE754-Floats",
            "Supports-Subnormals",
            "Value-Size",
            "Data-Protocol-Version",
            "Max-Buffer-Length",
            "Max-Chunk-Length",
            "Hostname",
            "Source-Id",
            "Session-Id",
        ] {
            assert!(text.contains(name), "the request is missing {name}");
        }
        assert!(text.starts_with("LSL:streamfeed/110 abc\r\n"));
        assert!(text.ends_with("\r\n\r\n"));
    }

    #[test]
    fn a_full_handshake_reaches_the_streaming_state() {
        let c = handshake(Format::Float32, 8, LITTLE, 0.0);
        assert_eq!(c.state(), State::Streaming);
        let a = c.agreement().expect("an agreement");
        assert_eq!(a.data_protocol_version, 110);
        assert_eq!(a.order, ByteOrder::Little);
    }

    #[test]
    fn a_slow_client_makes_the_server_swap() {
        let c = handshake(Format::Double64, 4, BIG, 0.0);
        assert_eq!(c.agreement().unwrap().order, ByteOrder::Big);
    }

    #[test]
    fn a_wrong_uid_in_the_answer_stops_the_feed() {
        let p = FeedParams::new("uid-1", Format::Float32);
        let (mut c, _) = FeedClient::start(p, Format::Float32, 1);
        let answer = "LSL/110 200 OK\r\nUID: another\r\nByte-Order: 1234\r\n\
                      Suppress-Subnormals: 0\r\nData-Protocol-Version: 110\r\n\r\n";
        match c.on_headers(answer.as_bytes()) {
            Err(Error::UidMismatch { actual, .. }) => assert_eq!(actual, "another"),
            other => panic!("expected a UID mismatch, got {other:?}"),
        }
        assert_eq!(c.state(), State::Failed);
    }

    #[test]
    fn a_404_answer_stops_the_feed() {
        let p = FeedParams::new("uid-1", Format::Float32);
        let (mut c, _) = FeedClient::start(p, Format::Float32, 1);
        match c.on_headers(b"LSL/110 404 Not found\r\n\r\n") {
            Err(Error::Status(s)) => assert_eq!(s.code, 404),
            other => panic!("expected a status error, got {other:?}"),
        }
        assert_eq!(c.state(), State::Failed);
    }

    #[test]
    fn a_byte_order_of_zero_means_the_native_order() {
        // The case for liblsl 1.13 and earlier.
        let p = FeedParams::new("uid-1", Format::Float32);
        let (mut c, _) = FeedClient::start(p, Format::Float32, 1);
        let answer = "LSL/110 200 OK\r\nUID: uid-1\r\nByte-Order: 0\r\n\
                      Suppress-Subnormals: 0\r\nData-Protocol-Version: 110\r\n\r\n";
        c.on_headers(answer.as_bytes()).expect("headers");
        assert_eq!(c.agreement().unwrap().order, ByteOrder::native());
    }

    #[test]
    fn a_wrong_test_pattern_stops_the_feed() {
        let p = FeedParams::new("uid-1", Format::Float32);
        let (mut c, _) = FeedClient::start(p, Format::Float32, 2);
        let answer = "LSL/110 200 OK\r\nUID: uid-1\r\nByte-Order: 1234\r\n\
                      Suppress-Subnormals: 0\r\nData-Protocol-Version: 110\r\n\r\n";
        c.on_headers(answer.as_bytes()).expect("headers");

        // Send the second pattern first. A wrong implementation does this.
        let codec = *c.codec().unwrap();
        let mut bytes = Vec::new();
        for off in [2, 4] {
            codec
                .encode(&lsl_wire::test_pattern(Format::Float32, 2, off), &mut bytes)
                .unwrap();
        }
        match c.on_test_pattern(&bytes) {
            Err(TestPatternError::Mismatch { index, .. }) => assert_eq!(index, 0),
            other => panic!("expected a mismatch, got {other:?}"),
        }
        assert_eq!(c.state(), State::Failed);
    }

    #[test]
    fn an_incomplete_buffer_asks_for_more() {
        let p = FeedParams::new("uid-1", Format::Float32);
        let (mut c, _) = FeedClient::start(p, Format::Float32, 1);
        assert_eq!(
            c.on_headers(b"LSL/110 200 OK\r\nUID: uid-1\r\n"),
            Err(Error::Incomplete)
        );
        assert_eq!(c.state(), State::AwaitingHeaders);
    }
}
