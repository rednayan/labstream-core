//! The description tree, and the whole document that carries it.
//!
//! A discovery answer holds a short description with an empty `<desc />`
//! (`src/stream_info_impl.cpp:160`). The whole document travels on the data
//! port, in answer to `LSL:fullinfo` (`src/tcp_server.cpp:508`). That document
//! carries the tree that an application builds, which is where channel labels,
//! units, and device names live.
//!
//! # Why the model keeps text as a node
//!
//! liblsl holds the tree in pugixml, where the text of an element is a child
//! node. The C ABI shows that model directly: `lsl_is_text` returns
//! `type() != node_element`, and `lsl_value` returns the value of the node
//! itself, which is empty for an element (`src/lsl_xml_element_c.cpp:36-38`).
//! A model that stores text in a field of the element cannot answer those two
//! calls correctly.
//!
//! # Where the layout rules come from
//!
//! Every rule below is measured. `oracle/descxml.cpp` builds trees through
//! real liblsl and writes the document. `oracle/descread.cpp` feeds documents
//! back in and writes what comes out. The results are in `captures/desc/`.
//!
//! | Case | What the oracle writes |
//! |---|---|
//! | An element with no children | `<x />` |
//! | An element with one empty text child | `<x></x>` |
//! | Text beside an element | `<v>a<b />c</v>`, all on one line |
//! | Text that holds only spaces | dropped, and the element becomes `<x />` |
//! | A comment | dropped |
//! | An attribute | kept, always in double quotes |
//! | A section of character data | kept as a section of character data |

/// What a node holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    /// A tag with a name.
    #[default]
    Element,
    /// A run of text.
    Text,
    /// A section of character data, which needs no escape.
    Cdata,
}

/// One node of a description tree.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Node {
    /// What this node holds.
    pub kind: Kind,
    /// The tag name. A text node has no name.
    pub name: String,
    /// The text. An element has no text of its own.
    pub value: String,
    /// The attributes of an element, in the order they were written.
    ///
    /// No call of the C ABI creates one (`include/lsl/xml.h`). An attribute
    /// reaches the tree only through a document that an inlet reads, and the
    /// oracle keeps it, so this keeps it too.
    pub attrs: Vec<(String, String)>,
    /// The nodes below this one.
    pub children: Vec<Node>,
}

/// Write a newline before the node.
const NEWLINE: u8 = 1;
/// Write the indent before the node.
const INDENT: u8 = 2;

impl Node {
    /// A tag with no content.
    pub fn element(name: &str) -> Node {
        Node {
            kind: Kind::Element,
            name: name.to_string(),
            ..Default::default()
        }
    }

    /// A run of text.
    pub fn text(value: &str) -> Node {
        Node {
            kind: Kind::Text,
            value: value.to_string(),
            ..Default::default()
        }
    }

    /// A section of character data.
    pub fn cdata(value: &str) -> Node {
        Node {
            kind: Kind::Cdata,
            value: value.to_string(),
            ..Default::default()
        }
    }

    /// Add a tag below this node and return it.
    pub fn append_child(&mut self, name: &str) -> &mut Node {
        self.children.push(Node::element(name));
        self.children.last_mut().unwrap()
    }

    /// Add a tag that holds text.
    ///
    /// This matches `lsl_append_child_value`, which appends the tag and then
    /// appends one text node below it (`src/lsl_xml_element_c.cpp:74`).
    pub fn append_child_value(&mut self, name: &str, value: &str) -> &mut Node {
        let mut e = Node::element(name);
        e.children.push(Node::text(value));
        self.children.push(e);
        self.children.last_mut().unwrap()
    }

    /// The first child with this name.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// The first child with this name, for a change.
    pub fn child_mut(&mut self, name: &str) -> Option<&mut Node> {
        self.children.iter_mut().find(|c| c.name == name)
    }

    /// The text of the first child that holds text.
    pub fn child_value(&self) -> &str {
        for c in &self.children {
            if c.kind != Kind::Element {
                return &c.value;
            }
        }
        ""
    }

    /// The text of the first child with this name.
    pub fn child_value_of(&self, name: &str) -> &str {
        match self.child(name) {
            Some(c) => c.child_value(),
            None => "",
        }
    }

    /// True when this node holds nothing at all.
    pub fn is_empty(&self) -> bool {
        self.name.is_empty()
            && self.value.is_empty()
            && self.children.is_empty()
            && self.attrs.is_empty()
    }

    /// Write this node and everything below it, and end with a newline.
    ///
    /// `depth` counts the tabs before the opening tag.
    pub fn write(&self, out: &mut String, depth: usize) {
        self.emit(out, depth, INDENT);
        out.push('\n');
    }

