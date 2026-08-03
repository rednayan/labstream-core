//! The stream description, and the query subset that a resolver sends.

use lsl_wire::Format;

/// The description of one stream.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamInfo {
    /// The stream name.
    pub name: String,
    /// The content type.
    pub stream_type: String,
    /// The channel count.
    pub channel_count: u32,
    /// The channel format.
    pub format: Format,
    /// The nominal rate. Zero means an irregular rate.
    pub nominal_srate: f64,
    /// The source identifier. An empty value blocks automatic recovery.
    pub source_id: String,
    /// The session identifier. A resolver query always tests this field.
    pub session_id: String,
    /// The host name of the sender.
    pub hostname: String,
    /// The stream instance identifier.
    pub uid: String,
    /// The clock reading at creation.
    pub created_at: f64,
    /// The address that the answer to a query came from.
    ///
    /// An outlet leaves this empty, and the resolver of the peer fills it in
    /// from the sender of the answer (`src/resolve_attempt_udp.cpp:135-140`).
    /// An inlet connects to this address, so a description with an empty one
    /// can only be reached on the machine that published it.
    pub v4address: String,
    /// The IPv6 address that the answer came from. Same rule as
    /// [`StreamInfo::v4address`].
    pub v6address: String,
    /// The TCP port that carries the data.
    pub v4data_port: u16,
    /// The UDP port that answers a query and a time probe.
    pub v4service_port: u16,
    /// The TCP port that carries the data over IPv6.
    pub v6data_port: u16,
    /// The UDP port that answers over IPv6.
    pub v6service_port: u16,
    /// The description tree.
    ///
    /// A discovery answer never carries this. It travels on the data port, in
    /// answer to `LSL:fullinfo`. The node itself is the `<desc>` element, so
    /// its children are what an application added.
    pub desc: crate::desc::Node,
    /// True when this description came from a document rather than from a
    /// program.
    ///
    /// The two write an empty field differently. A program builds
    /// `<v4address></v4address>`, because `write_xml` appends an empty text
    /// node (`src/stream_info_impl.cpp:56`). A reader drops that text node, so
    /// the same field writes back as `<v4address />`. SPEC.md 12.4.
    ///
    /// `HandleMetaData.cpp`, one of the example programs of liblsl, shows the
    /// difference: it prints the document that an inlet received.
    from_document: bool,
}

/// The format name that liblsl writes in a description.
///
/// `src/stream_info_impl.cpp` writes these exact words. A different word makes
/// the stream unreadable to a real inlet.
pub fn format_name(f: Format) -> &'static str {
    match f {
        Format::Float32 => "float32",
        Format::Double64 => "double64",
        Format::String => "string",
        Format::Int32 => "int32",
        Format::Int16 => "int16",
        Format::Int8 => "int8",
        Format::Int64 => "int64",
        Format::Undefined => "undefined",
    }
}

/// Read a format name.
pub fn format_from_name(s: &str) -> Option<Format> {
    Some(match s {
        "float32" => Format::Float32,
        "double64" => Format::Double64,
        "string" => Format::String,
        "int32" => Format::Int32,
        "int16" => Format::Int16,
        "int8" => Format::Int8,
        "int64" => Format::Int64,
        _ => return None,
    })
}

impl StreamInfo {
    /// Build a description with the usual values.
    pub fn new(
        name: &str,
        stream_type: &str,
        channel_count: u32,
        format: Format,
        srate: f64,
    ) -> Self {
        StreamInfo {
            name: name.to_string(),
            stream_type: stream_type.to_string(),
            channel_count,
            format,
            nominal_srate: srate,
            source_id: String::new(),
            // The session identifier arrives when the stream is published, not
            // when the description is built. `src/tcp_server.cpp:328` sets it
            // from the configuration as the server starts, so a description
            // that no outlet published reports an empty value.
            session_id: String::new(),
            hostname: hostname(),
            uid: String::new(),
            created_at: 0.0,
            v4address: String::new(),
            v6address: String::new(),
            v4data_port: 0,
            v4service_port: 0,
            v6data_port: 0,
            v6service_port: 0,
            desc: crate::desc::Node::element("desc"),
            from_document: false,
        }
    }

