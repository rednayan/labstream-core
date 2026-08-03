//! The feed negotiation. SPEC.md 5.
//!
//! The server decides three things: the data protocol version, the byte order,
//! and subnormal suppression. This module holds that decision as a pure
//! function, so a test can drive every branch with no socket.

use crate::header::{Headers, StatusLine};
use labstream_wire::{ByteOrder, Format};

/// The version constant of this implementation. `src/common.h:46`.
pub const LSL_PROTOCOL_VERSION: i32 = 110;

/// The byte order values that liblsl accepts. `src/util/endian.hpp:10-17`.
pub const LITTLE: i32 = 1234;
/// The big endian value.
pub const BIG: i32 = 4321;

/// True when liblsl can convert to the requested order.
///
/// `src/util/endian.hpp:23-30`. A one-byte value always passes, because no
/// conversion takes place. Any other size needs a known order on both sides.
pub fn can_convert_endian(requested: i32, value_size: i32) -> bool {
    if value_size == 1 {
        return true;
    }
    requested == LITTLE || requested == BIG
}

/// What the client asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedRequest {
    /// The version from the request line, such as 110.
    pub request_version: i32,
    /// The UID from the request line. An empty value skips the UID test.
    pub request_uid: String,
    /// `Native-Byte-Order`.
    pub native_byte_order: i32,
    /// `Endian-Performance`. A measured value that a test must never compare.
    pub endian_performance: f64,
    /// `Has-IEEE754-Floats`.
    pub has_ieee754_floats: bool,
    /// `Supports-Subnormals`.
    pub supports_subnormals: bool,
    /// `Value-Size`. Zero for a string stream.
    pub value_size: i32,
    /// `Max-Buffer-Length`.
    pub max_buffer_length: i32,
    /// `Max-Chunk-Length`.
    pub max_chunk_length: i32,
    /// The version from `Protocol-Version`.
    ///
    /// The server reads this name and not `Data-Protocol-Version`
    /// (`src/tcp_server.cpp:628`). SPEC.md 4.4 records the difference. The
    /// default is the version from the request line
    /// (`src/tcp_server.cpp:596-597`).
    pub protocol_version: i32,
}

impl FeedRequest {
    /// Read a request from a request line version, a UID, and a header block.
    ///
    /// Every default matches `src/tcp_server.cpp:590-600`. A missing header
    /// keeps its default, and a wrong header name is the same as a missing one.
    pub fn from_headers(
        request_version: i32,
        request_uid: &str,
        h: &Headers,
        channel_bytes: i32,
    ) -> Self {
        FeedRequest {
            request_version,
            request_uid: request_uid.to_string(),
            native_byte_order: h.get_int("native-byte-order").unwrap_or(LITTLE as i64) as i32,
            endian_performance: h.get_f64("endian-performance").unwrap_or(0.0),
            has_ieee754_floats: h.get_bool("has-ieee754-floats").unwrap_or(true),
            supports_subnormals: h.get_bool("supports-subnormals").unwrap_or(true),
            value_size: h.get_int("value-size").unwrap_or(channel_bytes as i64) as i32,
            max_buffer_length: h.get_int("max-buffer-length").unwrap_or(0) as i32,
            max_chunk_length: h.get_int("max-chunk-length").unwrap_or(0) as i32,
            // This reads `protocol-version`. A client that sends only
            // `Data-Protocol-Version` leaves the default in place.
            protocol_version: h
                .get_int("protocol-version")
                .unwrap_or(request_version as i64) as i32,
        }
    }
}

/// What the server knows about its own stream.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerStream {
    /// The stream UID.
    pub uid: String,
    /// The channel format.
    pub format: Format,
    /// The size of one channel value in bytes. Zero for a string stream.
    pub channel_bytes: i32,
    /// The version that this server runs.
    pub configured_version: i32,
    /// The byte order of this machine.
    pub native_byte_order: i32,
    /// How fast this machine converts a byte order.
    ///
    /// A measured value. A test injects a fixed number.
    pub endian_performance: f64,
}

impl ServerStream {
    /// Build a server description with the usual values.
    pub fn new(uid: &str, format: Format, native_byte_order: i32, endian_performance: f64) -> Self {
        ServerStream {
            uid: uid.to_string(),
            format,
            channel_bytes: format.width().unwrap_or(0) as i32,
            configured_version: LSL_PROTOCOL_VERSION,
            native_byte_order,
            endian_performance,
        }
    }
}

/// What the server decided.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The server refuses, and it sends this status line.
    Refuse(StatusLine),
    /// The server accepts.
    Accept(Accepted),
}

/// The accepted parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct Accepted {
    /// The negotiated data protocol version.
    pub data_protocol_version: i32,
    /// The byte order that goes in the answer.
    pub byte_order: i32,
    /// True when the server writes swapped values.
    pub reverse_byte_order: bool,
    /// True when the reader clears subnormal values.
    pub suppress_subnormals: bool,
    /// The buffer length that the client asked for.
    pub max_buffer_length: i32,
    /// The chunk length that the client asked for.
    pub max_chunk_length: i32,
}

