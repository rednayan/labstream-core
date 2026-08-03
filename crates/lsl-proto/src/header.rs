//! The header block parser, and the status line.
//!
//! liblsl parses request headers and answer headers with the same code
//! (`src/tcp_server.cpp:602-632`, `src/data_receiver.cpp:215-258`). The rules
//! are in SPEC.md 4.3.

use core::fmt;

/// The largest line that liblsl reads. `src/tcp_server.cpp:601`.
pub const LINE_LIMIT: usize = 16384;

/// A parsed header block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers {
    /// Name and value pairs, in the order that they arrived. Names are lower
    /// case, because the parser lowercases every line.
    pub fields: Vec<(String, String)>,
    /// Lines that held no colon. liblsl logs and ignores them.
    pub ignored: Vec<String>,
}

impl Headers {
    /// The value of the first field with this name.
    ///
    /// The caller passes a lower case name, because every parsed name is lower
    /// case.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The value of a field, read as an integer.
    pub fn get_int(&self, name: &str) -> Option<i64> {
        self.get(name).and_then(parse_leading_int)
    }

    /// The value of a field, read as a floating point number.
    pub fn get_f64(&self, name: &str) -> Option<f64> {
        self.get(name).and_then(parse_leading_f64)
    }

    /// The value of a field, read as a flag.
    ///
    /// liblsl reads a flag with `from_string<bool>`, which accepts `1` and `0`.
    pub fn get_bool(&self, name: &str) -> Option<bool> {
        self.get(name)
            .map(|v| v.trim() == "1" || v.trim() == "true")
    }
}