    /// Write the short description that a discovery answer carries.
    ///
    /// The field order and the number format follow the capture in
    /// `captures/discover.json`. A real inlet reads this with pugixml, so it
    /// needs valid XML and the exact field names.
    pub fn to_shortinfo_xml(&self) -> String {
        self.to_xml("\t<desc />\n")
    }

    /// Write the whole document, with the description tree.
    ///
    /// This is what a data port returns for `LSL:fullinfo`
    /// (`src/tcp_server.cpp:508`), and what `lsl_get_xml` returns
    /// (`src/lsl_streaminfo_c.cpp:63`). The only difference from the short form
    /// is the content of `<desc>`.
    pub fn to_fullinfo_xml(&self) -> String {
        let mut body = String::new();
        let mut node = self.desc.clone();
        node.name = "desc".to_string();
        node.write(&mut body, 1);
        self.to_xml(&body)
    }

    fn to_xml(&self, desc_body: &str) -> String {
        // A reader drops an empty text node, so a description that came from a
        // document writes an empty field as one empty tag. SPEC.md 12.4.
        let field = |name: &str, value: String| -> String {
            if value.is_empty() && self.from_document {
                format!("\t<{name} />\n")
            } else {
                format!("\t<{name}>{value}</{name}>\n")
            }
        };
        let mut out = String::from("<?xml version=\"1.0\"?>\n<info>\n");
        out.push_str(&field("name", escape(&self.name)));
        out.push_str(&field("type", escape(&self.stream_type)));
        out.push_str(&field("channel_count", self.channel_count.to_string()));
        out.push_str(&field(
            "channel_format",
            format_name(self.format).to_string(),
        ));
        out.push_str(&field("source_id", escape(&self.source_id)));
        out.push_str(&field("nominal_srate", show_point_16(self.nominal_srate)));
        out.push_str(&field("version", "1.100000000000000".to_string()));
        out.push_str(&field("created_at", show_point_16(self.created_at)));
        out.push_str(&field("uid", escape(&self.uid)));
        out.push_str(&field("session_id", escape(&self.session_id)));
        out.push_str(&field("hostname", escape(&self.hostname)));
        out.push_str(&field("v4address", escape(&self.v4address)));
        out.push_str(&field("v4data_port", self.v4data_port.to_string()));
        out.push_str(&field("v4service_port", self.v4service_port.to_string()));
        out.push_str(&field("v6address", escape(&self.v6address)));
        out.push_str(&field("v6data_port", self.v6data_port.to_string()));
        out.push_str(&field("v6service_port", self.v6service_port.to_string()));
        out.push_str(desc_body);
        out.push_str("</info>\n");
        out
    }