impl Accepted {
    /// The byte order as the codec sees it.
    pub fn codec_order(&self) -> Option<ByteOrder> {
        match self.byte_order {
            LITTLE => Some(ByteOrder::Little),
            BIG => Some(ByteOrder::Big),
            _ => None,
        }
    }

    /// Write the answer as liblsl writes it. `src/tcp_server.cpp:673-678`.
    pub fn to_wire(&self, configured_version: i32, uid: &str) -> String {
        let mut s = StatusLine {
            version: configured_version,
            code: 200,
            message: "OK".into(),
        }
        .to_wire();
        s.push_str(&format!("UID: {uid}\r\n"));
        s.push_str(&format!("Byte-Order: {}\r\n", self.byte_order));
        s.push_str(&format!(
            "Suppress-Subnormals: {}\r\n",
            if self.suppress_subnormals { 1 } else { 0 }
        ));
        s.push_str(&format!(
            "Data-Protocol-Version: {}\r\n",
            self.data_protocol_version
        ));
        s.push_str("\r\n");
        s
    }
}

/// True when the format carries a subnormal value.
///
/// liblsl calls this table `format_subnormal` (`src/sample.h:28`). Only the
/// two floating point formats hold one.
fn format_has_subnormals(f: Format) -> bool {
    f.is_float()
}

