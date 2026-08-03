//! The C ABI of LSL.
//!
//! Every other crate in this workspace forbids raw pointers. This one is where
//! they live, because a C application reaches the library through this surface
//! and no other.
//!
//! # What this has to satisfy
//!
//! An application binds these symbols by name at load time. `pylsl` sets an
//! argument list for all 85 symbols it uses while it imports, so a missing
//! symbol stops the import before any call happens. Every name it needs is
//! therefore present here, and a function that is not implemented yet says so
//! through the error code that the header already defines.
//!
//! # Ownership
//!
//! Each handle is a `Box` that crossed the boundary through `Box::into_raw`.
//! The matching destroy call takes it back with `Box::from_raw`. A caller that
//! never destroys leaks, which matches liblsl.

#![allow(non_camel_case_types)]
#![deny(missing_docs)]

use labstream_net::{desc, ContinuousResolver, Inlet, Outlet, StreamInfo};
use labstream_wire::{Format, Sample, Value};
use std::ffi::{c_char, c_double, c_int, c_void, CStr, CString};
use std::time::Duration;

// ---------------------------------------------------------------------------
// error codes and constants, from include/lsl/common.h
// ---------------------------------------------------------------------------

/// No error.
pub const LSL_NO_ERROR: i32 = 0;
/// The operation ran out of time.
pub const LSL_TIMEOUT_ERROR: i32 = -1;
/// The stream is gone.
pub const LSL_LOST_ERROR: i32 = -2;
/// An argument was wrong.
pub const LSL_ARGUMENT_ERROR: i32 = -3;
/// Anything else.
pub const LSL_INTERNAL_ERROR: i32 = -4;

/// The value that means "no fixed rate".
pub const LSL_IRREGULAR_RATE: f64 = 0.0;
/// The value that asks the reader to derive a timestamp.
pub const LSL_DEDUCED_TIMESTAMP: f64 = -1.0;

const LSL_PROTOCOL_VERSION: i32 = 110;
/// `src/common.h:49`. The value is major times 100 plus minor, so 1.17
/// reads as 117. It is not the patch level.
const LSL_LIBRARY_VERSION: i32 = 117;

// ---------------------------------------------------------------------------
// handles
// ---------------------------------------------------------------------------

/// The stream description handle.
pub type lsl_streaminfo = *mut c_void;
/// The outlet handle.
pub type lsl_outlet = *mut c_void;
/// The inlet handle.
pub type lsl_inlet = *mut c_void;
/// A node of the description tree.
pub type lsl_xml_ptr = *mut c_void;
/// The background resolver handle.
pub type lsl_continuous_resolver = *mut c_void;

struct InfoBox {
    info: StreamInfo,
    /// The whole document, in the shape that the C ABI needs.
    ///
    /// liblsl hands out a pointer into one pugixml document that holds every
    /// field as well as the tree, so `lsl_parent` on the description reaches
    /// `<info>` and then the document itself. `oracle/desctree.c` measures
    /// that. The same shape is built here.
    ///
    /// The struct fields stay the authority for `<info>`, and the tree stays
    /// the authority for `<desc>`. [`InfoBox::sync`] copies each one to the
    /// other. A write to an `<info>` field through the tree is therefore lost.
    /// No call of the C ABI reaches those fields except by walking up from the
    /// description, and nothing writes through that path.
    doc: Box<XmlNode>,
}

/// The fields of `<info>`, in the order that liblsl writes them.
/// `src/stream_info_impl.cpp:60-82`.
const INFO_FIELDS: [&str; 17] = [
    "name",
    "type",
    "channel_count",
    "channel_format",
    "source_id",
    "nominal_srate",
    "version",
    "created_at",
    "uid",
    "session_id",
    "hostname",
    "v4address",
    "v4data_port",
    "v4service_port",
    "v6address",
    "v6data_port",
    "v6service_port",
];

impl InfoBox {
    /// Build a box around a description.
    fn new(info: StreamInfo) -> *mut InfoBox {
        let mut doc = XmlNode::new(desc::Kind::Element, "", "", std::ptr::null_mut());
        doc.document = true;
        let here: *mut XmlNode = &mut *doc;
        let mut node = XmlNode::new(desc::Kind::Element, "info", "", here);
        let inside: *mut XmlNode = &mut *node;
        for f in INFO_FIELDS {
            let mut e = XmlNode::new(desc::Kind::Element, f, "", inside);
            let text: *mut XmlNode = &mut *e;
            e.children
                .push(XmlNode::new(desc::Kind::Text, "", "", text));
            node.children.push(e);
        }
        node.children.push(XmlNode::from_desc(&info.desc, inside));
        doc.children.push(node);

        let mut b = Box::new(InfoBox { info, doc });
        b.write_fields();
        Box::into_raw(b)
    }

    /// The `<info>` element.
    fn info_node(&mut self) -> &mut XmlNode {
        &mut self.doc.children[0]
    }

    /// The `<desc>` element.
    fn desc_node(&mut self) -> &mut XmlNode {
        let last = self.info_node().children.len() - 1;
        &mut self.info_node().children[last]
    }

    /// Copy the struct fields into `<info>`.
    fn write_fields(&mut self) {
        let text = field_text(&self.info);
        let node = self.info_node();
        for (k, v) in text.iter().enumerate() {
            node.children[k].children[0].value = v.clone();
        }
    }

    /// Copy the tree that the application edited back into the description.
    fn sync(&mut self) {
        let mut tree = self.desc_node().to_desc();
        tree.name = "desc".to_string();
        self.info.desc = tree;
        self.write_fields();
    }
}

/// The text of every field of `<info>`, in the order of [`INFO_FIELDS`].
fn field_text(info: &StreamInfo) -> Vec<String> {
    vec![
        info.name.clone(),
        info.stream_type.clone(),
        info.channel_count.to_string(),
        labstream_net::info::format_name(info.format).to_string(),
        info.source_id.clone(),
        labstream_net::info::show_point_16(info.nominal_srate),
        "1.100000000000000".to_string(),
        labstream_net::info::show_point_16(info.created_at),
        info.uid.clone(),
        info.session_id.clone(),
        info.hostname.clone(),
        String::new(),
        info.v4data_port.to_string(),
        info.v4service_port.to_string(),
        String::new(),
        "0".to_string(),
        "0".to_string(),
    ]
}

/// One node of the description tree.
///
/// liblsl holds this in pugixml, and the C ABI hands out a pointer to one
/// node (`src/lsl_xml_element_c.cpp`). A pointer stays valid while the
/// application adds nodes around it, so every node lives in its own
/// allocation and every node knows its parent.
#[derive(Debug)]
struct XmlNode {
    kind: desc::Kind,
    name: String,
    value: String,
    attrs: Vec<(String, String)>,
    /// True for the node that holds the whole document. liblsl reports that
    /// node as text, because its type is not an element
    /// (`src/lsl_xml_element_c.cpp:35`).
    document: bool,
    // The box is not spare. A C caller holds a pointer to a node while it adds
    // more nodes around it. A `Vec<XmlNode>` moves its contents when it grows,
    // and every pointer the caller holds then names freed memory.
    #[allow(clippy::vec_box)]
    children: Vec<Box<XmlNode>>,
    /// The node above this one. Null at the root of the tree.
    parent: *mut XmlNode,
}

impl XmlNode {
    fn new(kind: desc::Kind, name: &str, value: &str, parent: *mut XmlNode) -> Box<XmlNode> {
        Box::new(XmlNode {
            kind,
            name: name.to_string(),
            value: value.to_string(),
            attrs: Vec::new(),
            document: false,
            children: Vec::new(),
            parent,
        })
    }

    /// Copy a safe tree in, and link every node to its parent.
    fn from_desc(src: &desc::Node, parent: *mut XmlNode) -> Box<XmlNode> {
        let mut me = Box::new(XmlNode {
            kind: src.kind,
            name: src.name.clone(),
            value: src.value.clone(),
            attrs: src.attrs.clone(),
            document: false,
            children: Vec::new(),
            parent,
        });
        let here: *mut XmlNode = &mut *me;
        for c in &src.children {
            me.children.push(XmlNode::from_desc(c, here));
        }
        me
    }

    /// Copy the tree back out.
    fn to_desc(&self) -> desc::Node {
        desc::Node {
            kind: self.kind,
            name: self.name.clone(),
            value: self.value.clone(),
            attrs: self.attrs.clone(),
            children: self.children.iter().map(|c| c.to_desc()).collect(),
        }
    }

    /// Where this node sits in the list of its parent.
    ///
    /// # Safety
    /// The parent pointer must be null or name a live node.
    unsafe fn index_in_parent(&self) -> Option<(*mut XmlNode, usize)> {
        let p = self.parent.as_mut()?;
        let me = self as *const XmlNode;
        let k = p.children.iter().position(|c| std::ptr::eq(&**c, me))?;
        Some((p, k))
    }
}

struct OutletBox {
    outlet: Outlet,
    info: StreamInfo,
}

struct InletBox {
    inlet: Option<Inlet>,
    info: StreamInfo,
    buffer_samples: i32,
    recover: bool,
    postproc: u32,
    /// The half-time of the smoothing filter, in seconds.
    /// `src/api_config.cpp` names 90 as the default.
    halftime: f32,
}

impl InletBox {
    /// Open the stream if it is not open yet.
    ///
    /// liblsl starts the reader on the first pull, so `lsl_open_stream` is not
    /// required (`src/data_receiver.cpp:88`). An application that pulls
    /// straight after it creates the inlet must get samples, not an error.
    fn ensure_open(&mut self, timeout: f64) -> bool {
        if self.inlet.is_some() {
            return true;
        }
        let t = Duration::from_secs_f64(if timeout <= 0.0 { 32.0 } else { timeout });
        let opened = if self.recover {
            Inlet::open_recovering(&self.info, t, self.buffer_samples)
        } else {
            Inlet::open_with_buffer(&self.info, t, self.buffer_samples)
        };
        match opened {
            Ok(mut i) => {
                if self.postproc != 0 {
                    i.set_postprocessing(self.postproc);
                }
                self.inlet = Some(i);
                true
            }
            Err(_) => false,
        }
    }
}

fn format_from_c(v: c_int) -> Format {
    Format::from_u8(v as u8).unwrap_or(Format::Undefined)
}

/// Read a C string, treating a null pointer as empty.
///
/// # Safety
/// The pointer must be null or a valid zero-terminated string.
unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