    /// Read a short description.
    pub fn from_shortinfo_xml(xml: &str) -> Option<Self> {
        let s = lsl_proto::discovery::ShortInfo::parse(xml)?;
        Some(StreamInfo {
            name: s.name,
            stream_type: s.stream_type,
            channel_count: s.channel_count,
            format: format_from_name(&s.channel_format)?,
            nominal_srate: s.nominal_srate,
            source_id: s.source_id,
            session_id: tag(xml, "session_id").unwrap_or("default").to_string(),
            hostname: s.hostname,
            uid: s.uid,
            created_at: tag(xml, "created_at")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0.0),
            v4address: tag(xml, "v4address").unwrap_or("").trim().to_string(),
            v6address: tag(xml, "v6address").unwrap_or("").trim().to_string(),
            v4data_port: s.v4data_port,
            v4service_port: s.v4service_port,
            v6data_port: tag(xml, "v6data_port")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0),
            v6service_port: tag(xml, "v6service_port")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0),
            desc: crate::desc::Node::element("desc"),
            from_document: true,
        })
    }

    /// Read a whole document, with the description tree.
    ///
    /// The checks and their order follow `read_xml`
    /// (`src/stream_info_impl.cpp:99-155`). The text of the error is the text
    /// that liblsl throws, because the C ABI shows it: a rejected document
    /// leaves a blank description whose name reads `(invalid: TEXT)`.
    /// `captures/desc/desc_nouid.name` holds one measured example.
    pub fn from_fullinfo_xml(xml: &str) -> Result<Self, String> {
        let root = crate::desc::parse(xml).ok_or_else(|| "the document is not XML".to_string())?;
        let info = if root.name == "info" {
            root
        } else {
            root.child("info")
                .cloned()
                .ok_or_else(|| "no <info> element".to_string())?
        };

        let text = |k: &str| info.child_value_of(k).to_string();

        let name = text("name");
        if name.is_empty() {
            return Err("Received a stream info with empty <name> field.".into());
        }
        let channel_count = bounded(&text("channel_count"), "channel_count", 0, 0)? as u32;
        bounded(&text("nominal_srate"), "nominal_srate", 0, 0)?;
        let nominal_srate = stod(&text("nominal_srate"))?;
        let fmt = text("channel_format");
        let format =
            format_from_name(&fmt).ok_or_else(|| format!("Invalid channel format {fmt}"))?;
        if (stod(&text("version"))? * 100.0) as i32 <= 0 {
            return Err("The version of the given stream info is invalid.".into());
        }
        let created_at = stod(&text("created_at"))?;
        let uid = text("uid");
        if uid.is_empty() {
            return Err("The UID of the given stream info is empty.".into());
        }

        Ok(StreamInfo {
            name,
            stream_type: text("type"),
            channel_count,
            format,
            nominal_srate,
            source_id: text("source_id"),
            session_id: text("session_id"),
            hostname: text("hostname"),
            uid,
            created_at,
            v4address: text("v4address"),
            v6address: text("v6address"),
            v4data_port: bounded(&text("v4data_port"), "v4data_port", 0, 65535)? as u16,
            v4service_port: bounded(&text("v4service_port"), "v4service_port", 0, 65535)? as u16,
            v6data_port: bounded(&text("v6data_port"), "v6data_port", 0, 65535)? as u16,
            v6service_port: bounded(&text("v6service_port"), "v6service_port", 0, 65535)? as u16,
            desc: info
                .child("desc")
                .cloned()
                .unwrap_or_else(|| crate::desc::Node::element("desc")),
            from_document: true,
        })
    }

    /// The description as a tree, with the `<info>` element at the root.
    ///
    /// A query is evaluated against this (`src/stream_info_impl.cpp:214`). The
    /// text of every field is the text that the same field carries on the
    /// wire, so a query that names a number reads the same value that a peer
    /// would read.
    pub fn document(&self) -> crate::desc::Node {
        let mut info = crate::desc::Node::element("info");
        let mut put = |name: &str, value: String| {
            info.append_child_value(name, &value);
        };
        put("name", self.name.clone());
        put("type", self.stream_type.clone());
        put("channel_count", self.channel_count.to_string());
        put("channel_format", format_name(self.format).to_string());
        put("source_id", self.source_id.clone());
        put("nominal_srate", show_point_16(self.nominal_srate));
        put("version", "1.100000000000000".to_string());
        put("created_at", show_point_16(self.created_at));
        put("uid", self.uid.clone());
        put("session_id", self.session_id.clone());
        put("hostname", self.hostname.clone());
        put("v4address", self.v4address.clone());
        put("v4data_port", self.v4data_port.to_string());
        put("v4service_port", self.v4service_port.to_string());
        put("v6address", self.v6address.clone());
        put("v6data_port", self.v6data_port.to_string());
        put("v6service_port", self.v6service_port.to_string());
        let mut tree = self.desc.clone();
        tree.name = "desc".to_string();
        info.children.push(tree);
        info
    }

    /// Test a query against this description.
    ///
    /// The query is XPath, the way liblsl reads it
    /// (`src/stream_info_impl.cpp:214`). See [`crate::xpath`] for what the
    /// reader covers and what it refuses.
    pub fn matches(&self, query: &str) -> bool {
        crate::xpath::matches(query, &self.document())
    }
}