/// Decide what the server does with a request. SPEC.md 5.
///
/// This function reads no socket and no clock. Every input is an argument, so
/// a test can drive each branch.
pub fn negotiate(req: &FeedRequest, srv: &ServerStream) -> Outcome {
    // A newer major version stops the feed. `src/tcp_server.cpp:576-580`.
    if req.request_version / 100 > srv.configured_version / 100 {
        return Outcome::Refuse(StatusLine {
            version: srv.configured_version,
            code: 505,
            message: "Version not supported".into(),
        });
    }

    // A UID that names another stream stops the feed.
    // `src/tcp_server.cpp:585-588`. An empty UID skips the test.
    if !req.request_uid.is_empty() && req.request_uid != srv.uid {
        return Outcome::Refuse(StatusLine {
            version: srv.configured_version,
            code: 404,
            message: "Not found".into(),
        });
    }

    // A request below 1.10 carries no headers. `src/tcp_server.cpp:679-680`.
    if req.request_version < 110 {
        return Outcome::Accept(Accepted {
            data_protocol_version: 100,
            byte_order: srv.native_byte_order,
            reverse_byte_order: false,
            suppress_subnormals: false,
            max_buffer_length: req.max_buffer_length,
            max_chunk_length: req.max_chunk_length,
        });
    }

    // The lower of the two versions wins. `src/tcp_server.cpp:641`.
    let mut version = srv.configured_version.min(req.protocol_version);

    // Two conditions force protocol 1.00. `src/tcp_server.cpp:643-649`.
    if srv.format != Format::String && srv.channel_bytes != req.value_size {
        version = 100;
    }
    if !req.has_ieee754_floats {
        version = 100;
    }

    let mut byte_order = srv.native_byte_order;
    let mut reverse = false;
    let mut suppress = false;

    if version >= 110 {
        // Four conditions, all required. `src/tcp_server.cpp:652-664`.
        if srv.native_byte_order != req.native_byte_order
            && can_convert_endian(req.native_byte_order, req.value_size)
            && req.value_size > 1
            && srv.endian_performance > req.endian_performance
        {
            byte_order = req.native_byte_order;
            reverse = true;
        }
        // `src/tcp_server.cpp:667-668`.
        suppress = format_has_subnormals(srv.format) && !req.supports_subnormals;
    }

    Outcome::Accept(Accepted {
        data_protocol_version: version,
        byte_order,
        reverse_byte_order: reverse,
        suppress_subnormals: suppress,
        max_buffer_length: req.max_buffer_length,
        max_chunk_length: req.max_chunk_length,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> ServerStream {
        ServerStream::new("uid-1", Format::Float32, LITTLE, 1000.0)
    }

    fn request() -> FeedRequest {
        FeedRequest {
            request_version: 110,
            request_uid: "uid-1".into(),
            native_byte_order: LITTLE,
            endian_performance: 500.0,
            has_ieee754_floats: true,
            supports_subnormals: true,
            value_size: 4,
            max_buffer_length: 360,
            max_chunk_length: 0,
            protocol_version: 110,
        }
    }

    fn accepted(o: Outcome) -> Accepted {
        match o {
            Outcome::Accept(a) => a,
            Outcome::Refuse(s) => panic!("refused with {} {}", s.code, s.message),
        }
    }

    #[test]
    fn the_usual_request_is_accepted_with_no_swap() {
        let a = accepted(negotiate(&request(), &server()));
        assert_eq!(a.data_protocol_version, 110);
        assert_eq!(a.byte_order, LITTLE);
        assert!(!a.reverse_byte_order);
        assert!(!a.suppress_subnormals);
    }

    #[test]
    fn a_newer_major_version_gets_505() {
        let mut r = request();
        r.request_version = 200;
        match negotiate(&r, &server()) {
            Outcome::Refuse(s) => {
                assert_eq!(s.code, 505);
                assert_eq!(s.message, "Version not supported");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_newer_minor_version_is_accepted() {
        // 111 has the same major version as 110, so it passes the first test.
        let mut r = request();
        r.request_version = 111;
        r.protocol_version = 111;
        let a = accepted(negotiate(&r, &server()));
        assert_eq!(a.data_protocol_version, 110);
    }

    #[test]
    fn a_wrong_uid_gets_404() {
        let mut r = request();
        r.request_uid = "another".into();
        match negotiate(&r, &server()) {
            Outcome::Refuse(s) => assert_eq!(s.code, 404),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_uid_skips_the_test() {
        let mut r = request();
        r.request_uid = String::new();
        assert_eq!(
            accepted(negotiate(&r, &server())).data_protocol_version,
            110
        );
    }

    #[test]
    fn a_value_size_mismatch_forces_protocol_100() {
        let mut r = request();
        r.value_size = 8;
        assert_eq!(
            accepted(negotiate(&r, &server())).data_protocol_version,
            100
        );
    }

    #[test]
    fn a_string_stream_ignores_the_value_size() {
        let mut s = server();
        s.format = Format::String;
        s.channel_bytes = 0;
        let mut r = request();
        r.value_size = 0;
        assert_eq!(accepted(negotiate(&r, &s)).data_protocol_version, 110);
    }

    #[test]
    fn a_client_with_no_ieee754_floats_forces_protocol_100() {
        let mut r = request();
        r.has_ieee754_floats = false;
        assert_eq!(
            accepted(negotiate(&r, &server())).data_protocol_version,
            100
        );
    }

    #[test]
    fn the_server_swaps_when_it_is_faster() {
        let mut r = request();
        r.native_byte_order = BIG;
        r.endian_performance = 0.0;
        let a = accepted(negotiate(&r, &server()));
        assert_eq!(a.byte_order, BIG);
        assert!(a.reverse_byte_order);
    }

    #[test]
    fn the_server_does_not_swap_when_the_client_is_faster() {
        let mut r = request();
        r.native_byte_order = BIG;
        r.endian_performance = 999999.0;
        let a = accepted(negotiate(&r, &server()));
        assert_eq!(a.byte_order, LITTLE);
        assert!(!a.reverse_byte_order);
    }

    #[test]
    fn a_one_byte_format_never_swaps() {
        // Condition 3 needs a size above 1. This is the rule that makes the
        // swapped one-byte stream unreachable in M2.
        let mut s = server();
        s.format = Format::Int8;
        s.channel_bytes = 1;
        let mut r = request();
        r.value_size = 1;
        r.native_byte_order = BIG;
        r.endian_performance = 0.0;
        let a = accepted(negotiate(&r, &s));
        assert_eq!(a.byte_order, LITTLE);
        assert!(!a.reverse_byte_order);
    }

    #[test]
    fn an_unknown_byte_order_blocks_the_swap() {
        let mut r = request();
        r.native_byte_order = 2134; // the deprecated PDP order
        r.endian_performance = 0.0;
        let a = accepted(negotiate(&r, &server()));
        assert_eq!(a.byte_order, LITTLE);
        assert!(!a.reverse_byte_order);
    }

    #[test]
    fn subnormal_suppression_follows_the_client() {
        let mut r = request();
        r.supports_subnormals = false;
        assert!(accepted(negotiate(&r, &server())).suppress_subnormals);

        // An integer format holds no subnormal value.
        let mut s = server();
        s.format = Format::Int32;
        assert!(!accepted(negotiate(&r, &s)).suppress_subnormals);
    }

    #[test]
    fn the_server_reads_protocol_version_and_not_data_protocol_version() {
        // SPEC.md 4.4. This is the header that liblsl reads.
        use crate::header::parse_block;

        let with_dead = b"Data-Protocol-Version: 100\r\nValue-Size: 4\r\n\r\n";
        let (h, _) = parse_block(with_dead).unwrap();
        let r = FeedRequest::from_headers(110, "uid-1", &h, 4);
        assert_eq!(
            r.protocol_version, 110,
            "the dead header must not change the version"
        );
        assert_eq!(
            accepted(negotiate(&r, &server())).data_protocol_version,
            110
        );

        let with_live = b"Protocol-Version: 100\r\nValue-Size: 4\r\n\r\n";
        let (h, _) = parse_block(with_live).unwrap();
        let r = FeedRequest::from_headers(110, "uid-1", &h, 4);
        assert_eq!(r.protocol_version, 100);
        assert_eq!(
            accepted(negotiate(&r, &server())).data_protocol_version,
            100
        );
    }

    #[test]
    fn the_answer_matches_the_shape_that_liblsl_writes() {
        let a = accepted(negotiate(&request(), &server()));
        let wire = a.to_wire(110, "uid-1");
        assert_eq!(
            wire,
            "LSL/110 200 OK\r\nUID: uid-1\r\nByte-Order: 1234\r\n\
             Suppress-Subnormals: 0\r\nData-Protocol-Version: 110\r\n\r\n"
        );
    }
}