/// Hand a buffer of bytes to the caller, with a zero after them.
///
/// liblsl allocates these with `malloc` and frees them with `free`
/// (`src/lsl_streaminfo_c.cpp:63`, `src/common.cpp:53`). The same allocator
/// has to be used here. A C application is allowed to call `free` on the
/// result, and a Rust allocation would corrupt the heap.
fn out_bytes(bytes: &[u8]) -> *mut c_char {
    // SAFETY: the size is the length plus one byte for the terminator, and
    // every byte of the result is written before it is handed out.
    unsafe {
        let p = libc::malloc(bytes.len() + 1) as *mut u8;
        if p.is_null() {
            return std::ptr::null_mut();
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        *p.add(bytes.len()) = 0;
        p as *mut c_char
    }
}

/// Hand a string to the caller. The caller frees it with `lsl_destroy_string`.
fn out_string(s: &str) -> *mut c_char {
    out_bytes(s.as_bytes())
}

/// A string that the caller must not free.
///
/// liblsl returns a pointer into its own storage for these. Each one here
/// leaks a small allocation once per distinct value, which keeps the pointer
/// valid for the life of the process.
fn static_string(s: &str) -> *const c_char {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::OnceLock;
    static CACHE: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = cache.lock().unwrap();
    if let Some(p) = map.get(s) {
        return *p as *const c_char;
    }
    let c = match CString::new(s) {
        Ok(c) => c,
        Err(_) => return std::ptr::null(),
    };
    let p = c.into_raw();
    map.insert(s.to_string(), p as usize);
    p as *const c_char
}

// ---------------------------------------------------------------------------
// library information
// ---------------------------------------------------------------------------

/// The protocol version that this build speaks.
#[no_mangle]
pub extern "C" fn lsl_protocol_version() -> i32 {
    LSL_PROTOCOL_VERSION
}

/// The library version.
#[no_mangle]
pub extern "C" fn lsl_library_version() -> i32 {
    LSL_LIBRARY_VERSION
}

/// A line of text describing this build.
#[no_mangle]
pub extern "C" fn lsl_library_info() -> *const c_char {
    static_string("lsl-rust 0.0.0 (protocol 1.10)")
}

/// The clock that timestamps use. SPEC.md 3.0.
#[no_mangle]
pub extern "C" fn lsl_local_clock() -> c_double {
    labstream_net::clock()
}

/// Free a string that the library returned.
///
/// # Safety
/// The pointer must have come from this library and must not be used again.
#[no_mangle]
pub unsafe extern "C" fn lsl_destroy_string(s: *mut c_char) {
    if !s.is_null() {
        libc::free(s as *mut c_void);
    }
}

// ---------------------------------------------------------------------------
// the stream description
// ---------------------------------------------------------------------------

/// Build a stream description.
///
/// # Safety
/// Every string pointer must be null or zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_create_streaminfo(
    name: *const c_char,
    stype: *const c_char,
    channel_count: c_int,
    nominal_srate: c_double,
    channel_format: c_int,
    source_id: *const c_char,
) -> lsl_streaminfo {
    let mut info = StreamInfo::new(
        &cstr(name),
        &cstr(stype),
        channel_count.max(0) as u32,
        format_from_c(channel_format),
        nominal_srate,
    );
    info.source_id = cstr(source_id);
    InfoBox::new(info) as lsl_streaminfo
}

/// Release a stream description.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_destroy_streaminfo(info: lsl_streaminfo) {
    if !info.is_null() {
        drop(Box::from_raw(info as *mut InfoBox));
    }
}

/// Copy a stream description.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_copy_streaminfo(info: lsl_streaminfo) -> lsl_streaminfo {
    match (info as *mut InfoBox).as_mut() {
        Some(b) => {
            b.sync();
            InfoBox::new(b.info.clone()) as lsl_streaminfo
        }
        None => std::ptr::null_mut(),
    }
}

macro_rules! info_getter_str {
    ($name:ident, $field:ident) => {
        /// Read a field of the description.
        ///
        /// # Safety
        /// The handle must have come from this library.
        #[no_mangle]
        pub unsafe extern "C" fn $name(info: lsl_streaminfo) -> *const c_char {
            match (info as *mut InfoBox).as_ref() {
                Some(b) => static_string(&b.info.$field),
                None => static_string(""),
            }
        }
    };
}

info_getter_str!(lsl_get_name, name);
info_getter_str!(lsl_get_source_id, source_id);
info_getter_str!(lsl_get_session_id, session_id);
info_getter_str!(lsl_get_hostname, hostname);
info_getter_str!(lsl_get_uid, uid);

/// The content type of the stream.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_get_type(info: lsl_streaminfo) -> *const c_char {
    match (info as *mut InfoBox).as_ref() {
        Some(b) => static_string(&b.info.stream_type),
        None => static_string(""),
    }
}

/// The channel count.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_get_channel_count(info: lsl_streaminfo) -> i32 {
    match (info as *mut InfoBox).as_ref() {
        Some(b) => b.info.channel_count as i32,
        None => 0,
    }
}

/// The nominal rate. Zero means no fixed rate.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_get_nominal_srate(info: lsl_streaminfo) -> c_double {
    match (info as *mut InfoBox).as_ref() {
        Some(b) => b.info.nominal_srate,
        None => 0.0,
    }
}

/// The channel format.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_get_channel_format(info: lsl_streaminfo) -> i32 {
    match (info as *mut InfoBox).as_ref() {
        Some(b) => b.info.format as i32,
        None => 0,
    }
}

/// The clock reading when the stream was created.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_get_created_at(info: lsl_streaminfo) -> c_double {
    match (info as *mut InfoBox).as_ref() {
        Some(b) => b.info.created_at,
        None => 0.0,
    }
}

/// The protocol version that the sender speaks.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_get_version(_info: lsl_streaminfo) -> i32 {
    LSL_PROTOCOL_VERSION
}

/// The whole description as XML. The caller frees the result.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_get_xml(info: lsl_streaminfo) -> *mut c_char {
    match (info as *mut InfoBox).as_mut() {
        Some(b) => {
            b.sync();
            out_string(&b.info.to_fullinfo_xml())
        }
        None => out_string(""),
    }
}

/// The number of bytes in one channel value. Zero for a string.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_get_channel_bytes(info: lsl_streaminfo) -> i32 {
    match (info as *mut InfoBox).as_ref() {
        Some(b) => b.info.format.width().unwrap_or(0) as i32,
        None => 0,
    }
}

/// The number of bytes in one sample.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_get_sample_bytes(info: lsl_streaminfo) -> i32 {
    unsafe { lsl_get_channel_bytes(info) * lsl_get_channel_count(info) }
}

// ---------------------------------------------------------------------------
// the description tree
// ---------------------------------------------------------------------------
//
// Every call below matches one line of `src/lsl_xml_element_c.cpp`, which is a
// thin wrapper over pugixml. Three of those lines carry behavior that a reader
// of the header cannot guess:
//
//   lsl_append_child_value  returns the **parent**, not the new child.
//   lsl_value               returns nothing for an element. The text sits in a
//                           child node, so `lsl_child_value` is the call that
//                           reads it.
//   lsl_set_value           fails on an element, because pugixml only lets a
//                           text node carry a value.
//
// A null pointer stands for "no node". `lsl_empty` asks exactly that.

/// The root of the description tree.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_get_desc(info: lsl_streaminfo) -> lsl_xml_ptr {
    match (info as *mut InfoBox).as_mut() {
        Some(b) => b.desc_node() as *mut XmlNode as lsl_xml_ptr,
        None => std::ptr::null_mut(),
    }
}

/// Read a node pointer, or return the default when it is null.
macro_rules! node {
    ($e:expr, $bad:expr) => {
        match ($e as *mut XmlNode).as_mut() {
            Some(n) => n,
            None => return $bad,
        }
    };
}

fn as_ptr(n: &mut XmlNode) -> lsl_xml_ptr {
    n as *mut XmlNode as lsl_xml_ptr
}

// --- reading -----------------------------------------------------------

/// The name of a node. A text node has none.
///
/// # Safety
/// The pointer must be null or name a node of a live description.
#[no_mangle]
pub unsafe extern "C" fn lsl_name(e: lsl_xml_ptr) -> *const c_char {
    let n = node!(e, static_string(""));
    if n.kind == desc::Kind::Element {
        static_string(&n.name)
    } else {
        static_string("")
    }
}

/// The value of a node.
///
/// An element carries no value of its own. Its text sits in a child node, and
/// `lsl_child_value` reads that. `src/lsl_xml_element_c.cpp:38`.
///
/// # Safety
/// The pointer must be null or name a node of a live description.
#[no_mangle]
pub unsafe extern "C" fn lsl_value(e: lsl_xml_ptr) -> *const c_char {
    let n = node!(e, static_string(""));
    if n.kind == desc::Kind::Element {
        static_string("")
    } else {
        static_string(&n.value)
    }
}

/// The text of the first child that holds text.
///
/// # Safety
/// The pointer must be null or name a node of a live description.
#[no_mangle]
pub unsafe extern "C" fn lsl_child_value(e: lsl_xml_ptr) -> *const c_char {
    let n = node!(e, static_string(""));
    match n.children.iter().find(|c| c.kind != desc::Kind::Element) {
        Some(c) => static_string(&c.value),
        None => static_string(""),
    }
}

/// The text of the first child with this name.
///
/// # Safety
/// The pointer must be null or name a node of a live description, and the name
/// must be null or zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_child_value_n(e: lsl_xml_ptr, name: *const c_char) -> *const c_char {
    let child = lsl_child(e, name);
    if child.is_null() {
        return static_string("");
    }
    lsl_child_value(child)
}

/// True when the pointer names no node.
///
/// # Safety
/// The pointer must be null or name a node of a live description.
#[no_mangle]
pub unsafe extern "C" fn lsl_empty(e: lsl_xml_ptr) -> i32 {
    e.is_null() as i32
}

/// True when a node holds text rather than a tag.
///
/// # Safety
/// The pointer must be null or name a node of a live description.
#[no_mangle]
pub unsafe extern "C" fn lsl_is_text(e: lsl_xml_ptr) -> i32 {
    let n = node!(e, 0);
    (n.document || n.kind != desc::Kind::Element) as i32
}

// --- moving through the tree -------------------------------------------

/// The first child of a node.
///
/// # Safety
/// The pointer must be null or name a node of a live description.
#[no_mangle]
pub unsafe extern "C" fn lsl_first_child(e: lsl_xml_ptr) -> lsl_xml_ptr {
    let n = node!(e, std::ptr::null_mut());
    match n.children.first_mut() {
        Some(c) => as_ptr(c),
        None => std::ptr::null_mut(),
    }
}