/// Write a double the way liblsl writes one into a description.
///
/// `src/util/cast.cpp:9` builds the text with `setprecision(16)` and
/// `showpoint` on a default stream. That is the general format with 16
/// significant digits, and no trailing zero is removed. The rule:
///
/// * If the decimal exponent is below -4 or 16 and above, use the scientific
///   form with 15 digits after the point.
/// * If not, use the fixed form with `15 - exponent` digits after the point.
///
/// So a rate of 100 writes as `100.0000000000000` and a rate of 0 writes as
/// `0.000000000000000`. Both are 16 significant digits, and both appear in
/// `captures/desc/`.
pub fn show_point_16(v: f64) -> String {
    if v.is_nan() {
        return "nan".to_string();
    }
    if v.is_infinite() {
        return if v < 0.0 { "-inf" } else { "inf" }.to_string();
    }
    let sci = format!("{v:.15e}");
    let (mantissa, exp_text) = match sci.split_once('e') {
        Some(x) => x,
        None => return sci,
    };
    let exp: i32 = exp_text.parse().unwrap_or(0);
    if !(-4..16).contains(&exp) {
        let sign = if exp < 0 { '-' } else { '+' };
        return format!("{mantissa}e{sign}{:02}", exp.abs());
    }
    format!("{:.*}", (15 - exp) as usize, v)
}

/// Read a double the way `std::stod` reads one.
///
/// The reader takes the longest leading run that looks like a number and
/// rejects text that starts with anything else. liblsl lets that rejection
/// escape, and the whole description is then thrown away.
fn stod(text: &str) -> Result<f64, String> {
    let t = text.trim();
    let mut end = 0;
    for (i, c) in t.char_indices() {
        let keep = c.is_ascii_digit()
            || (i == 0 && (c == '-' || c == '+'))
            || c == '.'
            || c == 'e'
            || c == 'E'
            || ((c == '-' || c == '+')
                && matches!(t.as_bytes().get(i - 1), Some(b'e') | Some(b'E')));
        if !keep {
            break;
        }
        end = i + c.len_utf8();
    }
    t[..end].parse().map_err(|_| "stod".to_string())
}

/// Read a whole number and hold it inside a range.
///
/// liblsl reads these fields with `std::stoi`, which stops at the first
/// character that is not a digit. `100.0000000000000` therefore reads as 100
/// and never throws. `get_bounded_child_val` then tests the range, and a zero
/// maximum means no upper bound (`src/stream_info_impl.cpp:85`).
fn bounded(text: &str, field: &str, min: i32, max: i32) -> Result<i32, String> {
    let t = text.trim();
    let digits: String = t
        .char_indices()
        .take_while(|(i, c)| c.is_ascii_digit() || (*i == 0 && (*c == '-' || *c == '+')))
        .map(|(_, c)| c)
        .collect();
    let value: i32 = digits.parse().map_err(|_| "stoi".to_string())?;
    if value < min || (max != 0 && value > max) {
        let mut msg = format!("{field} must be >={min}");
        if max != 0 {
            msg.push_str(&format!(" and <={max}"));
        }
        return Err(msg);
    }
    Ok(value)
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let a = xml.find(&open)? + open.len();
    let b = xml[a..].find(&close)? + a;
    Some(&xml[a..b])
}

