//! Discovery over UDP. SPEC.md 2.

/// The default base port. `src/api_config.cpp:170`.
pub const BASE_PORT: u16 = 16572;
/// The default port range. `src/api_config.cpp:171`.
pub const PORT_RANGE: u16 = 32;

/// The default multicast and broadcast addresses.
/// `src/api_config.cpp:196-202`.
pub const LINK_ADDRESSES: [&str; 3] = ["255.255.255.255", "224.0.0.1", "224.0.0.183"];
/// The default site address.
pub const SITE_ADDRESSES: [&str; 1] = ["239.255.172.215"];

/// Build a shortinfo query. SPEC.md 2.1.
///
/// The query identifier is an opaque round-trip token. liblsl builds it from
/// `std::hash`, whose value differs between standard libraries. A caller
/// passes any value and compares the answer with what it sent. SPEC.md 2.3.
pub fn build_query(query: &str, return_port: u16, query_id: &str) -> Vec<u8> {
    format!("LSL:shortinfo\r\n{query}\r\n{return_port} {query_id}\r\n").into_bytes()
}

/// A parsed shortinfo query, as an outlet reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// The XPath query. An empty value matches every stream.
    pub query: String,
    /// The port that the answer goes to.
    pub return_port: u16,
    /// The token to echo.
    pub query_id: String,
}

/// Read a shortinfo query.
///
/// liblsl reads the query with `getline` and then reads the port and the
/// identifier with the stream operator (`src/udp_server.cpp`,
/// `process_shortinfo_request`).
pub fn parse_query(buf: &[u8]) -> Option<Query> {
    let text = String::from_utf8_lossy(buf);
    let mut lines = text.split('\n');
    let method = lines.next()?.trim_end_matches('\r').trim();
    if method != "LSL:shortinfo" {
        return None;
    }
    let query = lines.next()?.trim_end_matches('\r').trim().to_string();
    let tail = lines.next()?;
    let mut parts = tail.split_whitespace();
    let return_port = parts.next()?.parse().ok()?;
    let query_id = parts.next().unwrap_or("").to_string();
    Some(Query {
        query,
        return_port,
        query_id,
    })
}

/// Build a shortinfo answer. SPEC.md 2.2.
pub fn build_answer(query_id: &str, shortinfo_xml: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(query_id.len() + 2 + shortinfo_xml.len());
    v.extend_from_slice(query_id.as_bytes());
    v.extend_from_slice(b"\r\n");
    v.extend_from_slice(shortinfo_xml.as_bytes());
    v
}

/// A parsed shortinfo answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The token that came back.
    pub query_id: String,
    /// The XML description of the stream.
    pub xml: String,
}

/// Read a shortinfo answer.
///
/// The caller compares `query_id` with the value that it sent. A mismatch
/// means the answer belongs to another query, and liblsl drops it
/// (`src/resolve_attempt_udp.cpp:114`).
pub fn parse_answer(buf: &[u8]) -> Option<Answer> {
    let nl = buf.iter().position(|&b| b == b'\n')?;
    let id = String::from_utf8_lossy(&buf[..nl]).trim().to_string();
    let xml = String::from_utf8_lossy(&buf[nl + 1..]).to_string();
    Some(Answer { query_id: id, xml })
}

/// The fields that an inlet reads out of a short description.
///
/// This is a small reader and not a general XML parser. The fields sit at a
/// fixed depth, and every one of them holds text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShortInfo {
    /// The stream name.
    pub name: String,
    /// The content type.
    pub stream_type: String,
    /// The channel count.
    pub channel_count: u32,
    /// The channel format, as text.
    pub channel_format: String,
    /// The stream instance identifier.
    pub uid: String,
    /// The source identifier.
    pub source_id: String,
    /// The nominal rate.
    pub nominal_srate: f64,
    /// The host name of the sender.
    pub hostname: String,
    /// The TCP port that carries the data.
    pub v4data_port: u16,
    /// The UDP port that answers a time probe.
    pub v4service_port: u16,
}

fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let a = xml.find(&open)? + open.len();
    let b = xml[a..].find(&close)? + a;
    Some(&xml[a..b])
}

impl ShortInfo {
    /// Read the fields out of a short description.
    pub fn parse(xml: &str) -> Option<Self> {
        Some(ShortInfo {
            name: tag(xml, "name")?.to_string(),
            stream_type: tag(xml, "type").unwrap_or("").to_string(),
            channel_count: tag(xml, "channel_count")?.trim().parse().ok()?,
            channel_format: tag(xml, "channel_format")?.trim().to_string(),
            uid: tag(xml, "uid")?.to_string(),
            source_id: tag(xml, "source_id").unwrap_or("").to_string(),
            nominal_srate: tag(xml, "nominal_srate")
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0.0),
            hostname: tag(xml, "hostname").unwrap_or("").to_string(),
            v4data_port: tag(xml, "v4data_port")?.trim().parse().ok()?,
            v4service_port: tag(xml, "v4service_port")?.trim().parse().ok()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "<?xml version=\"1.0\"?><info><name>Probe</name><type>EEG</type>\
        <channel_count>8</channel_count><channel_format>float32</channel_format>\
        <source_id>src</source_id><nominal_srate>100.0000000000000</nominal_srate>\
        <uid>25cfa701-e55b-4e42-8fc2-103bbd2f86b6</uid><session_id>default</session_id>\
        <hostname>box</hostname><v4data_port>16572</v4data_port>\
        <v4service_port>16572</v4service_port></info>";

    #[test]
    fn a_query_round_trips() {
        let bytes = build_query("name='Probe'", 40000, "token-1");
        assert_eq!(
            String::from_utf8_lossy(&bytes),
            "LSL:shortinfo\r\nname='Probe'\r\n40000 token-1\r\n"
        );
        let q = parse_query(&bytes).expect("a query");
        assert_eq!(q.query, "name='Probe'");
        assert_eq!(q.return_port, 40000);
        assert_eq!(q.query_id, "token-1");
    }

    #[test]
    fn an_empty_query_survives_the_round_trip() {
        let bytes = build_query("", 1, "t");
        let q = parse_query(&bytes).expect("a query");
        assert_eq!(q.query, "");
    }

    #[test]
    fn another_method_is_not_a_query() {
        assert_eq!(parse_query(b"LSL:timedata\r\n1 2.0\r\n"), None);
    }

    #[test]
    fn an_answer_round_trips() {
        let bytes = build_answer("token-1", SAMPLE);
        let a = parse_answer(&bytes).expect("an answer");
        assert_eq!(a.query_id, "token-1");
        assert_eq!(a.xml, SAMPLE);
    }

    #[test]
    fn the_reader_finds_every_field() {
        let s = ShortInfo::parse(SAMPLE).expect("a description");
        assert_eq!(s.name, "Probe");
        assert_eq!(s.channel_count, 8);
        assert_eq!(s.channel_format, "float32");
        assert_eq!(s.uid, "25cfa701-e55b-4e42-8fc2-103bbd2f86b6");
        assert_eq!(s.v4data_port, 16572);
        assert_eq!(s.v4service_port, 16572);
        assert!((s.nominal_srate - 100.0).abs() < 1e-9);
    }
}