    /// Write one node and return the flags for the node that follows.
    ///
    /// pugixml carries two flags while it walks the tree. Text clears both, so
    /// everything after text on the same level stays on the same line, up to
    /// and including the closing tag. That rule produces `<v>a<b />c</v>` and
    /// `<blank></blank>`, both of which the oracle writes.
    fn emit(&self, out: &mut String, depth: usize, flags: u8) -> u8 {
        if self.kind == Kind::Text {
            escape_text_into(&self.value, out);
            return 0;
        }
        if self.kind == Kind::Cdata {
            out.push_str("<![CDATA[");
            out.push_str(&self.value);
            out.push_str("]]>");
            return 0;
        }
        if flags & NEWLINE != 0 {
            out.push('\n');
        }
        if flags & INDENT != 0 {
            for _ in 0..depth {
                out.push('\t');
            }
        }
        out.push('<');
        out.push_str(&self.name);
        for (k, v) in &self.attrs {
            out.push(' ');
            out.push_str(k);
            out.push_str("=\"");
            escape_attr_into(v, out);
            out.push('"');
        }
        if self.children.is_empty() {
            out.push_str(" />");
            return NEWLINE | INDENT;
        }
        out.push('>');
        let mut f = NEWLINE | INDENT;
        for c in &self.children {
            f = c.emit(out, depth + 1, f);
        }
        if f & NEWLINE != 0 {
            out.push('\n');
        }
        if f & INDENT != 0 {
            for _ in 0..depth {
                out.push('\t');
            }
        }
        out.push_str("</");
        out.push_str(&self.name);
        out.push('>');
        NEWLINE | INDENT
    }
}

/// Escape the three characters that the oracle escapes in text.
///
/// `captures/desc/desc_escapes.xml` holds the measurement. A double quote, a
/// single quote, a tab, a line feed, and a carriage return all pass through
/// unchanged. Only `&`, `<`, and `>` change.
fn escape_text_into(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
}

/// Escape an attribute value.
///
/// `captures/desc/desc_reader.txt`, case `attribute_escapes`, holds the
/// measurement for `&`, `"`, `<`, `>`, a line feed, and a tab. A greater-than
/// sign and a single quote stay as they are, and a tab writes as `&#09;` with
/// two digits. A carriage return is not measured. It is escaped here for the
/// same reason a line feed is. An XML reader turns an unescaped one into a
/// space.
fn escape_attr_into(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#09;"),
            other => out.push(other),
        }
    }
}

// ===========================================================================
// the reader
// ===========================================================================

/// Read a document and return its root element.
///
/// This follows the oracle, which loads the document with the default parse
/// flags and never changes them (`src/stream_info_impl.cpp:183`):
///
/// * Text that holds only whitespace is dropped.
///   `captures/desc/desc_reader.txt`, case `whitespace_only_text`.
/// * A comment is dropped. Same file, case `comment`.
/// * A line end is normalized. `\r\n` and a lone `\r` both become `\n`.
///   `captures/desc/desc_reparse.hex` shows a carriage return that does not
///   survive the round trip.
/// * An entity reference is decoded, and a reference with no semicolon stays
///   as it is. Same file, case `unterminated_reference`.
pub fn parse(xml: &str) -> Option<Node> {
    let b = xml.as_bytes();
    let mut i = 0usize;
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;

    while i < b.len() {
        if b[i] != b'<' {
            let start = i;
            while i < b.len() && b[i] != b'<' {
                i += 1;
            }
            let raw = &xml[start..i];
            if !raw.trim().is_empty() {
                if let Some(top) = stack.last_mut() {
                    top.children.push(Node::text(&decode(raw)));
                }
            }
            continue;
        }
        if xml[i..].starts_with("<?") {
            i = find(b, i, b"?>")? + 2;
            continue;
        }
        if xml[i..].starts_with("<!--") {
            i = find(b, i, b"-->")? + 3;
            continue;
        }
        if xml[i..].starts_with("<![CDATA[") {
            let end = find(b, i, b"]]>")?;
            // A section of character data holds no entity reference. The line
            // ends still change.
            let raw = normalize_eol(&xml[i + 9..end]);
            if let Some(top) = stack.last_mut() {
                top.children.push(Node::cdata(&raw));
            }
            i = end + 3;
            continue;
        }
        if xml[i..].starts_with("<!") {
            i = find(b, i, b">")? + 1;
            continue;
        }
        if xml[i..].starts_with("</") {
            let end = find(b, i, b">")?;
            let done = stack.pop()?;
            if stack.is_empty() {
                root = Some(done);
            } else {
                stack.last_mut()?.children.push(done);
            }
            i = end + 1;
            continue;
        }

        // an opening tag
        let end = tag_end(b, i)?;
        let inner = &xml[i + 1..end];
        let selfclose = inner.trim_end().ends_with('/');
        let inner = match inner.trim_end().strip_suffix('/') {
            Some(x) => x,
            None => inner,
        };
        let (name, rest) = split_name(inner);
        if name.is_empty() {
            return None;
        }
        let mut node = Node::element(name);
        node.attrs = read_attrs(rest);
        if selfclose {
            if let Some(top) = stack.last_mut() {
                top.children.push(node);
            } else {
                root = Some(node);
            }
        } else {
            stack.push(node);
        }
        i = end + 1;
    }

    if !stack.is_empty() {
        return None;
    }
    root
}