/// The last child of a node.
///
/// # Safety
/// The pointer must be null or name a node of a live description.
#[no_mangle]
pub unsafe extern "C" fn lsl_last_child(e: lsl_xml_ptr) -> lsl_xml_ptr {
    let n = node!(e, std::ptr::null_mut());
    match n.children.last_mut() {
        Some(c) => as_ptr(c),
        None => std::ptr::null_mut(),
    }
}

/// The node above this one.
///
/// The root of the tree is the `<desc>` element. Above it sits `<info>`, and
/// above that sits the document. `oracle/desctree.c` measures both.
///
/// # Safety
/// The pointer must be null or name a node of a live description.
#[no_mangle]
pub unsafe extern "C" fn lsl_parent(e: lsl_xml_ptr) -> lsl_xml_ptr {
    let n = node!(e, std::ptr::null_mut());
    if n.parent.is_null() {
        std::ptr::null_mut()
    } else {
        n.parent as lsl_xml_ptr
    }
}

/// The first child with this name.
///
/// # Safety
/// The pointer must be null or name a node of a live description, and the name
/// must be null or zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_child(e: lsl_xml_ptr, name: *const c_char) -> lsl_xml_ptr {
    let n = node!(e, std::ptr::null_mut());
    let want = cstr(name);
    match n.children.iter_mut().find(|c| c.name == want) {
        Some(c) => as_ptr(c),
        None => std::ptr::null_mut(),
    }
}

/// Step to a sibling.
///
/// `step` is 1 for the next one and -1 for the one before. An empty name
/// accepts any sibling.
///
/// # Safety
/// The pointer must name a node of a live description.
unsafe fn sibling(e: lsl_xml_ptr, step: isize, want: Option<String>) -> lsl_xml_ptr {
    let n = node!(e, std::ptr::null_mut());
    let (parent, here) = match n.index_in_parent() {
        Some(x) => x,
        None => return std::ptr::null_mut(),
    };
    let parent = match parent.as_mut() {
        Some(p) => p,
        None => return std::ptr::null_mut(),
    };
    let count = parent.children.len() as isize;
    let mut k = here as isize + step;
    while k >= 0 && k < count {
        let c = &mut parent.children[k as usize];
        match &want {
            None => return as_ptr(c),
            Some(w) if c.name == *w => return as_ptr(c),
            Some(_) => {}
        }
        k += step;
    }
    std::ptr::null_mut()
}

/// The sibling that follows this node.
///
/// # Safety
/// The pointer must be null or name a node of a live description.
#[no_mangle]
pub unsafe extern "C" fn lsl_next_sibling(e: lsl_xml_ptr) -> lsl_xml_ptr {
    sibling(e, 1, None)
}

/// The sibling before this node.
///
/// # Safety
/// The pointer must be null or name a node of a live description.
#[no_mangle]
pub unsafe extern "C" fn lsl_previous_sibling(e: lsl_xml_ptr) -> lsl_xml_ptr {
    sibling(e, -1, None)
}

/// The next sibling with this name.
///
/// # Safety
/// The pointer must be null or name a node of a live description, and the name
/// must be null or zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_next_sibling_n(e: lsl_xml_ptr, name: *const c_char) -> lsl_xml_ptr {
    sibling(e, 1, Some(cstr(name)))
}

/// The sibling before this one with this name.
///
/// # Safety
/// The pointer must be null or name a node of a live description, and the name
/// must be null or zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_previous_sibling_n(
    e: lsl_xml_ptr,
    name: *const c_char,
) -> lsl_xml_ptr {
    sibling(e, -1, Some(cstr(name)))
}

// --- changing the tree -------------------------------------------------

/// Add a tag at the end and return it.
///
/// # Safety
/// The pointer must be null or name a node of a live description, and the name
/// must be null or zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_append_child(e: lsl_xml_ptr, name: *const c_char) -> lsl_xml_ptr {
    let parent = node!(e, std::ptr::null_mut());
    let here: *mut XmlNode = parent;
    parent
        .children
        .push(XmlNode::new(desc::Kind::Element, &cstr(name), "", here));
    as_ptr(parent.children.last_mut().unwrap())
}

/// Add a tag at the start and return it.
///
/// # Safety
/// The pointer must be null or name a node of a live description, and the name
/// must be null or zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_prepend_child(e: lsl_xml_ptr, name: *const c_char) -> lsl_xml_ptr {
    let parent = node!(e, std::ptr::null_mut());
    let here: *mut XmlNode = parent;
    parent
        .children
        .insert(0, XmlNode::new(desc::Kind::Element, &cstr(name), "", here));
    as_ptr(&mut parent.children[0])
}

/// Add a tag that holds text, at the end.
///
/// **The result is the parent**, not the new tag
/// (`src/lsl_xml_element_c.cpp:74`). An application can therefore chain
/// several calls on one node.
///
/// # Safety
/// The pointer must be null or name a node of a live description, and both
/// strings must be null or zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_append_child_value(
    e: lsl_xml_ptr,
    name: *const c_char,
    value: *const c_char,
) -> lsl_xml_ptr {
    let child = lsl_append_child(e, name);
    if let Some(c) = (child as *mut XmlNode).as_mut() {
        let here: *mut XmlNode = c;
        c.children
            .push(XmlNode::new(desc::Kind::Text, "", &cstr(value), here));
    }
    e
}

/// Add a tag that holds text, at the start.
///
/// The result is the parent, the same way `lsl_append_child_value` returns it.
///
/// # Safety
/// The pointer must be null or name a node of a live description, and both
/// strings must be null or zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_prepend_child_value(
    e: lsl_xml_ptr,
    name: *const c_char,
    value: *const c_char,
) -> lsl_xml_ptr {
    let child = lsl_prepend_child(e, name);
    if let Some(c) = (child as *mut XmlNode).as_mut() {
        let here: *mut XmlNode = c;
        c.children
            .push(XmlNode::new(desc::Kind::Text, "", &cstr(value), here));
    }
    e
}

/// Replace the name of a tag.
///
/// A text node has no name, and the call reports a failure for one.
///
/// # Safety
/// The pointer must be null or name a node of a live description, and the name
/// must be null or zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_set_name(e: lsl_xml_ptr, name: *const c_char) -> i32 {
    let n = node!(e, 0);
    if n.kind != desc::Kind::Element {
        return 0;
    }
    n.name = cstr(name);
    1
}

/// Replace the value of a text node.
///
/// An element carries no value, and the call reports a failure for one. That
/// is what pugixml does (`src/lsl_xml_element_c.cpp:47`).
///
/// # Safety
/// The pointer must be null or name a node of a live description, and the
/// value must be null or zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_set_value(e: lsl_xml_ptr, value: *const c_char) -> i32 {
    let n = node!(e, 0);
    if n.kind == desc::Kind::Element {
        return 0;
    }
    n.value = cstr(value);
    1
}

/// Replace the text of a named child.
///
/// liblsl sets the value of the **first** child of that tag, whatever it is
/// (`src/lsl_xml_element_c.cpp:69`). A tag whose first child is another tag
/// therefore reports a failure and changes nothing.
///
/// # Safety
/// The pointer must be null or name a node of a live description, and both
/// strings must be null or zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_set_child_value(
    e: lsl_xml_ptr,
    name: *const c_char,
    value: *const c_char,
) -> i32 {
    let child = lsl_child(e, name);
    let first = lsl_first_child(child);
    lsl_set_value(first, value)
}

/// Copy a subtree in at the end, and return the copy.
///
/// # Safety
/// Both pointers must be null or name a node of a live description.
#[no_mangle]
pub unsafe extern "C" fn lsl_append_copy(e: lsl_xml_ptr, source: lsl_xml_ptr) -> lsl_xml_ptr {
    let src = match (source as *mut XmlNode).as_ref() {
        Some(s) => s.to_desc(),
        None => return std::ptr::null_mut(),
    };
    let parent = node!(e, std::ptr::null_mut());
    let here: *mut XmlNode = parent;
    parent.children.push(XmlNode::from_desc(&src, here));
    as_ptr(parent.children.last_mut().unwrap())
}

/// Copy a subtree in at the start, and return the copy.
///
/// # Safety
/// Both pointers must be null or name a node of a live description.
#[no_mangle]
pub unsafe extern "C" fn lsl_prepend_copy(e: lsl_xml_ptr, source: lsl_xml_ptr) -> lsl_xml_ptr {
    let src = match (source as *mut XmlNode).as_ref() {
        Some(s) => s.to_desc(),
        None => return std::ptr::null_mut(),
    };
    let parent = node!(e, std::ptr::null_mut());
    let here: *mut XmlNode = parent;
    parent.children.insert(0, XmlNode::from_desc(&src, here));
    as_ptr(&mut parent.children[0])
}

/// Remove the first child with this name.
///
/// # Safety
/// The pointer must be null or name a node of a live description, and the name
/// must be null or zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_remove_child_n(e: lsl_xml_ptr, name: *const c_char) {
    let n = match (e as *mut XmlNode).as_mut() {
        Some(n) => n,
        None => return,
    };
    let want = cstr(name);
    if let Some(k) = n.children.iter().position(|c| c.name == want) {
        n.children.remove(k);
    }
}

/// Remove one subtree.
///
/// Every pointer into that subtree stops being valid.
///
/// # Safety
/// Both pointers must be null or name a node of a live description.
#[no_mangle]
pub unsafe extern "C" fn lsl_remove_child(e: lsl_xml_ptr, child: lsl_xml_ptr) {
    let n = match (e as *mut XmlNode).as_mut() {
        Some(n) => n,
        None => return,
    };
    let target = child as *const XmlNode;
    if target.is_null() {
        return;
    }
    if let Some(k) = n
        .children
        .iter()
        .position(|c| std::ptr::eq(&**c as *const XmlNode, target))
    {
        n.children.remove(k);
    }
}

// ---------------------------------------------------------------------------
// the outlet
// ---------------------------------------------------------------------------

/// Publish a stream.
///
/// # Safety
/// The description handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_create_outlet(
    info: lsl_streaminfo,
    chunk_size: c_int,
    max_buffered: c_int,
) -> lsl_outlet {
    lsl_create_outlet_ex(info, chunk_size, max_buffered, 0)
}