/// liblsl reads a number with `std::stoi`, which stops at the first character
/// that does not belong to a number. It does not reject trailing text.
fn parse_leading_int(s: &str) -> Option<i64> {
    let t = s.trim_start();
    let mut end = 0;
    for (i, c) in t.char_indices() {
        if c.is_ascii_digit() || (i == 0 && (c == '-' || c == '+')) {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    t[..end].parse().ok()
}

/// liblsl reads a real number with `std::stod`, which behaves the same way.
fn parse_leading_f64(s: &str) -> Option<f64> {
    let t = s.trim_start();
    let mut end = 0;
    for (i, c) in t.char_indices() {
        if c.is_ascii_digit()
            || c == '.'
            || ((c == '-' || c == '+') && i == 0)
            || c == 'e'
            || c == 'E'
        {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    t[..end].parse().ok()
}

/// Split a line into a header name and a value.
///
/// The rules, from SPEC.md 4.3 and `src/tcp_server.cpp:604-616`:
///
/// 1. A line with no colon is ignored.
/// 2. A semicolon starts a comment. The parser removes it and the rest.
/// 3. The parser lowercases the whole line, so names are case-insensitive.
/// 4. The parser trims the name and the value.
///
/// liblsl finds the colon before it removes the comment, and it then uses the
/// old position. A semicolon before the colon therefore reads past the end of
/// the shortened line. This function returns `None` for that input, which is
/// the safe reading of a path that no upstream test covers. SPEC.md 4.3 records
/// the behavior as observed.
pub fn split_line(line: &str) -> Option<(String, String)> {
    let colon = line.find(':')?;
    let cut = match line.find(';') {
        Some(semi) => &line[..semi],
        None => line,
    };
    let lower = cut.to_ascii_lowercase();
    if colon >= lower.len() {
        // The comment removed the colon. liblsl reads past the end here.
        return None;
    }
    let name = lower[..colon].trim().to_string();
    let value = lower[colon + 1..].trim().to_string();
    Some((name, value))
}

/// Read a header block. The block ends at a line that starts with a carriage
/// return, or at an empty line.
///
/// Returns the headers and the number of bytes that the block used. Returns
/// `None` when the buffer holds no complete block.
pub fn parse_block(buf: &[u8]) -> Option<(Headers, usize)> {
    let mut headers = Headers::default();
    let mut pos = 0usize;

    loop {
        let rest = &buf[pos..];
        let nl = rest.iter().position(|&b| b == b'\n')?;
        let raw = &rest[..nl];
        pos += nl + 1;

        // liblsl checks the first byte for a carriage return
        // (`src/tcp_server.cpp:602`). An empty line ends the block too.
        if raw.first() == Some(&b'\r') || raw.is_empty() {
            return Some((headers, pos));
        }

        let line = String::from_utf8_lossy(raw)
            .trim_end_matches('\r')
            .to_string();
        match split_line(&line) {
            Some((k, v)) => headers.fields.push((k, v)),
            None => headers.ignored.push(line),
        }
    }
}

/// A status line, such as `LSL/110 200 OK`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLine {
    /// The protocol version that the other side runs.
    pub version: i32,
    /// The status code.
    pub code: i32,
    /// The message after the code. It can hold spaces.
    pub message: String,
}

/// What a client does with a status code. SPEC.md 4.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusAction {
    /// Read the answer headers and continue.
    Proceed,
    /// Treat the stream as lost, then resolve again.
    StreamLost,
    /// Treat this as a redirect, which also marks the stream lost.
    Redirect,
    /// Raise an error.
    Fail,
}

impl StatusLine {
    /// Write the line as liblsl writes it. `src/tcp_server.cpp:673`.
    pub fn to_wire(&self) -> String {
        format!("LSL/{} {} {}\r\n", self.version, self.code, self.message)
    }

    /// Read a status line.
    ///
    /// liblsl splits on spaces and needs three parts. The first part starts
    /// with `LSL/` (`src/data_receiver.cpp:196-199`).
    pub fn parse(line: &str) -> Result<Self, Error> {
        let clean = line.trim_end_matches(['\r', '\n']);
        let parts: Vec<&str> = clean.split(' ').filter(|p| !p.is_empty()).collect();
        if parts.len() < 3 || !parts[0].starts_with("LSL/") {
            return Err(Error::MalformedStatusLine(clean.to_string()));
        }
        let version = parts[0][4..]
            .parse()
            .map_err(|_| Error::MalformedStatusLine(clean.to_string()))?;
        let code = parts[1]
            .parse()
            .map_err(|_| Error::MalformedStatusLine(clean.to_string()))?;
        Ok(StatusLine {
            version,
            code,
            message: parts[2..].join(" "),
        })
    }

    /// What a client does with this code. SPEC.md 4.5.
    ///
    /// liblsl checks ranges and not exact values
    /// (`src/data_receiver.cpp:207-212`). The order matters: 404 comes before
    /// the test for 400 and above.
    pub fn action(&self) -> StatusAction {
        if self.code == 404 {
            StatusAction::StreamLost
        } else if self.code >= 400 {
            StatusAction::Fail
        } else if self.code >= 300 {
            StatusAction::Redirect
        } else {
            StatusAction::Proceed
        }
    }
}

/// A protocol error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The status line held fewer than three parts, or a wrong prefix.
    MalformedStatusLine(String),
    /// The server agreed to a version that this implementation does not read.
    ///
    /// Protocol 1.00 carries every sample in a Boost archive
    /// (`src/sample.cpp:330`). Reading one is out of scope, so a feed that
    /// agrees to 1.00 stops here rather than decoding the bytes as 1.10.
    VersionTooOld {
        /// The version that the server named.
        theirs: i32,
    },
    /// The other side runs a newer major version.
    VersionTooNew {
        /// The version that the other side reported.
        theirs: i32,
        /// The version that this side supports.
        ours: i32,
    },
    /// The answer named a UID that does not match this connection.
    UidMismatch {
        /// The UID that this connection holds.
        expected: String,
        /// The UID that the answer named.
        actual: String,
    },
    /// The answer asked for a byte order that this side cannot produce.
    ByteOrderNotSupported(i32),
    /// The other side answered with a status code that stops the feed.
    Status(StatusLine),
    /// The buffer held no complete block.
    Incomplete,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::MalformedStatusLine(l) => write!(f, "the status line is malformed: {l}"),
            Error::VersionTooOld { theirs } => {
                write!(
                    f,
                    "the server agreed to protocol {theirs}, and this implementation reads 1.10 and above"
                )
            }
            Error::VersionTooNew { theirs, ours } => {
                write!(
                    f,
                    "the other side runs version {theirs} and this side runs {ours}"
                )
            }
            Error::UidMismatch { expected, actual } => {
                write!(
                    f,
                    "the answer named the UID {actual} and this connection holds {expected}"
                )
            }
            Error::ByteOrderNotSupported(v) => write!(f, "the byte order {v} is not supported"),
            Error::Status(s) => write!(f, "the other side answered {} {}", s.code, s.message),
            Error::Incomplete => write!(f, "the buffer holds no complete block"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_with_no_colon_is_ignored() {
        assert_eq!(split_line("this line has no colon"), None);
    }

    #[test]
    fn a_semicolon_starts_a_comment() {
        assert_eq!(
            split_line("Max-Chunk-Length: 32 ; the rest is a comment"),
            Some(("max-chunk-length".into(), "32".into()))
        );
    }

    #[test]
    fn a_name_is_case_insensitive() {
        assert_eq!(
            split_line("NaTiVe-ByTe-OrDeR: 1234"),
            Some(("native-byte-order".into(), "1234".into()))
        );
    }

    #[test]
    fn a_comment_before_the_colon_yields_nothing() {
        // SPEC.md 4.3 records this path as observed. liblsl reads past the end
        // of the shortened line. This parser refuses instead.
        assert_eq!(split_line("; a comment: with a colon"), None);
    }

    #[test]
    fn a_block_ends_at_a_blank_line() {
        let buf = b"A: 1\r\nB: 2\r\n\r\ntrailing";
        let (h, used) = parse_block(buf).expect("a complete block");
        assert_eq!(
            h.fields,
            vec![("a".into(), "1".into()), ("b".into(), "2".into())]
        );
        assert_eq!(&buf[used..], b"trailing");
    }

    #[test]
    fn an_incomplete_block_returns_nothing() {
        assert_eq!(parse_block(b"A: 1\r\nB: 2\r\n"), None);
    }

    #[test]
    fn a_status_line_round_trips() {
        let s = StatusLine {
            version: 110,
            code: 200,
            message: "OK".into(),
        };
        assert_eq!(s.to_wire(), "LSL/110 200 OK\r\n");
        assert_eq!(StatusLine::parse(&s.to_wire()).unwrap(), s);
    }

    #[test]
    fn a_status_message_can_hold_spaces() {
        let s = StatusLine::parse("LSL/110 505 Version not supported").unwrap();
        assert_eq!(s.code, 505);
        assert_eq!(s.message, "Version not supported");
        assert_eq!(s.action(), StatusAction::Fail);
    }

    #[test]
    fn status_codes_map_to_the_documented_action() {
        let mk = |c| StatusLine {
            version: 110,
            code: c,
            message: "x".into(),
        };
        assert_eq!(mk(200).action(), StatusAction::Proceed);
        assert_eq!(mk(301).action(), StatusAction::Redirect);
        assert_eq!(mk(404).action(), StatusAction::StreamLost);
        assert_eq!(mk(500).action(), StatusAction::Fail);
        assert_eq!(mk(505).action(), StatusAction::Fail);
    }

    #[test]
    fn a_malformed_status_line_is_rejected() {
        assert!(StatusLine::parse("HTTP/1.1 200 OK").is_err());
        assert!(StatusLine::parse("LSL/110 200").is_err());
    }

    #[test]
    fn a_number_stops_at_trailing_text() {
        // liblsl reads a number with std::stoi, which ignores trailing text.
        let mut h = Headers::default();
        h.fields.push(("value-size".into(), "4 bytes".into()));
        assert_eq!(h.get_int("value-size"), Some(4));
    }
}