/// The host name of this machine, or `unknown`.
pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> StreamInfo {
        let mut i = StreamInfo::new("Probe", "EEG", 8, Format::Float32, 100.0);
        // The outlet sets this when the stream is published.
        i.session_id = crate::SESSION_ID.into();
        i.uid = "25cfa701-e55b-4e42-8fc2-103bbd2f86b6".into();
        i.source_id = "src".into();
        i.hostname = "box".into();
        i.v4data_port = 16572;
        i.v4service_port = 16572;
        i
    }

    #[test]
    fn the_description_round_trips() {
        let a = info();
        let xml = a.to_shortinfo_xml();
        let b = StreamInfo::from_shortinfo_xml(&xml).expect("a description");
        assert_eq!(b.name, a.name);
        assert_eq!(b.channel_count, a.channel_count);
        assert_eq!(b.format, a.format);
        assert_eq!(b.uid, a.uid);
        assert_eq!(b.v4data_port, a.v4data_port);
        assert!((b.nominal_srate - a.nominal_srate).abs() < 1e-9);
    }

    #[test]
    fn the_query_that_liblsl_builds_matches() {
        // `src/resolver_impl.cpp:66-73` builds exactly this shape.
        let i = info();
        assert!(i.matches("session_id='default' and name='Probe'"));
        assert!(i.matches("session_id='default'"));
        assert!(!i.matches("session_id='default' and name='Other'"));
        assert!(!i.matches("session_id='other' and name='Probe'"));
    }

    #[test]
    fn an_empty_query_matches_every_stream() {
        assert!(info().matches(""));
        assert!(info().matches("   "));
    }

    #[test]
    fn a_query_beyond_equality_is_answered() {
        // These once had to fail closed, because the matcher read only
        // equality. The reader is XPath now, and the oracle answers all three.
        let i = info();
        assert!(i.matches("contains(name,'Pro')"));
        assert!(i.matches("channel_count>4"));
        assert!(i.matches("starts-with(type,'EE')"));
    }

    #[test]
    fn a_value_with_a_space_survives_the_split() {
        let mut i = info();
        i.name = "My Stream and More".into();
        assert!(i.matches("session_id='default' and name='My Stream and More'"));
    }

    #[test]
    fn every_field_that_a_query_names_is_readable() {
        let i = info();
        for f in [
            "name",
            "type",
            "source_id",
            "session_id",
            "hostname",
            "uid",
            "channel_format",
            "channel_count",
        ] {
            // A bare name is true in XPath when the element exists.
            assert!(i.matches(f), "the field {f} is missing from the document");
        }
    }

    #[test]
    fn the_format_names_match_liblsl() {
        for (f, n) in [
            (Format::Float32, "float32"),
            (Format::Double64, "double64"),
            (Format::String, "string"),
            (Format::Int32, "int32"),
            (Format::Int16, "int16"),
            (Format::Int8, "int8"),
            (Format::Int64, "int64"),
        ] {
            assert_eq!(format_name(f), n);
            assert_eq!(format_from_name(n), Some(f));
        }
    }
}

#[cfg(test)]
mod measured_xpath {
    use super::*;

    /// The queries that `oracle/xpath.py` measured against the oracle.
    ///
    /// The battery showed that liblsl supports every group of XPath 1.0. This
    /// crate implements the conjunction of equalities that
    /// `resolver_impl::build_query` emits, and it fails closed on the rest.
    ///
    /// These tests pin both halves. A later change that starts answering a
    /// query outside the subset must be deliberate, because it changes what a
    /// resolver sees.
    fn info() -> StreamInfo {
        let mut i = StreamInfo::new("XpTest", "EEG", 8, Format::Float32, 100.0);
        // The outlet sets this when the stream is published.
        i.session_id = crate::SESSION_ID.into();
        i.uid = "25cfa701-e55b-4e42-8fc2-103bbd2f86b6".into();
        i.source_id = "xp_src".into();
        i
    }

    #[test]
    fn the_subset_that_a_resolver_emits_is_answered() {
        let i = info();
        for q in [
            "",
            "session_id='default'",
            "session_id='default' and name='XpTest'",
            "name='XpTest'",
            "type='EEG'",
            "source_id='xp_src'",
        ] {
            assert!(i.matches(q), "the subset must answer {q:?}");
        }
        for q in [
            "session_id='default' and name='Other'",
            "name='Nothing'",
            "type='EMG'",
        ] {
            assert!(!i.matches(q), "the subset must refuse {q:?}");
        }
    }