/// Publish a stream, with transport flags.
///
/// # Safety
/// The description handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_create_outlet_ex(
    info: lsl_streaminfo,
    _chunk_size: c_int,
    _max_buffered: c_int,
    flags: c_int,
) -> lsl_outlet {
    let b = match (info as *mut InfoBox).as_mut() {
        Some(b) => b,
        None => return std::ptr::null_mut(),
    };
    // liblsl builds the answer for `LSL:fullinfo` once, when the server starts
    // (`src/tcp_server.cpp:378`). A change to the tree after this call never
    // reaches a consumer, on either implementation.
    b.sync();
    // `transp_sync_blocking` writes each sample before the push returns.
    // `include/lsl/common.h:172`.
    const SYNC_BLOCKING: c_int = 4;
    let built = if flags & SYNC_BLOCKING != 0 {
        Outlet::new_blocking(b.info.clone())
    } else {
        Outlet::new(b.info.clone())
    };
    match built {
        Ok(outlet) => {
            let published = outlet.info().clone();
            Box::into_raw(Box::new(OutletBox {
                outlet,
                info: published,
            })) as lsl_outlet
        }
        Err(_) => std::ptr::null_mut(),
    }
}

/// Stop a stream.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_destroy_outlet(out: lsl_outlet) {
    if !out.is_null() {
        drop(Box::from_raw(out as *mut OutletBox));
    }
}

/// The description that an outlet publishes.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_get_info(out: lsl_outlet) -> lsl_streaminfo {
    match (out as *mut OutletBox).as_ref() {
        Some(b) => InfoBox::new(b.info.clone()) as lsl_streaminfo,
        None => std::ptr::null_mut(),
    }
}

/// True when at least one consumer is connected.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_have_consumers(out: lsl_outlet) -> i32 {
    match (out as *mut OutletBox).as_ref() {
        Some(b) => (b.outlet.consumer_count() > 0) as i32,
        None => 0,
    }
}

/// Wait until a consumer connects.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_wait_for_consumers(out: lsl_outlet, timeout: c_double) -> i32 {
    match (out as *mut OutletBox).as_ref() {
        Some(b) => {
            b.outlet
                .wait_for_consumers(Duration::from_secs_f64(timeout.max(0.0))) as i32
        }
        None => 0,
    }
}

unsafe fn push_typed(
    out: lsl_outlet,
    build: impl Fn(usize) -> Value,
    timestamp: c_double,
    _pushthrough: c_int,
) -> i32 {
    let b = match (out as *mut OutletBox).as_ref() {
        Some(b) => b,
        None => return LSL_ARGUMENT_ERROR,
    };
    let n = b.info.channel_count as usize;
    // A timestamp of zero means the current clock. SPEC.md 8.3.
    let t = if timestamp == 0.0 {
        labstream_net::clock()
    } else {
        timestamp
    };
    let values: Vec<Value> = (0..n).map(&build).collect();
    b.outlet.push(&Sample {
        timestamp: t,
        values,
    });
    LSL_NO_ERROR
}

macro_rules! push_sample {
    ($name:ident, $ty:ty, $variant:path) => {
        /// Send one sample.
        ///
        /// # Safety
        /// The handle must have come from this library, and the buffer must
        /// hold one value for each channel.
        #[no_mangle]
        pub unsafe extern "C" fn $name(
            out: lsl_outlet,
            data: *const $ty,
            timestamp: c_double,
            pushthrough: c_int,
        ) -> i32 {
            if data.is_null() {
                return LSL_ARGUMENT_ERROR;
            }
            push_typed(out, |i| $variant(*data.add(i)), timestamp, pushthrough)
        }
    };
}

push_sample!(lsl_push_sample_ftp, f32, Value::F32);
push_sample!(lsl_push_sample_dtp, f64, Value::F64);
push_sample!(lsl_push_sample_itp, i32, Value::I32);
push_sample!(lsl_push_sample_stp, i16, Value::I16);

/// Send one sample of bytes.
///
/// # Safety
/// The handle must have come from this library, and the buffer must hold one
/// value for each channel.
#[no_mangle]
pub unsafe extern "C" fn lsl_push_sample_ctp(
    out: lsl_outlet,
    data: *const c_char,
    timestamp: c_double,
    pushthrough: c_int,
) -> i32 {
    if data.is_null() {
        return LSL_ARGUMENT_ERROR;
    }
    push_typed(
        out,
        |i| Value::I8(*data.add(i) as _),
        timestamp,
        pushthrough,
    )
}
push_sample!(lsl_push_sample_ltp, i64, Value::I64);

/// Send one sample of strings.
///
/// # Safety
/// The handle must have come from this library, and the buffer must hold one
/// zero-terminated string for each channel.
#[no_mangle]
pub unsafe extern "C" fn lsl_push_sample_strtp(
    out: lsl_outlet,
    data: *const *const c_char,
    timestamp: c_double,
    pushthrough: c_int,
) -> i32 {
    if data.is_null() {
        return LSL_ARGUMENT_ERROR;
    }
    push_typed(
        out,
        |i| Value::Str(cstr(*data.add(i)).into_bytes()),
        timestamp,
        pushthrough,
    )
}

/// Send several samples that share one timestamp. SPEC.md 9.2.
///
/// The timestamp names the **last** sample. A stream with a rate counts
/// backward from it, so the first sample of the chunk is dated
/// `timestamp - (count - 1) / rate`. Only that first sample carries a
/// timestamp on the wire. The rest carry the deduced tag.
unsafe fn push_chunk_typed(
    out: lsl_outlet,
    count_values: u64,
    build: impl Fn(usize) -> Value,
    timestamp: c_double,
    _pushthrough: c_int,
) -> i32 {
    let b = match (out as *mut OutletBox).as_ref() {
        Some(b) => b,
        None => return LSL_ARGUMENT_ERROR,
    };
    let chans = b.info.channel_count as usize;
    if chans == 0 {
        return LSL_ARGUMENT_ERROR;
    }
    let total = count_values as usize;
    if total % chans != 0 {
        // A buffer that is not a whole number of samples is an error.
        return LSL_ARGUMENT_ERROR;
    }
    let samples = total / chans;
    if samples == 0 {
        return LSL_NO_ERROR;
    }

    let mut first = if timestamp == 0.0 {
        labstream_net::clock()
    } else {
        timestamp
    };
    if b.info.nominal_srate != LSL_IRREGULAR_RATE {
        first -= (samples - 1) as f64 / b.info.nominal_srate;
    }

    for k in 0..samples {
        let values: Vec<Value> = (0..chans).map(|c| build(k * chans + c)).collect();
        b.outlet.push(&Sample {
            timestamp: if k == 0 { first } else { LSL_DEDUCED_TIMESTAMP },
            values,
        });
    }
    LSL_NO_ERROR
}

/// Send several samples, each with its own timestamp.
unsafe fn push_chunk_stamped(
    out: lsl_outlet,
    count_values: u64,
    build: impl Fn(usize) -> Value,
    stamps: *const c_double,
    _pushthrough: c_int,
) -> i32 {
    let b = match (out as *mut OutletBox).as_ref() {
        Some(b) => b,
        None => return LSL_ARGUMENT_ERROR,
    };
    if stamps.is_null() {
        return LSL_ARGUMENT_ERROR;
    }
    let chans = b.info.channel_count as usize;
    if chans == 0 || count_values as usize % chans != 0 {
        return LSL_ARGUMENT_ERROR;
    }
    let samples = count_values as usize / chans;
    for k in 0..samples {
        let values: Vec<Value> = (0..chans).map(|c| build(k * chans + c)).collect();
        let t = *stamps.add(k);
        b.outlet.push(&Sample {
            timestamp: if t == 0.0 { labstream_net::clock() } else { t },
            values,
        });
    }
    LSL_NO_ERROR
}

macro_rules! push_chunk {
    ($tp:ident, $tnp:ident, $ty:ty, $make:expr) => {
        /// Send several samples that share one timestamp. SPEC.md 9.2.
        ///
        /// # Safety
        /// The handle must have come from this library, and the buffer must
        /// hold `data_elements` values.
        #[no_mangle]
        pub unsafe extern "C" fn $tp(
            out: lsl_outlet,
            data: *const $ty,
            data_elements: u64,
            timestamp: c_double,
            pushthrough: c_int,
        ) -> i32 {
            if data.is_null() {
                return LSL_ARGUMENT_ERROR;
            }
            let f = $make;
            push_chunk_typed(out, data_elements, |i| f(data, i), timestamp, pushthrough)
        }

        /// Send several samples, each with its own timestamp.
        ///
        /// # Safety
        /// The handle must have come from this library, the data buffer must
        /// hold `data_elements` values, and the timestamp buffer must hold one
        /// value for each sample.
        #[no_mangle]
        pub unsafe extern "C" fn $tnp(
            out: lsl_outlet,
            data: *const $ty,
            data_elements: u64,
            stamps: *const c_double,
            pushthrough: c_int,
        ) -> i32 {
            if data.is_null() {
                return LSL_ARGUMENT_ERROR;
            }
            let f = $make;
            push_chunk_stamped(out, data_elements, |i| f(data, i), stamps, pushthrough)
        }
    };
}

push_chunk!(
    lsl_push_chunk_ftp,
    lsl_push_chunk_ftnp,
    f32,
    |d: *const f32, i: usize| Value::F32(*d.add(i))
);
push_chunk!(
    lsl_push_chunk_dtp,
    lsl_push_chunk_dtnp,
    f64,
    |d: *const f64, i: usize| Value::F64(*d.add(i))
);
push_chunk!(
    lsl_push_chunk_itp,
    lsl_push_chunk_itnp,
    i32,
    |d: *const i32, i: usize| Value::I32(*d.add(i))
);
push_chunk!(
    lsl_push_chunk_stp,
    lsl_push_chunk_stnp,
    i16,
    |d: *const i16, i: usize| Value::I16(*d.add(i))
);
push_chunk!(
    lsl_push_chunk_ctp,
    lsl_push_chunk_ctnp,
    c_char,
    |d: *const c_char, i: usize| Value::I8(*d.add(i) as _)
);
push_chunk!(
    lsl_push_chunk_ltp,
    lsl_push_chunk_ltnp,
    i64,
    |d: *const i64, i: usize| Value::I64(*d.add(i))
);
push_chunk!(
    lsl_push_chunk_strtp,
    lsl_push_chunk_strtnp,
    *const c_char,
    |d: *const *const c_char, i: usize| Value::Str(cstr(*d.add(i)).into_bytes())
);

// ---------------------------------------------------------------------------
// resolving
// ---------------------------------------------------------------------------