/// Find the `>` that closes a tag, skipping one inside a quoted value.
fn tag_end(b: &[u8], from: usize) -> Option<usize> {
    let mut quote = 0u8;
    for (k, &c) in b.iter().enumerate().skip(from) {
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'>' {
            return Some(k);
        }
    }
    None
}

fn split_name(inner: &str) -> (&str, &str) {
    match inner.find(|c: char| c.is_ascii_whitespace()) {
        Some(k) => (&inner[..k], &inner[k..]),
        None => (inner, ""),
    }
}

/// Read `name="value"` pairs. A name with no value is dropped.
fn read_attrs(rest: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let chars: Vec<char> = rest.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i] != '=' && !chars[i].is_ascii_whitespace() {
            i += 1;
        }
        let name: String = chars[start..i].iter().collect();
        while i < chars.len() && chars[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= chars.len() || chars[i] != '=' {
            continue;
        }
        i += 1;
        while i < chars.len() && chars[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let quote = chars[i];
        if quote != '"' && quote != '\'' {
            break;
        }
        i += 1;
        let vstart = i;
        while i < chars.len() && chars[i] != quote {
            i += 1;
        }
        let value: String = chars[vstart..i].iter().collect();
        i += 1;
        if !name.is_empty() {
            out.push((name, decode(&value)));
        }
    }
    out
}

fn find(b: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    (from..=b.len().checked_sub(pat.len())?).find(|&k| &b[k..k + pat.len()] == pat)
}

/// Turn `\r\n` and a lone `\r` into `\n`.
fn normalize_eol(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\r' {
            if it.peek() == Some(&'\n') {
                it.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

/// Decode entity references, then normalize the line ends.
fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'&' {
            let start = i;
            while i < b.len() && b[i] != b'&' {
                i += 1;
            }
            out.push_str(&s[start..i]);
            continue;
        }
        let end = match find(b, i, b";") {
            Some(e) if e - i <= 10 => e,
            // An ampersand with no terminator stays as it is.
            _ => {
                out.push('&');
                i += 1;
                continue;
            }
        };
        let name = &s[i + 1..end];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => numeric_entity(name),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                i = end + 1;
            }
            None => {
                out.push('&');
                i += 1;
            }
        }
    }
    normalize_eol(&out)
}