    #[test]
    fn every_shape_that_the_oracle_answers_is_answered_here() {
        // `oracle/xpath.py` sends each of these to a real outlet and records
        // the answer. Every one of them matches there, and every one of them
        // used to be refused here.
        let i = info();
        for q in [
            "contains(name,'XpT')",
            "starts-with(name,'Xp')",
            "string-length(name)>3",
            "substring(name,1,2)='Xp'",
            "concat(name,type)='XpTestEEG'",
            "channel_count>4",
            "number(channel_count)+1=9",
            "//name='XpTest'",
            "count(//name)=1",
            "not(name='Nothing')",
            "name='XpTest' or name='Nothing'",
            "true()",
        ] {
            assert!(i.matches(q), "{q:?} must match, and the oracle answers it");
        }
    }

    #[test]
    fn a_malformed_query_matches_nothing() {
        // The oracle behaves the same way: pugixml throws, liblsl catches and
        // returns false (`src/stream_info_impl.cpp:239-241`).
        let i = info();
        for q in [
            "in'va'lid",
            "name=",
            "and and and",
            "name='XpTest'))",
            "!!!",
        ] {
            assert!(!i.matches(q), "a malformed query must not match: {q:?}");
        }
    }
}

/// Flags that change how a buffer length is read. `include/lsl/common.h:156-172`.
pub mod transport {
    /// The default. The requested length is a count of seconds, or of hundreds
    /// of samples when the stream carries no rate.
    pub const DEFAULT: u32 = 0;
    /// The requested length is a literal count of samples.
    pub const BUFSIZE_SAMPLES: u32 = 1;
    /// Divide the result by 1000.
    pub const BUFSIZE_THOUSANDTHS: u32 = 2;
}

impl StreamInfo {
    /// Convert a requested buffer length into a count of samples.
    ///
    /// This is `calc_transport_buf_samples` (`src/stream_info_impl.cpp:254-270`),
    /// and liblsl runs it at the C boundary (`src/lsl_inlet_c.cpp:20`). The
    /// wire only ever carries samples. SPEC.md 8.6.
    ///
    /// An implementation that skips this step builds a buffer wrong by a factor
    /// of the rate. At 1000 Hz a request of 20 means 20,000 samples.
    pub fn buffer_samples(&self, requested: i32, flags: u32) -> i32 {
        if flags & transport::BUFSIZE_SAMPLES != 0 && flags & transport::BUFSIZE_THOUSANDTHS != 0 {
            // liblsl throws here. A caller that asks for both gets the literal
            // reading, which is the safer of the two.
            return requested.max(1);
        }
        let mut samples: i64 = if flags & transport::BUFSIZE_SAMPLES != 0 {
            requested as i64
        } else if self.nominal_srate == 0.0 {
            requested as i64 * 100
        } else {
            (self.nominal_srate * requested as f64) as i64
        };
        if flags & transport::BUFSIZE_THOUSANDTHS != 0 {
            samples /= 1000;
        }
        if samples > i32::MAX as i64 {
            return i32::MAX;
        }
        if samples > 0 {
            samples as i32
        } else {
            1
        }
    }
}

#[cfg(test)]
mod buffer_length {
    use super::*;

    fn info(srate: f64) -> StreamInfo {
        StreamInfo::new("B", "T", 2, Format::Float32, srate)
    }

    #[test]
    fn a_rate_makes_the_request_a_count_of_seconds() {
        // The trap from SPEC.md 8.6. A request of 20 at 1000 Hz is 20,000.
        assert_eq!(info(1000.0).buffer_samples(20, transport::DEFAULT), 20_000);
        assert_eq!(info(100.0).buffer_samples(360, transport::DEFAULT), 36_000);
    }

    #[test]
    fn no_rate_multiplies_by_one_hundred() {
        assert_eq!(info(0.0).buffer_samples(1, transport::DEFAULT), 100);
        assert_eq!(info(0.0).buffer_samples(6, transport::DEFAULT), 600);
    }

    #[test]
    fn the_samples_flag_takes_the_value_as_it_stands() {
        assert_eq!(
            info(1000.0).buffer_samples(20, transport::BUFSIZE_SAMPLES),
            20
        );
        assert_eq!(info(0.0).buffer_samples(20, transport::BUFSIZE_SAMPLES), 20);
    }