unsafe fn fill_results(
    found: Vec<StreamInfo>,
    buffer: *mut lsl_streaminfo,
    buffer_elements: u32,
) -> i32 {
    if buffer.is_null() {
        return LSL_ARGUMENT_ERROR;
    }
    let n = found.len().min(buffer_elements as usize);
    for (i, info) in found.into_iter().take(n).enumerate() {
        let h = InfoBox::new(info) as lsl_streaminfo;
        *buffer.add(i) = h;
    }
    n as i32
}

/// Find every stream on the network.
///
/// # Safety
/// The buffer must hold `buffer_elements` handles.
#[no_mangle]
pub unsafe extern "C" fn lsl_resolve_all(
    buffer: *mut lsl_streaminfo,
    buffer_elements: u32,
    wait_time: c_double,
) -> i32 {
    let found = labstream_net::resolve(
        &format!("session_id='{}'", session_id()),
        1,
        Duration::from_secs_f64(wait_time.max(0.1)),
    )
    .unwrap_or_default();
    fill_results(found, buffer, buffer_elements)
}

/// Find every stream whose named field holds a value.
///
/// # Safety
/// The buffer must hold `buffer_elements` handles, and the strings must be
/// zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_resolve_byprop(
    buffer: *mut lsl_streaminfo,
    buffer_elements: u32,
    prop: *const c_char,
    value: *const c_char,
    minimum: c_int,
    timeout: c_double,
) -> i32 {
    let q = labstream_net::build_property_query(&session_id(), &cstr(prop), &cstr(value));
    let found = labstream_net::resolve(
        &q,
        minimum.max(1) as usize,
        Duration::from_secs_f64(timeout.max(0.1)),
    )
    .unwrap_or_default();
    fill_results(found, buffer, buffer_elements)
}

/// Find every stream that matches a query.
///
/// # Safety
/// The buffer must hold `buffer_elements` handles.
#[no_mangle]
pub unsafe extern "C" fn lsl_resolve_bypred(
    buffer: *mut lsl_streaminfo,
    buffer_elements: u32,
    pred: *const c_char,
    minimum: c_int,
    timeout: c_double,
) -> i32 {
    let found = labstream_net::resolve(
        &cstr(pred),
        minimum.max(1) as usize,
        Duration::from_secs_f64(timeout.max(0.1)),
    )
    .unwrap_or_default();
    fill_results(found, buffer, buffer_elements)
}

/// Start a background resolver for every stream of this session.
///
/// The query is the one that liblsl builds with no property
/// (`src/resolver_impl.cpp:66`).
///
/// # Safety
/// The result must be released with `lsl_destroy_continuous_resolver`.
#[no_mangle]
pub unsafe extern "C" fn lsl_create_continuous_resolver(
    forget_after: c_double,
) -> lsl_continuous_resolver {
    make_resolver(format!("session_id='{}'", session_id()), forget_after)
}

/// Start a background resolver for one field.
///
/// # Safety
/// The strings must be zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_create_continuous_resolver_byprop(
    prop: *const c_char,
    value: *const c_char,
    forget_after: c_double,
) -> lsl_continuous_resolver {
    let query = labstream_net::build_property_query(&session_id(), &cstr(prop), &cstr(value));
    make_resolver(query, forget_after)
}

/// Start a background resolver for a query.
///
/// liblsl joins the query to the session test with `and`
/// (`src/resolver_impl.cpp:70`), so a stream of another session never
/// matches.
///
/// # Safety
/// The string must be zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_create_continuous_resolver_bypred(
    pred: *const c_char,
    forget_after: c_double,
) -> lsl_continuous_resolver {
    let query = format!("session_id='{}' and {}", session_id(), cstr(pred));
    make_resolver(query, forget_after)
}

/// The session that this process belongs to. `lab.SessionID`.
fn session_id() -> String {
    labstream_net::config::get().session_id.clone()
}

fn make_resolver(query: String, forget_after: c_double) -> lsl_continuous_resolver {
    // liblsl treats a value of zero or below as "keep every stream".
    let forget = if forget_after > 0.0 {
        forget_after
    } else {
        f64::INFINITY
    };
    Box::into_raw(Box::new(ContinuousResolver::new(&query, forget))) as lsl_continuous_resolver
}

/// Read what a background resolver found.
///
/// The caller releases each handle with `lsl_destroy_streaminfo`.
///
/// # Safety
/// The handle must have come from this library, and the buffer must hold
/// `buffer_elements` handles.
#[no_mangle]
pub unsafe extern "C" fn lsl_resolver_results(
    res: lsl_continuous_resolver,
    buffer: *mut lsl_streaminfo,
    buffer_elements: u32,
) -> i32 {
    let r = match (res as *mut ContinuousResolver).as_ref() {
        Some(r) => r,
        None => return LSL_ARGUMENT_ERROR,
    };
    if buffer.is_null() {
        return LSL_ARGUMENT_ERROR;
    }
    let found = r.results(buffer_elements as usize);
    for (k, info) in found.iter().enumerate() {
        *buffer.add(k) = InfoBox::new(info.clone()) as lsl_streaminfo;
    }
    found.len() as i32
}

/// Stop a background resolver.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_destroy_continuous_resolver(res: lsl_continuous_resolver) {
    if !res.is_null() {
        drop(Box::from_raw(res as *mut ContinuousResolver));
    }
}

/// Subscribe to a stream.
///
/// # Safety
/// The description handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_create_inlet(
    info: lsl_streaminfo,
    max_buflen: c_int,
    max_chunklen: c_int,
    recover: c_int,
) -> lsl_inlet {
    lsl_create_inlet_ex(info, max_buflen, max_chunklen, recover, 0)
}

/// Subscribe to a stream, with transport flags.
///
/// The buffer length converts here, at the boundary, exactly as liblsl does
/// (`src/lsl_inlet_c.cpp:20`). The wire only ever carries samples. SPEC.md 8.6.
///
/// # Safety
/// The description handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_create_inlet_ex(
    info: lsl_streaminfo,
    max_buflen: c_int,
    _max_chunklen: c_int,
    recover: c_int,
    flags: c_int,
) -> lsl_inlet {
    let b = match (info as *mut InfoBox).as_ref() {
        Some(b) => b,
        None => return std::ptr::null_mut(),
    };
    let buffer_samples = b.info.buffer_samples(max_buflen, flags as u32);
    Box::into_raw(Box::new(InletBox {
        inlet: None,
        info: b.info.clone(),
        buffer_samples,
        recover: recover != 0,
        postproc: 0,
        halftime: 90.0,
    })) as lsl_inlet
}

/// Release an inlet.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_destroy_inlet(inlet: lsl_inlet) {
    if !inlet.is_null() {
        drop(Box::from_raw(inlet as *mut InletBox));
    }
}

/// Open the data feed.
///
/// # Safety
/// The handle must have come from this library, and `ec` must be null or point
/// to one `int32`.
#[no_mangle]
pub unsafe extern "C" fn lsl_open_stream(inlet: lsl_inlet, timeout: c_double, ec: *mut i32) {
    let b = match (inlet as *mut InletBox).as_mut() {
        Some(b) => b,
        None => {
            if !ec.is_null() {
                *ec = LSL_ARGUMENT_ERROR;
            }
            return;
        }
    };
    if !ec.is_null() {
        *ec = if b.ensure_open(timeout) {
            LSL_NO_ERROR
        } else {
            LSL_TIMEOUT_ERROR
        };
    } else {
        b.ensure_open(timeout);
    }
}

/// Close the data feed.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_close_stream(inlet: lsl_inlet) {
    if let Some(b) = (inlet as *mut InletBox).as_mut() {
        b.inlet = None;
    }
}

/// The description of the connected stream.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_get_fullinfo(
    inlet: lsl_inlet,
    timeout: c_double,
    ec: *mut i32,
) -> lsl_streaminfo {
    let b = match (inlet as *mut InletBox).as_ref() {
        Some(b) => b,
        None => {
            if !ec.is_null() {
                *ec = LSL_ARGUMENT_ERROR;
            }
            return std::ptr::null_mut();
        }
    };
    if !ec.is_null() {
        *ec = LSL_NO_ERROR;
    }
    // The short description that discovery returned carries no tree. The tree
    // travels on the data port, so ask for it there. `src/info_receiver.cpp`.
    let info = match labstream_net::read_fullinfo(&b.info, Duration::from_secs_f64(timeout.max(0.1))) {
        Ok(full) => full,
        // liblsl blocks until the whole description arrives and reports a
        // timeout. A failure here leaves the short description, which holds
        // every field except the tree.
        Err(_) => b.info.clone(),
    };
    InfoBox::new(info) as lsl_streaminfo
}

/// Choose the post-processing stages. SPEC.md 8.5.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_set_postprocessing(inlet: lsl_inlet, flags: u32) -> i32 {
    match (inlet as *mut InletBox).as_mut() {
        Some(b) => {
            b.postproc = flags;
            if let Some(i) = b.inlet.as_mut() {
                i.set_postprocessing(flags);
            }
            LSL_NO_ERROR
        }
        None => LSL_ARGUMENT_ERROR,
    }
}

/// The measured offset between the two clocks. SPEC.md 3.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_time_correction(
    inlet: lsl_inlet,
    timeout: c_double,
    ec: *mut i32,
) -> c_double {
    let b = match (inlet as *mut InletBox).as_ref() {
        Some(b) => b,
        None => {
            if !ec.is_null() {
                *ec = LSL_ARGUMENT_ERROR;
            }
            return 0.0;
        }
    };
    match b.inlet.as_ref() {
        Some(i) => {
            let t = Duration::from_secs_f64(if timeout <= 0.0 { 5.0 } else { timeout });
            match i.wait_for_time_correction(t) {
                Some(v) => {
                    if !ec.is_null() {
                        *ec = LSL_NO_ERROR;
                    }
                    v
                }
                None => {
                    if !ec.is_null() {
                        *ec = LSL_TIMEOUT_ERROR;
                    }
                    0.0
                }
            }
        }
        None => {
            if !ec.is_null() {
                *ec = LSL_LOST_ERROR;
            }
            0.0
        }
    }
}

/// True when the connection reset since the last call.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_was_clock_reset(inlet: lsl_inlet) -> i32 {
    match (inlet as *mut InletBox).as_ref() {
        Some(b) => b
            .inlet
            .as_ref()
            .map(|i| (i.recoveries() > 0) as i32)
            .unwrap_or(0),
        None => 0,
    }
}