fn numeric_entity(name: &str) -> Option<char> {
    let digits = name.strip_prefix('#')?;
    let code = match digits
        .strip_prefix('x')
        .or_else(|| digits.strip_prefix('X'))
    {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => digits.parse::<u32>().ok()?,
    };
    char::from_u32(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(n: &Node) -> String {
        let mut s = String::new();
        n.write(&mut s, 0);
        s
    }

    /// Read a document, write it back, and return the `<desc>` part.
    ///
    /// This is the shape that `captures/desc/desc_reader.txt` records.
    fn round(desc: &str) -> String {
        let n = parse(desc).expect("a tree");
        let mut s = String::new();
        n.emit(&mut s, 1, INDENT);
        s
    }

    #[test]
    fn an_element_with_no_children_prints_as_an_empty_tag() {
        assert_eq!(render(&Node::element("desc")), "<desc />\n");
    }

    #[test]
    fn an_empty_text_child_prints_as_a_pair_of_tags() {
        // `captures/desc/desc_blank.xml` shows `<blank></blank>` beside
        // `<hollow />`. The two states have to stay apart.
        let mut d = Node::element("desc");
        d.append_child_value("blank", "");
        d.append_child("hollow");
        assert_eq!(
            render(&d),
            "<desc>\n\t<blank></blank>\n\t<hollow />\n</desc>\n"
        );
    }

    #[test]
    fn text_beside_an_element_keeps_the_whole_element_on_one_line() {
        // `captures/desc/desc_reader.txt`, case `mixed_content`.
        assert_eq!(
            round("<desc><v>a<b />c</v></desc>"),
            "\t<desc>\n\t\t<v>a<b />c</v>\n\t</desc>"
        );
    }

    #[test]
    fn only_three_characters_are_escaped_in_text() {
        let mut d = Node::element("d");
        d.append_child_value("v", "a & b <x> \" ' \t");
        assert_eq!(
            render(&d),
            "<d>\n\t<v>a &amp; b &lt;x&gt; \" ' \t</v>\n</d>\n"
        );
    }

    #[test]
    fn an_attribute_survives_a_round_trip() {
        // `captures/desc/desc_reader.txt`, case `attribute`. A single quote in
        // the source becomes a double quote in the output.
        assert_eq!(
            round("<desc><v id=\"1\" unit='mV'>x</v></desc>"),
            "\t<desc>\n\t\t<v id=\"1\" unit=\"mV\">x</v>\n\t</desc>"
        );
    }

    #[test]
    fn an_attribute_escapes_the_characters_that_the_oracle_escapes() {
        // `captures/desc/desc_reader.txt`, case `attribute_escapes`.
        assert_eq!(
            round("<desc><v a=\"x&amp;y&quot;z&#10;w&#9;t&lt;u&gt;\" b='s&apos;q'>1</v></desc>"),
            "\t<desc>\n\t\t<v a=\"x&amp;y&quot;z&#10;w&#09;t&lt;u>\" b=\"s'q\">1</v>\n\t</desc>"
        );
    }

    #[test]
    fn a_section_of_character_data_stays_one() {
        // `captures/desc/desc_reader.txt`, cases `cdata` and `cdata_and_text`.
        assert_eq!(
            round("<desc><v><![CDATA[a & <b>]]></v></desc>"),
            "\t<desc>\n\t\t<v><![CDATA[a & <b>]]></v>\n\t</desc>"
        );
        assert_eq!(
            round("<desc><v>a<![CDATA[<b>]]>c</v></desc>"),
            "\t<desc>\n\t\t<v>a<![CDATA[<b>]]>c</v>\n\t</desc>"
        );
    }

    #[test]
    fn a_comment_is_dropped() {
        // `captures/desc/desc_reader.txt`, case `comment`.
        assert_eq!(
            round("<desc><!-- gone --><v>x</v></desc>"),
            "\t<desc>\n\t\t<v>x</v>\n\t</desc>"
        );
    }

    #[test]
    fn text_that_holds_only_spaces_is_dropped() {
        // `captures/desc/desc_reader.txt`, case `whitespace_only_text`.
        assert_eq!(
            round("<desc><v>   </v></desc>"),
            "\t<desc>\n\t\t<v />\n\t</desc>"
        );
    }

    #[test]
    fn a_reference_with_no_semicolon_stays_as_it_is() {
        // `captures/desc/desc_reader.txt`, case `unterminated_reference`. The
        // value keeps the ampersand, so writing it out escapes that ampersand.
        let n = parse("<desc><v>100% &amp fine</v></desc>").expect("a tree");
        assert_eq!(n.child_value_of("v"), "100% &amp fine");
        assert_eq!(
            round("<desc><v>100% &amp fine</v></desc>"),
            "\t<desc>\n\t\t<v>100% &amp;amp fine</v>\n\t</desc>"
        );
    }

    #[test]
    fn a_numeric_reference_is_decoded() {
        // `captures/desc/desc_reader.txt`, case `numeric_reference`.
        let n = parse("<desc><v>&#65;&#x42;&#10;</v></desc>").expect("a tree");
        assert_eq!(n.child_value_of("v"), "AB\n");
    }

    #[test]
    fn a_carriage_return_does_not_survive_the_reader() {
        // Measured. `captures/desc/desc_reparse.hex` holds
        // `lines 6f6e650a74776f0a746872656509666f7572`.
        let n = parse("<a><lines>one\ntwo\rthree\tfour</lines></a>").expect("a tree");
        assert_eq!(n.child_value_of("lines"), "one\ntwo\nthree\tfour");
    }

    #[test]
    fn a_tag_that_never_closes_is_rejected() {
        assert!(parse("<a><b></a>").is_none());
        assert!(parse("<a>").is_none());
    }

    #[test]
    fn text_reads_back_as_a_child_node() {
        // The C ABI needs this. `lsl_is_text` asks the node itself.
        let n = parse("<a><b>v</b></a>").expect("a tree");
        let b = n.child("b").expect("b");
        assert_eq!(b.kind, Kind::Element);
        assert_eq!(b.value, "");
        assert_eq!(b.children.len(), 1);
        assert_eq!(b.children[0].kind, Kind::Text);
        assert_eq!(b.children[0].value, "v");
    }

    #[test]
    fn a_greater_than_sign_inside_an_attribute_does_not_end_the_tag() {
        let n = parse("<a><b x=\"1>2\">v</b></a>").expect("a tree");
        assert_eq!(
            n.child("b").expect("b").attrs,
            vec![("x".into(), "1>2".into())]
        );
    }
}