    #[test]
    fn the_thousandths_flag_divides() {
        assert_eq!(
            info(1000.0).buffer_samples(20_000, transport::BUFSIZE_THOUSANDTHS),
            20_000
        );
    }

    #[test]
    fn the_result_never_falls_below_one() {
        assert_eq!(info(0.001).buffer_samples(1, transport::DEFAULT), 1);
        assert_eq!(info(100.0).buffer_samples(0, transport::DEFAULT), 1);
        assert_eq!(info(100.0).buffer_samples(-5, transport::DEFAULT), 1);
    }
}

impl StreamInfo {
    /// The query that an inlet uses to find a stream again after it is lost.
    ///
    /// `src/inlet_connection.cpp:154-173`. The fields are the ones that survive
    /// a restart of the source. A stream keeps its name, its type, its source
    /// identifier, its channel count, and its channel format. It receives a new
    /// instance identifier.
    ///
    /// The nominal rate is left out on purpose. liblsl explains why in a
    /// comment: a rate written as text and read back is often not the same
    /// number, and a resolver then never finds the stream.
    ///
    /// The session identifier is also left out, unlike the query that a plain
    /// resolve builds (`src/resolver_impl.cpp:66-73`).
    pub fn recovery_query(&self) -> String {
        let mut q = format!("channel_count='{}'", self.channel_count);
        if !self.name.is_empty() {
            q.push_str(&format!(" and name='{}'", self.name));
        }
        if !self.stream_type.is_empty() {
            q.push_str(&format!(" and type='{}'", self.stream_type));
        }
        if !self.source_id.is_empty() {
            q.push_str(&format!(" and source_id='{}'", self.source_id));
        }
        q.push_str(&format!(
            " and channel_format='{}'",
            format_name(self.format)
        ));
        q
    }

    /// True when this stream can be found again after its source restarts.
    ///
    /// A stream with no source identifier cannot. liblsl warns about it at
    /// connection time (`src/inlet_connection.cpp:40`).
    pub fn is_recoverable(&self) -> bool {
        !self.source_id.is_empty()
    }
}

#[cfg(test)]
mod recovery {
    use super::*;

    fn info() -> StreamInfo {
        let mut i = StreamInfo::new("RcTest", "EEG", 4, Format::Float32, 200.0);
        // The outlet sets this when the stream is published.
        i.session_id = crate::SESSION_ID.into();
        i.source_id = "rc_fixed_source".into();
        i.uid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into();
        i
    }

    #[test]
    fn the_query_matches_the_shape_that_liblsl_builds() {
        assert_eq!(
            info().recovery_query(),
            "channel_count='4' and name='RcTest' and type='EEG' \
             and source_id='rc_fixed_source' and channel_format='float32'"
        );
    }

    #[test]
    fn the_query_leaves_out_the_rate() {
        // A rate written as text and read back is often a different number.
        // liblsl leaves it out for that reason.
        assert!(!info().recovery_query().contains("nominal_srate"));
    }

    #[test]
    fn the_query_finds_the_same_stream_after_a_new_identifier() {
        let before = info();
        let mut after = info();
        after.uid = "11111111-2222-3333-4444-555555555555".into();
        // The restarted stream carries a new instance identifier and every
        // other field is the same.
        assert!(after.matches(&before.recovery_query()));
        assert_ne!(before.uid, after.uid);
    }

    #[test]
    fn the_query_refuses_a_different_stream() {
        let q = info().recovery_query();
        let mut other = info();
        other.name = "Something".into();
        assert!(!other.matches(&q));

        let mut wider = info();
        wider.channel_count = 8;
        assert!(!wider.matches(&q));

        let mut retyped = info();
        retyped.format = Format::Double64;
        assert!(!retyped.matches(&q));
    }

    #[test]
    fn a_stream_with_no_source_identifier_cannot_be_recovered() {
        let mut i = info();
        i.source_id = String::new();
        assert!(!i.is_recoverable());
        assert!(info().is_recoverable());
        // The query then carries no source identifier either, so it can match
        // more than one stream.
        assert!(!i.recovery_query().contains("source_id"));
    }
}