/// How many samples wait for the application.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_samples_available(inlet: lsl_inlet) -> u32 {
    match (inlet as *mut InletBox).as_ref() {
        Some(b) => b
            .inlet
            .as_ref()
            .map(|i| i.samples_available() as u32)
            .unwrap_or(0),
        None => 0,
    }
}

/// Drop everything waiting and report how many went.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_inlet_flush(inlet: lsl_inlet) -> u32 {
    let b = match (inlet as *mut InletBox).as_mut() {
        Some(b) => b,
        None => return 0,
    };
    let mut n = 0;
    if let Some(i) = b.inlet.as_mut() {
        while i.pull(Duration::from_millis(0)).ok().flatten().is_some() {
            n += 1;
        }
    }
    n
}

unsafe fn pull_typed(
    inlet: lsl_inlet,
    timeout: c_double,
    ec: *mut i32,
    mut store: impl FnMut(usize, &Value),
    capacity: usize,
) -> c_double {
    let b = match (inlet as *mut InletBox).as_mut() {
        Some(b) => b,
        None => {
            if !ec.is_null() {
                *ec = LSL_ARGUMENT_ERROR;
            }
            return 0.0;
        }
    };
    // liblsl starts the reader on the first pull. `src/data_receiver.cpp:88`.
    b.ensure_open(timeout);
    let i = match b.inlet.as_mut() {
        Some(i) => i,
        None => {
            if !ec.is_null() {
                *ec = LSL_LOST_ERROR;
            }
            return 0.0;
        }
    };
    match i.pull(Duration::from_secs_f64(timeout.max(0.0))) {
        Ok(Some(s)) => {
            for (k, v) in s.values.iter().enumerate().take(capacity) {
                store(k, v);
            }
            if !ec.is_null() {
                *ec = LSL_NO_ERROR;
            }
            s.timestamp
        }
        Ok(None) => {
            // liblsl reports a timeout by returning zero with no error.
            if !ec.is_null() {
                *ec = LSL_NO_ERROR;
            }
            0.0
        }
        Err(_) => {
            if !ec.is_null() {
                *ec = LSL_LOST_ERROR;
            }
            0.0
        }
    }
}

macro_rules! pull_sample {
    ($name:ident, $ty:ty, $pat:pat => $expr:expr) => {
        /// Read one sample.
        ///
        /// # Safety
        /// The handle must have come from this library, and the buffer must
        /// hold `buffer_elements` values.
        #[no_mangle]
        pub unsafe extern "C" fn $name(
            inlet: lsl_inlet,
            buffer: *mut $ty,
            buffer_elements: c_int,
            timeout: c_double,
            ec: *mut i32,
        ) -> c_double {
            if buffer.is_null() {
                if !ec.is_null() {
                    *ec = LSL_ARGUMENT_ERROR;
                }
                return 0.0;
            }
            let cap = buffer_elements.max(0) as usize;
            pull_typed(
                inlet,
                timeout,
                ec,
                |k, v| {
                    if let $pat = v {
                        *buffer.add(k) = $expr;
                    }
                },
                cap,
            )
        }
    };
}

pull_sample!(lsl_pull_sample_f, f32, Value::F32(x) => *x);
pull_sample!(lsl_pull_sample_d, f64, Value::F64(x) => *x);
pull_sample!(lsl_pull_sample_i, i32, Value::I32(x) => *x);
pull_sample!(lsl_pull_sample_s, i16, Value::I16(x) => *x);
pull_sample!(lsl_pull_sample_c, c_char, Value::I8(x) => *x as c_char);
pull_sample!(lsl_pull_sample_l, i64, Value::I64(x) => *x);

/// Read one sample of strings. The caller frees each string.
///
/// # Safety
/// The handle must have come from this library, and the buffer must hold
/// `buffer_elements` pointers.
#[no_mangle]
pub unsafe extern "C" fn lsl_pull_sample_str(
    inlet: lsl_inlet,
    buffer: *mut *mut c_char,
    buffer_elements: c_int,
    timeout: c_double,
    ec: *mut i32,
) -> c_double {
    if buffer.is_null() {
        if !ec.is_null() {
            *ec = LSL_ARGUMENT_ERROR;
        }
        return 0.0;
    }
    let cap = buffer_elements.max(0) as usize;
    pull_typed(
        inlet,
        timeout,
        ec,
        |k, v| {
            if let Value::Str(b) = v {
                *buffer.add(k) = out_bytes(b);
            }
        },
        cap,
    )
}

/// Read as many whole samples as the buffers hold.
///
/// Returns the number of **values** written, which is the sample count times
/// the channel count. liblsl returns the same number.
unsafe fn pull_chunk_typed(
    inlet: lsl_inlet,
    data_elements: u64,
    stamp_elements: u64,
    stamps: *mut c_double,
    timeout: c_double,
    ec: *mut i32,
    mut store: impl FnMut(usize, &Value),
) -> u64 {
    let b = match (inlet as *mut InletBox).as_mut() {
        Some(b) => b,
        None => {
            if !ec.is_null() {
                *ec = LSL_ARGUMENT_ERROR;
            }
            return 0;
        }
    };
    let chans = b.info.channel_count as usize;
    // liblsl starts the reader on the first pull. `src/data_receiver.cpp:88`.
    b.ensure_open(timeout);
    let i = match b.inlet.as_mut() {
        Some(i) => i,
        None => {
            if !ec.is_null() {
                *ec = LSL_LOST_ERROR;
            }
            return 0;
        }
    };
    if chans == 0 {
        if !ec.is_null() {
            *ec = LSL_ARGUMENT_ERROR;
        }
        return 0;
    }
    let room_samples = (data_elements as usize / chans).min(stamp_elements as usize);
    let mut written = 0usize;
    // The first sample waits for the timeout. Every later one is taken only if
    // it is already there, so a chunk read never waits once per sample.
    let mut wait = Duration::from_secs_f64(timeout.max(0.0));
    while written < room_samples {
        match i.pull(wait) {
            Ok(Some(s)) => {
                for (k, v) in s.values.iter().enumerate().take(chans) {
                    store(written * chans + k, v);
                }
                if !stamps.is_null() {
                    *stamps.add(written) = s.timestamp;
                }
                written += 1;
                wait = Duration::from_millis(0);
            }
            Ok(None) => break,
            Err(_) => {
                if !ec.is_null() {
                    *ec = LSL_LOST_ERROR;
                }
                return (written * chans) as u64;
            }
        }
    }
    if !ec.is_null() {
        *ec = LSL_NO_ERROR;
    }
    (written * chans) as u64
}

macro_rules! pull_chunk {
    ($name:ident, $ty:ty, $pat:pat => $expr:expr) => {
        /// Read several samples.
        ///
        /// # Safety
        /// The handle must have come from this library, and both buffers must
        /// hold the number of elements that the arguments name.
        #[no_mangle]
        pub unsafe extern "C" fn $name(
            inlet: lsl_inlet,
            data: *mut $ty,
            stamps: *mut c_double,
            data_elements: u64,
            stamp_elements: u64,
            timeout: c_double,
            ec: *mut i32,
        ) -> u64 {
            if data.is_null() {
                if !ec.is_null() {
                    *ec = LSL_ARGUMENT_ERROR;
                }
                return 0;
            }
            pull_chunk_typed(
                inlet,
                data_elements,
                stamp_elements,
                stamps,
                timeout,
                ec,
                |k, v| {
                    if let $pat = v {
                        *data.add(k) = $expr;
                    }
                },
            )
        }
    };
}

pull_chunk!(lsl_pull_chunk_f, f32, Value::F32(x) => *x);
pull_chunk!(lsl_pull_chunk_d, f64, Value::F64(x) => *x);
pull_chunk!(lsl_pull_chunk_i, i32, Value::I32(x) => *x);
pull_chunk!(lsl_pull_chunk_s, i16, Value::I16(x) => *x);
pull_chunk!(lsl_pull_chunk_c, c_char, Value::I8(x) => *x as c_char);
pull_chunk!(lsl_pull_chunk_l, i64, Value::I64(x) => *x);
pull_chunk!(lsl_pull_chunk_str, *mut c_char, Value::Str(b) => out_bytes(b));

/// Read several samples of strings, each with its own length.
///
/// The library allocates each string, and the caller frees it with
/// `lsl_destroy_string`. The length buffer is the only way to learn how long a
/// value is, because a string can hold a zero byte.
///
/// # Safety
/// The handle must have come from this library. Both buffers must hold
/// `data_elements` entries, and the timestamp buffer must hold
/// `stamp_elements`.
#[no_mangle]
pub unsafe extern "C" fn lsl_pull_chunk_buf(
    inlet: lsl_inlet,
    data: *mut *mut c_char,
    lengths: *mut u32,
    stamps: *mut c_double,
    data_elements: u64,
    stamp_elements: u64,
    timeout: c_double,
    ec: *mut i32,
) -> u64 {
    if data.is_null() || lengths.is_null() {
        if !ec.is_null() {
            *ec = LSL_ARGUMENT_ERROR;
        }
        return 0;
    }
    pull_chunk_typed(
        inlet,
        data_elements,
        stamp_elements,
        stamps,
        timeout,
        ec,
        |k, v| {
            let bytes = match v {
                Value::Str(b) => b.clone(),
                other => format!("{other:?}").into_bytes(),
            };
            *data.add(k) = out_bytes(&bytes);
            *lengths.add(k) = bytes.len() as u32;
        },
    )
}

// ---------------------------------------------------------------------------
// configuration
// ---------------------------------------------------------------------------

/// Point the library at a configuration file.
///
/// The file is read at the first call that needs a value from it, and never
/// again (`src/api_config.cpp:326`). A call after that moment has no effect.
/// The `LSLAPICFG` environment variable wins over this name
/// (`src/api_config.cpp:84`).
///
/// # Safety
/// The string must be zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_set_config_filename(name: *const c_char) {
    labstream_net::config::set_filename(&cstr(name));
}

/// Give the library a configuration as text.
///
/// The text wins over every file. liblsl uses it once and then drops it
/// (`src/api_config.cpp:57-70`).
///
/// # Safety
/// The string must be zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_set_config_content(content: *const c_char) {
    labstream_net::config::set_content(&cstr(content));
}

// ---------------------------------------------------------------------------
// the rest of the outlet and inlet surface
// ---------------------------------------------------------------------------
//
// liblsl exports a plain form and a timestamp form of every push call. Both
// forward to the form that also carries the pushthrough flag, with a timestamp
// of 0.0 and pushthrough on (`src/lsl_outlet_c.cpp:114-120`, and the default
// arguments at `src/stream_outlet_impl.h:122`). A timestamp of 0.0 means the
// current clock. SPEC.md 8.3.
//
// `lsl_cpp.h` reaches these plain names directly, so a C++ application cannot
// load a library that leaves them out.

/// Build the plain form and the timestamp form of one push call.
macro_rules! forward_sample {
    ($plain:ident, $stamped:ident, $target:ident, $ty:ty) => {
        /// Send one sample, timestamped now.
        ///
        /// # Safety
        /// The handle must have come from this library, and the buffer must
        /// hold one value for each channel.
        #[no_mangle]
        pub unsafe extern "C" fn $plain(out: lsl_outlet, data: *const $ty) -> i32 {
            $target(out, data, 0.0, 1)
        }

        /// Send one sample with a timestamp.
        ///
        /// # Safety
        /// The handle must have come from this library, and the buffer must
        /// hold one value for each channel.
        #[no_mangle]
        pub unsafe extern "C" fn $stamped(
            out: lsl_outlet,
            data: *const $ty,
            timestamp: c_double,
        ) -> i32 {
            $target(out, data, timestamp, 1)
        }
    };
}

forward_sample!(
    lsl_push_sample_f,
    lsl_push_sample_ft,
    lsl_push_sample_ftp,
    f32
);
forward_sample!(
    lsl_push_sample_d,
    lsl_push_sample_dt,
    lsl_push_sample_dtp,
    f64
);
forward_sample!(
    lsl_push_sample_i,
    lsl_push_sample_it,
    lsl_push_sample_itp,
    i32
);
forward_sample!(
    lsl_push_sample_s,
    lsl_push_sample_st,
    lsl_push_sample_stp,
    i16
);
forward_sample!(
    lsl_push_sample_l,
    lsl_push_sample_lt,
    lsl_push_sample_ltp,
    i64
);
forward_sample!(
    lsl_push_sample_c,
    lsl_push_sample_ct,
    lsl_push_sample_ctp,
    c_char
);
forward_sample!(
    lsl_push_sample_str,
    lsl_push_sample_strt,
    lsl_push_sample_strtp,
    *const c_char
);

/// Build the plain, timestamp, and timestamp-list forms of one chunk call.
macro_rules! forward_chunk {
    ($plain:ident, $stamped:ident, $each:ident, $tp:ident, $tnp:ident, $ty:ty) => {
        /// Send several samples, timestamped now.
        ///
        /// # Safety
        /// The handle must have come from this library, and the buffer must
        /// hold `data_elements` values.
        #[no_mangle]
        pub unsafe extern "C" fn $plain(
            out: lsl_outlet,
            data: *const $ty,
            data_elements: u64,
        ) -> i32 {
            $tp(out, data, data_elements, 0.0, 1)
        }

        /// Send several samples that share one timestamp.
        ///
        /// # Safety
        /// The handle must have come from this library, and the buffer must
        /// hold `data_elements` values.
        #[no_mangle]
        pub unsafe extern "C" fn $stamped(
            out: lsl_outlet,
            data: *const $ty,
            data_elements: u64,
            timestamp: c_double,
        ) -> i32 {
            $tp(out, data, data_elements, timestamp, 1)
        }

        /// Send several samples, each with its own timestamp.
        ///
        /// # Safety
        /// The handle must have come from this library, the data buffer must
        /// hold `data_elements` values, and the timestamp buffer must hold one
        /// value for each sample.
        #[no_mangle]
        pub unsafe extern "C" fn $each(
            out: lsl_outlet,
            data: *const $ty,
            data_elements: u64,
            timestamps: *const c_double,
        ) -> i32 {
            $tnp(out, data, data_elements, timestamps, 1)
        }
    };
}

forward_chunk!(
    lsl_push_chunk_f,
    lsl_push_chunk_ft,
    lsl_push_chunk_ftn,
    lsl_push_chunk_ftp,
    lsl_push_chunk_ftnp,
    f32
);
forward_chunk!(
    lsl_push_chunk_d,
    lsl_push_chunk_dt,
    lsl_push_chunk_dtn,
    lsl_push_chunk_dtp,
    lsl_push_chunk_dtnp,
    f64
);
forward_chunk!(
    lsl_push_chunk_i,
    lsl_push_chunk_it,
    lsl_push_chunk_itn,
    lsl_push_chunk_itp,
    lsl_push_chunk_itnp,
    i32
);
forward_chunk!(
    lsl_push_chunk_s,
    lsl_push_chunk_st,
    lsl_push_chunk_stn,
    lsl_push_chunk_stp,
    lsl_push_chunk_stnp,
    i16
);
forward_chunk!(
    lsl_push_chunk_l,
    lsl_push_chunk_lt,
    lsl_push_chunk_ltn,
    lsl_push_chunk_ltp,
    lsl_push_chunk_ltnp,
    i64
);
forward_chunk!(
    lsl_push_chunk_c,
    lsl_push_chunk_ct,
    lsl_push_chunk_ctn,
    lsl_push_chunk_ctp,
    lsl_push_chunk_ctnp,
    c_char
);
forward_chunk!(
    lsl_push_chunk_str,
    lsl_push_chunk_strt,
    lsl_push_chunk_strtn,
    lsl_push_chunk_strtp,
    lsl_push_chunk_strtnp,
    *const c_char
);

// --- the untyped form --------------------------------------------------

/// Read one channel value out of a raw buffer.
///
/// `push_numeric_raw` copies `datasize()` bytes straight into the sample
/// (`src/stream_outlet_impl.cpp:171`). The buffer therefore holds the values
/// of one sample, in the byte order of this machine, with no padding.
///
/// # Safety
/// The buffer must hold one value of `format` at `index`.
unsafe fn raw_value(format: Format, data: *const u8, index: usize) -> Option<Value> {
    let width = format.width()?;
    let p = data.add(index * width);
    let mut buf = [0u8; 8];
    std::ptr::copy_nonoverlapping(p, buf.as_mut_ptr(), width);
    Some(match format {
        Format::Float32 => Value::F32(f32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]])),
        Format::Double64 => Value::F64(f64::from_ne_bytes(buf)),
        Format::Int32 => Value::I32(i32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]])),
        Format::Int16 => Value::I16(i16::from_ne_bytes([buf[0], buf[1]])),
        Format::Int8 => Value::I8(buf[0] as i8),
        Format::Int64 => Value::I64(i64::from_ne_bytes(buf)),
        Format::String | Format::Undefined => return None,
    })
}

/// Write one channel value into a raw buffer.
///
/// # Safety
/// The buffer must hold room for one value of the format at `index`.
unsafe fn write_raw_value(v: &Value, data: *mut u8, index: usize) -> usize {
    let (bytes, width): ([u8; 8], usize) = match v {
        Value::F32(x) => {
            let b = x.to_ne_bytes();
            ([b[0], b[1], b[2], b[3], 0, 0, 0, 0], 4)
        }
        Value::F64(x) => (x.to_ne_bytes(), 8),
        Value::I32(x) => {
            let b = x.to_ne_bytes();
            ([b[0], b[1], b[2], b[3], 0, 0, 0, 0], 4)
        }
        Value::I16(x) => {
            let b = x.to_ne_bytes();
            ([b[0], b[1], 0, 0, 0, 0, 0, 0], 2)
        }
        Value::I8(x) => ([*x as u8, 0, 0, 0, 0, 0, 0, 0], 1),
        Value::I64(x) => (x.to_ne_bytes(), 8),
        Value::Str(_) => return 0,
    };
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), data.add(index * width), width);
    width
}

/// Send one sample from a raw buffer.
///
/// # Safety
/// The handle must have come from this library, and the buffer must hold one
/// value of the stream format for each channel.
#[no_mangle]
pub unsafe extern "C" fn lsl_push_sample_vtp(
    out: lsl_outlet,
    data: *const c_void,
    timestamp: c_double,
    pushthrough: c_int,
) -> i32 {
    let b = match (out as *mut OutletBox).as_ref() {
        Some(b) => b,
        None => return LSL_ARGUMENT_ERROR,
    };
    if data.is_null() || b.info.format.width().is_none() {
        return LSL_ARGUMENT_ERROR;
    }
    let format = b.info.format;
    let raw = data as *const u8;
    push_typed(
        out,
        |i| raw_value(format, raw, i).unwrap_or(Value::I8(0)),
        timestamp,
        pushthrough,
    )
}

/// Send one sample from a raw buffer, timestamped now.
///
/// # Safety
/// The handle must have come from this library, and the buffer must hold one
/// value of the stream format for each channel.
#[no_mangle]
pub unsafe extern "C" fn lsl_push_sample_v(out: lsl_outlet, data: *const c_void) -> i32 {
    lsl_push_sample_vtp(out, data, 0.0, 1)
}

/// Send one sample from a raw buffer, with a timestamp.
///
/// # Safety
/// The handle must have come from this library, and the buffer must hold one
/// value of the stream format for each channel.
#[no_mangle]
pub unsafe extern "C" fn lsl_push_sample_vt(
    out: lsl_outlet,
    data: *const c_void,
    timestamp: c_double,
) -> i32 {
    lsl_push_sample_vtp(out, data, timestamp, 1)
}

/// Read one sample into a raw buffer.
///
/// # Safety
/// The handle must have come from this library, and the buffer must hold
/// `buffer_bytes` bytes.
#[no_mangle]
pub unsafe extern "C" fn lsl_pull_sample_v(
    inlet: lsl_inlet,
    buffer: *mut c_void,
    buffer_bytes: c_int,
    timeout: c_double,
    ec: *mut i32,
) -> c_double {
    let b = match (inlet as *mut InletBox).as_mut() {
        Some(b) => b,
        None => {
            if !ec.is_null() {
                *ec = LSL_ARGUMENT_ERROR;
            }
            return 0.0;
        }
    };
    let width = match b.info.format.width() {
        Some(w) => w,
        None => {
            if !ec.is_null() {
                *ec = LSL_ARGUMENT_ERROR;
            }
            return 0.0;
        }
    };
    let need = width * b.info.channel_count as usize;
    if buffer.is_null() || (buffer_bytes as usize) < need {
        if !ec.is_null() {
            *ec = LSL_ARGUMENT_ERROR;
        }
        return 0.0;
    }
    let raw = buffer as *mut u8;
    pull_typed(
        inlet,
        timeout,
        ec,
        |k, v| {
            write_raw_value(v, raw, k);
        },
        b.info.channel_count as usize,
    )
}

// --- the length-prefixed string form ------------------------------------

/// Send one sample of strings, each with its own length.
///
/// This is the only way to send a string that holds a zero byte.
///
/// # Safety
/// The handle must have come from this library, and both buffers must hold one
/// entry for each channel.
#[no_mangle]
pub unsafe extern "C" fn lsl_push_sample_buftp(
    out: lsl_outlet,
    data: *const *const c_char,
    lengths: *const u32,
    timestamp: c_double,
    pushthrough: c_int,
) -> i32 {
    if data.is_null() || lengths.is_null() {
        return LSL_ARGUMENT_ERROR;
    }
    push_typed(out, |i| buf_value(data, lengths, i), timestamp, pushthrough)
}

/// Read one string value out of a data buffer and a length buffer.
///
/// # Safety
/// Both buffers must hold an entry at `index`, and the data entry must hold
/// the number of bytes that the length entry names.
unsafe fn buf_value(data: *const *const c_char, lengths: *const u32, index: usize) -> Value {
    let p = *data.add(index);
    let n = *lengths.add(index) as usize;
    if p.is_null() || n == 0 {
        return Value::Str(Vec::new());
    }
    Value::Str(std::slice::from_raw_parts(p as *const u8, n).to_vec())
}

/// Send one sample of strings with lengths, timestamped now.
///
/// # Safety
/// The handle must have come from this library, and both buffers must hold one
/// entry for each channel.
#[no_mangle]
pub unsafe extern "C" fn lsl_push_sample_buf(
    out: lsl_outlet,
    data: *const *const c_char,
    lengths: *const u32,
) -> i32 {
    lsl_push_sample_buftp(out, data, lengths, 0.0, 1)
}

/// Send one sample of strings with lengths, with a timestamp.
///
/// # Safety
/// The handle must have come from this library, and both buffers must hold one
/// entry for each channel.
#[no_mangle]
pub unsafe extern "C" fn lsl_push_sample_buft(
    out: lsl_outlet,
    data: *const *const c_char,
    lengths: *const u32,
    timestamp: c_double,
) -> i32 {
    lsl_push_sample_buftp(out, data, lengths, timestamp, 1)
}

/// Send several samples of strings with lengths, sharing one timestamp.
///
/// # Safety
/// The handle must have come from this library, and both buffers must hold
/// `data_elements` entries.
#[no_mangle]
pub unsafe extern "C" fn lsl_push_chunk_buftp(
    out: lsl_outlet,
    data: *const *const c_char,
    lengths: *const u32,
    data_elements: u64,
    timestamp: c_double,
    pushthrough: c_int,
) -> i32 {
    if data.is_null() || lengths.is_null() {
        return LSL_ARGUMENT_ERROR;
    }
    push_chunk_typed(
        out,
        data_elements,
        |i| buf_value(data, lengths, i),
        timestamp,
        pushthrough,
    )
}

/// Send several samples of strings with lengths, each with its own timestamp.
///
/// # Safety
/// The handle must have come from this library, both buffers must hold
/// `data_elements` entries, and the timestamp buffer must hold one value for
/// each sample.
#[no_mangle]
pub unsafe extern "C" fn lsl_push_chunk_buftnp(
    out: lsl_outlet,
    data: *const *const c_char,
    lengths: *const u32,
    data_elements: u64,
    timestamps: *const c_double,
    pushthrough: c_int,
) -> i32 {
    if data.is_null() || lengths.is_null() {
        return LSL_ARGUMENT_ERROR;
    }
    push_chunk_stamped(
        out,
        data_elements,
        |i| buf_value(data, lengths, i),
        timestamps,
        pushthrough,
    )
}

/// Send several samples of strings with lengths, timestamped now.
///
/// # Safety
/// The handle must have come from this library, and both buffers must hold
/// `data_elements` entries.
#[no_mangle]
pub unsafe extern "C" fn lsl_push_chunk_buf(
    out: lsl_outlet,
    data: *const *const c_char,
    lengths: *const u32,
    data_elements: u64,
) -> i32 {
    lsl_push_chunk_buftp(out, data, lengths, data_elements, 0.0, 1)
}

/// Send several samples of strings with lengths, sharing one timestamp.
///
/// # Safety
/// The handle must have come from this library, and both buffers must hold
/// `data_elements` entries.
#[no_mangle]
pub unsafe extern "C" fn lsl_push_chunk_buft(
    out: lsl_outlet,
    data: *const *const c_char,
    lengths: *const u32,
    data_elements: u64,
    timestamp: c_double,
) -> i32 {
    lsl_push_chunk_buftp(out, data, lengths, data_elements, timestamp, 1)
}

/// Send several samples of strings with lengths, each with its own timestamp.
///
/// # Safety
/// The handle must have come from this library, both buffers must hold
/// `data_elements` entries, and the timestamp buffer must hold one value for
/// each sample.
#[no_mangle]
pub unsafe extern "C" fn lsl_push_chunk_buftn(
    out: lsl_outlet,
    data: *const *const c_char,
    lengths: *const u32,
    data_elements: u64,
    timestamps: *const c_double,
) -> i32 {
    lsl_push_chunk_buftnp(out, data, lengths, data_elements, timestamps, 1)
}

/// Read one sample of strings, with a length for each one.
///
/// The library allocates each string. The caller frees it with
/// `lsl_destroy_string`. liblsl copies the bytes with no zero terminator
/// (`src/lsl_inlet_c.cpp:157-160`), so the length buffer is the only way to
/// know how long a value is.
///
/// # Safety
/// The handle must have come from this library, and both buffers must hold
/// `buffer_elements` entries.
#[no_mangle]
pub unsafe extern "C" fn lsl_pull_sample_buf(
    inlet: lsl_inlet,
    buffer: *mut *mut c_char,
    buffer_lengths: *mut u32,
    buffer_elements: c_int,
    timeout: c_double,
    ec: *mut i32,
) -> c_double {
    if buffer.is_null() || buffer_lengths.is_null() {
        if !ec.is_null() {
            *ec = LSL_ARGUMENT_ERROR;
        }
        return 0.0;
    }
    let room = buffer_elements.max(0) as usize;
    pull_typed(
        inlet,
        timeout,
        ec,
        |k, v| {
            let bytes = match v {
                Value::Str(b) => b.clone(),
                other => format!("{other:?}").into_bytes(),
            };
            // liblsl copies the bytes with no terminator and reports the
            // length beside them. One is added here as well, so a caller that
            // treats the result as a C string still reads something valid.
            *buffer.add(k) = out_bytes(&bytes);
            *buffer_lengths.add(k) = bytes.len() as u32;
        },
        room,
    )
}

// --- the rest of the description and the inlet --------------------------

/// The text of the last error on this thread.
///
/// **liblsl always returns an empty string here.** `src/common.cpp:57` declares
/// a buffer of zeros inside the function and returns it, and nothing in the
/// library writes to that buffer. An implementation that returned a real
/// message would not match.
#[no_mangle]
pub extern "C" fn lsl_last_error() -> *const c_char {
    static_string("")
}

/// Build a description from a document.
///
/// A document that the reader refuses leaves a blank description whose name
/// holds the reason. SPEC.md 12.5.
///
/// # Safety
/// The string must be zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_streaminfo_from_xml(xml: *const c_char) -> lsl_streaminfo {
    let text = cstr(xml);
    match StreamInfo::from_fullinfo_xml(&text) {
        Ok(info) => InfoBox::new(info) as lsl_streaminfo,
        Err(reason) => {
            let mut blank = StreamInfo::new("", "", 0, Format::Undefined, 0.0);
            blank.hostname = String::new();
            blank.name = format!("(invalid: {reason})");
            InfoBox::new(blank) as lsl_streaminfo
        }
    }
}

/// Test a query against a description.
///
/// This covers the subset that a resolver builds, and no more. A query outside
/// the subset returns false. SPEC.md 2.4.
///
/// # Safety
/// The handle must have come from this library, and the string must be
/// zero-terminated.
#[no_mangle]
pub unsafe extern "C" fn lsl_stream_info_matches_query(
    info: lsl_streaminfo,
    query: *const c_char,
) -> i32 {
    match (info as *mut InfoBox).as_ref() {
        Some(b) => b.info.matches(&cstr(query)) as i32,
        None => 0,
    }
}

/// Give the description a new instance identifier and return it.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_reset_uid(info: lsl_streaminfo) -> *const c_char {
    match (info as *mut InfoBox).as_mut() {
        Some(b) => {
            b.info.uid = labstream_net::make_uid();
            b.sync();
            static_string(&b.info.uid)
        }
        None => static_string(""),
    }
}

/// Set the half-time of the smoothing filter, in seconds.
///
/// liblsl tests nothing here. `src/time_postprocessor.h:63` assigns the value
/// as it stands, and `src/lsl_inlet_c.cpp:298` always reports no error. A
/// value of zero or below is therefore accepted. Nothing on the wire carries
/// this number.
///
/// # Safety
/// The handle must have come from this library.
#[no_mangle]
pub unsafe extern "C" fn lsl_smoothing_halftime(inlet: lsl_inlet, value: f32) -> i32 {
    match (inlet as *mut InletBox).as_mut() {
        Some(b) => {
            b.halftime = value;
            LSL_NO_ERROR
        }
        None => LSL_ARGUMENT_ERROR,
    }
}

/// The clock offset, with the remote clock reading and the uncertainty.
///
/// # Safety
/// The handle must have come from this library. Each output pointer must be
/// null or hold room for one value.
#[no_mangle]
pub unsafe extern "C" fn lsl_time_correction_ex(
    inlet: lsl_inlet,
    remote_time: *mut c_double,
    uncertainty: *mut c_double,
    timeout: c_double,
    ec: *mut i32,
) -> c_double {
    let offset = lsl_time_correction(inlet, timeout, ec);
    if let Some(b) = (inlet as *mut InletBox).as_ref() {
        if let Some(i) = b.inlet.as_ref() {
            if !uncertainty.is_null() {
                *uncertainty = i.time_uncertainty().unwrap_or(0.0);
            }
            if !remote_time.is_null() {
                // The remote clock reading that belongs to this offset is the
                // local clock moved by it.
                *remote_time = labstream_net::clock() - offset;
            }
            return offset;
        }
    }
    if !uncertainty.is_null() {
        *uncertainty = 0.0;
    }
    if !remote_time.is_null() {
        *remote_time = 0.0;
    }
    offset
}
