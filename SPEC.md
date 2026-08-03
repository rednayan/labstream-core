# The LSL wire protocol

**Status:** version 1. Released with labstream-core 0.1.0.
**Oracle:** `sccn/liblsl` at `e651023ca67996a05a028fd88a28603297120294`.
**Date:** 2026-08-02.

No LSL protocol specification exists. The C++ implementation is the
specification. This document reads that implementation and writes down what it
does.

Every claim cites a file and a line in the pinned oracle. A claim with no
citation is not a claim.

A citation of the form `src/tcp_server.cpp:328` names a file and a line in the
pinned C++ library. A citation of the form `captures/…`, `artifacts/…`, or
`oracle/…` names a file in the conformance workbench. The workbench is a
separate repository. Read `docs/conformance.md`.

## How to read this document

Each statement carries one of three labels.

| Label | Meaning |
|---|---|
| **Normative** | A different implementation must do this to interoperate. |
| **Observed** | liblsl does this. A different implementation can do something else. |
| **Captured** | A capture from a real outlet shows the claim on the wire. |

The difference between normative and observed matters. A suite that tests
observed behavior as if it were normative fails a correct implementation. Section
12 indexes every observed item.

---

## 1. Transports

LSL uses three channels. Each channel has its own socket and its own message
format.

| Channel | Transport | Direction | Purpose |
|---|---|---|---|
| Discovery | UDP multicast, broadcast, unicast | Inlet asks, outlet answers | Find streams that match a query |
| Time sync | UDP unicast | Inlet probes, outlet answers | Measure the offset between two clocks |
| Data | TCP | Outlet sends to inlet | Carry metadata and samples |

**Normative.** The data channel needs TCP (`src/data_receiver.cpp:310-325`).
Section 8 gives the reason. A
timestamp can depend on every byte before it. A lost datagram therefore corrupts
every later timestamp, and nothing detects the loss.

### 1.1 Ports

**Observed.** The base port is 16572 and the port range is 32
(`src/api_config.cpp:170-171`). An outlet binds the first free port in that
range. A resolver sends a unicast query to every port in the range.

**Normative.** A multicast or broadcast query goes to port 16571
(`src/api_config.cpp:169`, `ports.MulticastPort`). That port sits **outside**
the range that starts at the base port.

An outlet listens on both. It binds one port in the range for a unicast query,
and it binds the multicast port for a multicast or broadcast query
(`src/stream_outlet_impl.cpp:90-101`).

An implementation that listens only on the range never answers a multicast
query. M5 found this the hard way: the loopback interface carries no multicast,
so a resolver on this machine sends only to a real interface, and the outlet
stayed invisible.

**Observed.** The default multicast addresses are
`255.255.255.255`, `224.0.0.1`, `224.0.0.183`, and `239.255.172.215`
(`src/api_config.cpp:196-202`).

**Normative.** An outlet joins each group on each interface that carries
multicast (`src/udp_server.cpp`, the multicast constructor). A join that names
no interface lets the operating system pick one, and it can pick an interface
with no multicast. liblsl skips such an interface, and the log line
`netif 'lo' (status: 1, multicast: 0, broadcast: 0)` names one.

**Normative.** An outlet binds one stack for each protocol that the
configuration allows (`src/stream_outlet_impl.cpp:43-53`). Each stack has its
own TCP acceptor and its own unicast UDP server, and each binds its own port
out of the same range (`src/tcp_server.cpp:333-352`, `src/udp_server.cpp:20`).
The two families therefore hold **different** port numbers, and the description
reports all four.

**Normative.** `ports.IPv6` decides which stacks exist
(`src/api_config.cpp:172-185`). `disabled` leaves IPv4 alone, `forced` leaves
IPv6 alone, and anything else is an error that throws the whole file away.

**Normative.** Each scope adds an IPv6 group beside its IPv4 groups
(`src/api_config.cpp:191-247`). The suffix comes from
`multicast.IPv6MulticastGroup`, and the prefix from the scope: `FF02:` for a
link, `FF05:` for a site, `FF08:` for an organization, and `FF0E:` for the
globe.

**Normative.** An IPv6 join names an interface by its index, not by its
address (`src/udp_server.cpp:78`, which reads `scope_id`).

**Normative.** A resolver runs one attempt for each protocol. Its receiving
socket has one protocol, and it sends only to the groups of that protocol
(`src/resolve_attempt_udp.cpp:174`). The query carries the port that the answer
returns to, and that port belongs to one socket.

**Normative.** An inlet takes the IPv4 endpoint when the description holds one,
and the IPv6 endpoint when it does not
(`src/inlet_connection.cpp:115-125`, `set_protocols`).

**A link-local address needs its interface number.** The address that a
resolver stores is the text that asio writes. For a link-local address that
text ends with `%` and the number of the interface. A connection to the address
without that number fails. This is not stated in liblsl. It was measured: an
inlet of this implementation failed with `Invalid argument` against a real
liblsl outlet until the number was kept.

**Normative.** A UDP server sets `reuse_address` before it binds
(`src/udp_server.cpp:60`). Every outlet therefore holds the multicast port at
the same time, inside one process and across processes.

**Why that matters.** A program that publishes a data stream and a marker
stream builds two outlets. Without the option only the first one binds the
port. The second answers a unicast query and a broadcast query, and never a
multicast query. On one machine every test still passes, because a unicast
query walks the whole port range. Another machine sends a multicast query and
finds one of the two streams.

---

## 2. Discovery

### 2.1 The query

**Normative.** A resolver sends this datagram
(`src/resolve_attempt_udp.cpp:52-57`):

```
LSL:shortinfo\r\n
<query>\r\n
<return_port> <query_id>\r\n
```

The `return_port` is a port that the resolver opened. The outlet answers to that
port at the source address of the datagram.

### 2.2 The answer

**Normative.** An outlet that matches the query answers with the query
identifier, a line break, and the short XML description
(`src/udp_server.cpp`, `process_shortinfo_request`):

```
<query_id>\r\n<shortinfo_xml>
```

**Normative.** If the query does not match, the outlet stays silent
(`src/udp_server.cpp`, `process_shortinfo_request`). It sends no negative
answer.

### 2.3 The query identifier is opaque

**Observed.** liblsl builds the identifier with
`std::to_string(std::hash<std::string>()(query))`
(`src/resolve_attempt_udp.cpp:51`).

**Normative.** The identifier is an opaque round-trip token. The outlet echoes
it (`src/udp_server.cpp`, `process_shortinfo_request`). The resolver compares it
with what it sent (`src/resolve_attempt_udp.cpp:114`).

`std::hash` returns a different value in libstdc++, libc++, and MSVC. An
implementation must never derive this value and must never check it against a
value from another implementation.

### 2.4 The query language

**Normative.** A query is XPath 1.0. liblsl builds it with pugixml and asks for
a boolean (`src/stream_info_impl.cpp:214`). The context node is the `<info>`
element of the stream.

**Normative.** An empty query matches every stream
(`src/stream_info_impl.cpp:198`).

**Normative.** A query that pugixml refuses matches nothing. liblsl catches the
error, writes a warning, and reports no match
(`src/stream_info_impl.cpp:239-241`). **A stream that refused the query and a
stream that failed to parse it look the same from outside.**

**Normative.** `resolver_impl::build_query` only ever writes
`session_id='X' and prop='Y'` (`src/resolver_impl.cpp:66-73`). Every query that
liblsl builds for itself has that shape. An application that calls
`lsl_resolve_bypred` supplies its own expression, and liblsl joins it to the
session test with `and` (`src/resolver_impl.cpp:70`).

**Captured.** `oracle/xpath.py` sends 42 queries to a real outlet.
`artifacts/xpath-liblsl.json` holds the answers. Every shape below is answered:

| Group | What works |
|---|---|
| boolean | `and`, `or`, `not()`, parentheses, `true()`, `false()` |
| comparison | `=`, `!=`, `<`, `>`, `<=`, `>=` |
| string | `contains`, `starts-with`, `substring`, `string-length`, `translate`, `concat`, `normalize-space` |
| number | `channel_count>4`, `nominal_srate=100`, `number(x)+1=9`, `floor(x div 3)` |
| path | `//name='X'`, `count(//name)=1`, a bare `name`, `//v4data_port>0` |
| malformed | five shapes, all refused |

**A path is read from the `<info>` element, not from the document above it.**
`info/name='XpTest'` matches nothing, because it looks for an `<info>` inside
`<info>`. Measured, same file.

**A comparison against a field is a comparison against a set of nodes.** XPath
makes it true when **any** node of the set satisfies it. That matters for a
query over `<desc>`, where one name can appear many times.


---

## 3. Time synchronization

### 3.0 The clock

**Normative.** `lsl_local_clock` returns
`steady_clock::now().time_since_epoch()` in seconds (`src/common.cpp:20`). On
Linux that clock is `CLOCK_MONOTONIC`, and its origin is the moment the machine
started.

**Normative.** Two processes on one machine must read the same value. An
application can stamp a sample with this clock, and a consumer on the same
machine reads that stamp directly. No post-processing runs by default
(`include/lsl/common.h:103`), so nothing corrects a difference in the origin.

**Captured.** An implementation that measures from the start of its own process
appears to work. Every timestamp is then wrong by the age of the machine, and a
live measurement against a real outlet reported an offset of 31,426 seconds.
After the correction the same measurement reported 15 microseconds.

**Observed.** `/proc/uptime` is not a source for this value. It counts time
spent suspended, and on one machine it read 66,567 where `CLOCK_MONOTONIC` read
31,492.

### 3.1 The probe

**Normative.** The inlet sends this datagram to the service port
(`src/time_receiver.cpp:136`):

```
LSL:timedata\r\n<wave_id> <t0>\r\n
```

### 3.2 The answer

**Normative.** The outlet answers with a leading space, then four values
(`src/udp_server.cpp`, `process_timedata_request`):

```
 <wave_id> <t0> <t1> <t2>
```

The outlet writes the values with 16 digits of precision. `t1` is the outlet
clock when the request arrived. `t2` is the outlet clock at the moment of the
answer.

**Captured.** A capture returns ` 12345 7958.920835802 7958.921048403
7958.921142721`. The full capture is in `captures/timeprobe.json`.

### 3.3 The estimate

**Normative.** The inlet reads `t3` from its own clock on arrival. It then
computes two values (`src/time_receiver.cpp:170-178`):

```
rtt    = (t3 - t0) - (t2 - t1)
offset = ((t1 - t0) + (t2 - t3)) / 2
```

**Observed.** liblsl sends a burst of probes. It keeps the estimate with the
lowest round-trip time and discards the others
(`src/time_receiver.cpp:190-205`). NTP uses the same rule.

**Observed.** The stored offset is the negative of the computed offset
(`src/time_receiver.cpp:204`).

### 3.4 The wave identifier

**Normative.** The inlet ignores any answer whose wave identifier does not match
the current burst (`src/time_receiver.cpp:166`). This rejects a late answer from
an earlier burst.

**Observed.** liblsl draws the identifier with `std::rand()`
(`src/time_receiver.cpp:116`).

---

## 4. The data channel

### 4.1 Request lines

**Normative.** A client opens a TCP connection and sends one of four request
lines (`src/tcp_server.cpp:502-526`):

| Request line | Purpose |
|---|---|
| `LSL:shortinfo\r\n<query>\r\n` | Discovery answer over TCP |
| `LSL:fullinfo\r\n` | Complete metadata as XML |
| `LSL:streamfeed\r\n` | Sample feed, protocol 1.00 |
| `LSL:streamfeed/<version> <uid>\r\n` | Sample feed, protocol 1.10 and later |

**Observed.** The version constant `LSL_PROTOCOL_VERSION` is 110
(`src/common.h:46`).

### 4.2 Request headers

**Normative.** After a `LSL:streamfeed/<version>` request line, the client sends
header lines. A blank line ends the block (`src/data_receiver.cpp:170-190`).

```
LSL:streamfeed/110 <uid>\r\n
Native-Byte-Order: 1234\r\n
Endian-Performance: 0\r\n
Has-IEEE754-Floats: 1\r\n
Supports-Subnormals: 1\r\n
Value-Size: 4\r\n
Data-Protocol-Version: 110\r\n
Max-Buffer-Length: 360\r\n
Max-Chunk-Length: 0\r\n
Hostname: <host>\r\n
Source-Id: <id>\r\n
Session-Id: default\r\n
\r\n
```

### 4.3 Header parsing rules

Both sides parse headers with the same code
(`src/tcp_server.cpp:602-632`, `src/data_receiver.cpp:215-258`).

**Normative.** A line with no colon is ignored (`src/tcp_server.cpp:604`).

**Normative.** A semicolon starts a comment. The parser removes the semicolon
and everything after it (`src/tcp_server.cpp:610`, `src/data_receiver.cpp:220`).

**Normative.** The parser converts the whole line to lowercase, so header names
are case-insensitive (`src/tcp_server.cpp:613`).

**Normative.** The parser trims the name and the value
(`src/tcp_server.cpp:615-616`).

**Observed.** The line buffer holds 16384 bytes (`src/tcp_server.cpp:601`).

**Observed.** The parser finds the colon before it removes the comment, and it
then uses the old position. If a semicolon comes before the colon, the value
lookup reads past the end of the shortened line. No test covers this input.

### 4.4 The server reads a header that no client sends

**Captured.** This is a real defect in the protocol, and it changes what a
conformant server must do.

The client sends `Data-Protocol-Version` (`src/data_receiver.cpp:183`). The
server compares the name against `protocol-version`
(`src/tcp_server.cpp:628`). The two names differ, so the server never reads the
header that the client sends.

A live experiment against the pinned oracle confirms this:

| Client sends | Server answers with |
|---|---|
| `Data-Protocol-Version: 100` | `Data-Protocol-Version: 110` |
| `Protocol-Version: 100` | `Data-Protocol-Version: 100` |
| Neither header | `Data-Protocol-Version: 110` |

**Normative.** A server must read `Protocol-Version` from the request. A server
that reads `Data-Protocol-Version` instead negotiates a different version than
liblsl does.

**Normative.** A server must write `Data-Protocol-Version` in the answer. The
client reads that name (`src/data_receiver.cpp:244`).

The defect is harmless in liblsl. The default value of the client version is the version from the request line
(`src/tcp_server.cpp:596-597`). The client sends the same number in both places.

### 4.5 Status line and status codes

**Captured.** The answer starts with a status line
(`src/tcp_server.cpp:673`). A capture returns `LSL/110 200 OK`.

```
LSL/<version> <status_code> <message>\r\n
```

| Code | Sent when | Source |
|---|---|---|
| 200 | The request is accepted | `src/tcp_server.cpp:673` |
| 404 | The request holds a UID that does not match this stream | `src/tcp_server.cpp:586` |
| 505 | The client major version is newer than the server major version | `src/tcp_server.cpp:578` |

**Normative.** The client rejects a malformed status line. The line needs three
parts, and the first part starts with `LSL/`
(`src/data_receiver.cpp:197-199`).

**Normative.** The client treats each range differently
(`src/data_receiver.cpp:207-212`).

| Range | Client action |
|---|---|
| 404 | Treat the stream as lost, then resolve again |
| 400 and above | Raise an error |
| 300 to 399 | Treat as a redirect, and treat the stream as lost |

### 4.6 Answer headers

**Captured.** The server answers with four headers
(`src/tcp_server.cpp:673-678`). A capture returns this block.

```
LSL/110 200 OK
UID: 0b4645ec-dd4b-4582-a256-36119e117403
Byte-Order: 1234
Suppress-Subnormals: 0
Data-Protocol-Version: 110
```

**Normative.** If the UID does not match the connection UID, the client treats
the stream as lost (`src/data_receiver.cpp:242`). A restarted source carries a
new UID.

**Normative.** A `Byte-Order` value of 0 means the native order of the reader.
This case exists for liblsl 1.13 and earlier
(`src/data_receiver.cpp:231`).

**Normative.** If the answer names a protocol version above the client maximum,
the client raises an error (`src/data_receiver.cpp:246-251`).

---

## 5. Negotiation

The server decides three things: the data protocol version, the byte order, and
subnormal suppression.

### 5.1 Version

**Normative.** The server starts from the lower of the two versions
(`src/tcp_server.cpp:641`):

```
data_protocol_version = min(server_configured_version, client_protocol_version)
```

**Normative.** Two conditions force a downgrade to protocol 1.00.

1. The format is not a string, and the client value size differs from the server
   channel size (`src/tcp_server.cpp:643-644`).
2. The server lacks IEEE-754 doubles, or the format is `float32` and the server
   lacks IEEE-754 floats, or the client reports no IEEE-754 floats
   (`src/tcp_server.cpp:645-649`).

**Normative.** If the client major version is newer than the server major
version, the server answers 505 and closes (`src/tcp_server.cpp:576-580`).

### 5.2 Byte order

**Normative.** The server converts the byte order only when all four conditions
hold (`src/tcp_server.cpp:652-664`):

1. The server byte order differs from `Native-Byte-Order`.
2. `can_convert_endian` accepts the client order and the client value size.
3. The client value size is more than 1.
4. The server measures its own conversion speed above `Endian-Performance`.

The fourth condition decides which side does the work. The faster side converts.

**Normative.** If all four conditions hold, the answer carries the client order
in `Byte-Order`, and the server writes swapped values. Otherwise the answer
carries the server order.

**Normative.** `can_convert_endian` accepts a value size of 1 without a test.
For any other size it needs the requested order to be `1234` or `4321`, and it
needs the local order to be one of the two (`src/util/endian.hpp:23-30`).

**Observed.** `Endian-Performance` is a measured benchmark result
(`src/data_receiver.cpp:174-175`). Its value differs between machines and
between runs. A conformance test must never compare it. A test can make sure that it is present. A test can also make sure that it
parses as a number.

### 5.3 Subnormal suppression

**Normative.** The server enables suppression when the format supports
subnormal values and the client reports no support
(`src/tcp_server.cpp:667-668`).

---

## 6. The test pattern handshake

**Captured.** This is the largest fact that a source read alone missed. The
protocol carries a built-in codec self-test.

### 6.1 What the server sends

**Normative.** After the answer headers, and before any real sample, the server
sends exactly two samples (`src/tcp_server.cpp:694-705`). It builds them with
`assign_test_pattern(4)` and then `assign_test_pattern(2)`.

### 6.2 What the client does

**Normative.** The client builds the same two samples locally. It then reads two
samples and compares each one for equality
(`src/data_receiver.cpp:276-297`).

On a mismatch the client raises this error
(`src/data_receiver.cpp:294`):

```
The received test-pattern samples do not match the specification. The protocol formats are likely incompatible.
```

### 6.3 The pattern

**Normative.** Every test pattern sample carries the timestamp `123456.789`
(`src/sample.cpp:366`).

**Normative.** The channel values follow this rule
(`src/sample.cpp:356-362`).

```
value[k] = (k + offset) * (k is even ? 1 : -1)
```

For an integer format the code takes the value modulo the type maximum before it
applies the sign.

**Normative.** The offset depends on the format
(`src/sample.cpp:368-400`). The value 4 or 2 from the caller adds to a per-format
base.

| Format | Base offset |
|---|---|
| `cft_float32` | 0 |
| `cft_double64` | 16777217 |
| `cft_int32` | 65537 |
| `cft_int16` | 257 |
| `cft_int8` | 1 |
| `cft_int64` | 2147483649 |

**Normative.** A string format ignores the offset. Channel `k` holds
`(k + 10) * (k is even ? 1 : -1)` as text (`src/sample.cpp:375-380`).

### 6.4 On the wire

A capture of a stream with 8 `float32` channels holds these first three samples.
The full capture is in `captures/feed.json`.

| Index | Timestamp | Channels |
|---|---|---|
| 0 | 123456.789 | 4, -5, 6, -7, 8, -9, 10, -11 |
| 1 | 123456.789 | 2, -3, 4, -5, 6, -7, 8, -9 |
| 2 | 7959.379452 | Real data |

### 6.5 Why this matters

The protocol tests the codec on every connection. An implementation that
encodes a sample wrongly is rejected at the handshake, and the error names the
cause.

This gives the rewrite a strong early signal. A Rust outlet that a real liblsl
inlet accepts has a correct codec for that format and that channel count.

---

## 7. Sample encoding

This section covers protocol 1.10 and later
(`src/sample.cpp:189-282`). Protocol 1.00 uses a different format. Section 11
covers it.

### 7.1 Layout

**Normative.** One sample holds a tag byte, an optional timestamp, and the
channel values.

```
u8  tag          1 = deduced timestamp, no timestamp bytes follow
                 2 = transmitted timestamp, an f64 follows
f64 timestamp    present only when tag = 2
... values       one value per channel
```

The tag constants are in `src/sample.h:19-20`.

**Normative.** A numeric value is swapped only when the negotiation selected the
opposite order and the value is wider than one byte
(`src/sample.cpp:219-226`, `src/sample.cpp:265-266`).

### 7.2 Channel formats

**Normative.** The format identifiers are in `include/lsl/common.h:64-84`.

| Format | Value | Width |
|---|---|---|
| `cft_undefined` | 0 | — |
| `cft_float32` | 1 | 4 |
| `cft_double64` | 2 | 8 |
| `cft_string` | 3 | variable |
| `cft_int32` | 4 | 4 |
| `cft_int16` | 5 | 2 |
| `cft_int8` | 6 | 1 |
| `cft_int64` | 7 | 8 |

### 7.3 Strings: the length prefix

This answers an open question from the conformance plan.

**Normative.** A string channel writes a width byte, then the length in that
width, then the bytes (`src/sample.cpp:198-214`).

The encoder picks the width by this rule:

| String length | Width byte | Length field |
|---|---|---|
| 0 to 255 | 1 | `u8` |
| 256 to 4294967295 | 4 | `u32` |
| Above that | 8 | `u64` |

**Normative.** The encoder never writes the width 2. The decoder accepts it
(`src/sample.cpp:251`).

This asymmetry is the trap. An implementation that writes a `u16` length for a string of 300 bytes makes a
stream that liblsl reads correctly. No liblsl outlet ever makes that stream.
An implementation that rejects width 2 on read fails against a peer that writes
it.

**Normative.** The decoder rejects any width other than 1, 2, 4, and 8
(`src/sample.cpp:256`). The error text is
`Stream contents corrupted (invalid varlen int).`

**Normative.** The length field itself is byte-swapped when the connection
swaps order (`src/sample.cpp:251-254`). The width byte is never swapped.

**Observed.** On a platform with a 32-bit `size_t`, a width of 8 raises
`This platform does not support strings of 64-bit length.`
(`src/sample.cpp:246-249`).

### 7.4 Subnormal suppression

**Normative.** When the answer sets `Suppress-Subnormals`, the reader clears
every subnormal value and keeps the sign bit
(`src/sample.cpp:267-280`).

For `cft_float32`, the reader tests `value & 0x7fffffff <= 0x007fffff` and then
applies `value &= 0x80000000`.

For `cft_double64`, the reader tests
`value & 0x7fffffffffffffff <= 0x000fffffffffffff` and then applies
`value &= 0x8000000000000000`.

**Normative.** The reader applies suppression after the byte order conversion
(`src/sample.cpp:265-267`).

---

## 8. Timestamps

### 8.1 Reconstruction

**Normative.** A deduced timestamp carries no bytes. The reader reconstructs it
(`src/data_receiver.cpp:310-325`):

```cpp
double last_timestamp = 0.0;          // once, before the loop
double srate = conn_.current_srate();
for (int k = 0; ...; k++) {
    ...
    if (samp->timestamp() == DEDUCED_TIMESTAMP) {
        samp->timestamp() = last_timestamp;
        if (srate != IRREGULAR_RATE) samp->timestamp() += 1.0 / srate;
    }
    last_timestamp = samp->timestamp();
}
```

**Normative.** The accumulator starts at 0.0 and lives outside the loop. A
deduced timestamp is therefore the previous timestamp plus one sample period.

**Normative.** If the rate is irregular, a deduced timestamp equals the previous
timestamp with no increment.

### 8.2 Why the data channel needs TCP

The accumulator runs for the lifetime of the connection. Every deduced timestamp
depends on every sample that came before it.

The 1.10 sample format carries no sequence number. A lost sample therefore
shifts every later timestamp by one sample period, and nothing detects the loss.

This is the reason the data channel cannot use an unreliable transport.

### 8.3 A pushed timestamp of zero means now

**Normative.** When an application pushes a sample with the timestamp `0.0`,
the outlet replaces it with the current clock reading
(`src/stream_outlet_impl.h:258`, `src/stream_outlet_impl.cpp:166`).

The value `0.0` therefore never reaches the wire from a push. An outlet API
that passes `0.0` through writes a timestamp that liblsl never writes.

**Observed.** M2 found this by accident. A golden vector used `0.0` as a fixed
test value, and the recorded stream carried a clock reading instead.


**Normative.** The rule lives in the outlet, not at the C boundary
(`src/stream_outlet_impl.cpp:170`). Every caller of the outlet gets it,
whatever language it uses.

**Why it has to be there.** A consumer reads a timestamp of zero as "no
sample". `lsl_pull_sample_f` returns 0.0, and pylsl turns that into
`(None, None)`. An outlet that sends a zero therefore delivers correct channel
values that no application can read or plot. A test that compares only the
values passes.

### 8.4 The first sample

**Observed.** If the first sample of a connection carries a deduced timestamp,
the reader returns `1.0 / srate`, because the accumulator starts at 0.0. Upstream report 161 describes a related effect.

### 8.5 Post-processing

**Normative.** The inlet applies up to three stages in a fixed order
(`src/time_postprocessor.cpp:60-105`): clock sync, then jitter removal, then
monotonic clamping.

**Observed.** Clock sync refreshes the offset every 50 samples, and never more
than twice per second (`src/time_postprocessor.cpp:64`,
`src/time_postprocessor.cpp:18`).

**Observed.** Jitter removal fits `t = w0 + w1 * n` with recursive least
squares. The forgetting factor is `2 ^ (-1 / (srate * halftime))`
(`src/time_postprocessor.cpp:108`). The filter subtracts an integer baseline for
numerical accuracy (`src/time_postprocessor.cpp:105`).

This filter is deterministic for a given input sequence. It can be tested
against golden values.

---

### 8.6 The buffer length carries two different units

**Normative.** The `Max-Buffer-Length` header carries a count of **samples**.

**Normative.** The public API does not. `lsl_create_inlet_ex` converts the
argument before anything else sees it (`src/lsl_inlet_c.cpp:20`), with the rule
in `src/stream_info_impl.cpp:254-270`:

| Condition | The buffer holds |
|---|---|
| The `transp_bufsize_samples` flag is set | The requested value, as samples |
| The stream carries no rate | The requested value multiplied by 100 |
| The stream carries a rate | The rate multiplied by the requested value, so the request is a count of seconds |

The `transp_bufsize_thousandths` flag then divides the result by 1000. The two
flags together raise an error. A result of zero or less becomes 1.

**Captured.** A stream with no rate and a request of 1 gives a buffer of 100
samples. A slow reader against a fast writer received sample 0, then samples
500 to 599 of 600. That is a ring of 100 that keeps the newest.

**Normative.** An implementation that treats the API argument as samples builds
a buffer that is wrong by a factor of the rate. At 1000 Hz a request of 20
means 20,000 samples and not 20.

### 8.8 Each consumer holds its own buffer

**Captured.** One outlet fed two inlets at once. The fast reader took all 400
samples with no gap. The slow reader, with a buffer of 100, took 121 samples
across 16 gaps.

**Normative.** A slow consumer never holds back a fast one. Every consumer
holds its own queue, and a full queue drops only that consumer's samples
(`src/send_buffer.h:67`, `src/tcp_server.cpp:746`).

### 8.7 A full buffer drops the oldest sample

**Captured.** The writer does not wait for a reader. In the capture above the
writer finished far sooner than the reader could consume, and no sample after
the first arrived until the writer had stopped.

**Normative.** A full buffer overwrites the oldest sample. The reader then sees
a contiguous late range with one gap.

**Normative.** Nothing reports the loss. No error reaches the reader, and no
count of dropped samples arrives with the data. A gap is visible only by
reading a value that the sender put in the sample.

The one exception is the `transp_sync_blocking` flag, which makes a push wait
for the consumer up to the configured send timeout
(`src/api_config.h:188-190`).

## 9. Chunking

This answers a second open question from the conformance plan.

**Normative.** A chunk is not a wire unit. The stream carries no chunk header,
no chunk length, and no delimiter. A reader cannot detect a chunk boundary.

**Observed.** A chunk decides when the server calls one socket write
(`src/tcp_server.cpp:782`). The server writes when a sample carries the
push-through flag, or when the count reaches the chunk size.

**Observed.** The server picks the chunk size in this order
(`src/tcp_server.cpp:747-751`):

1. `Max-Chunk-Length` from the request, when it is not zero.
2. The chunk size configured on the outlet, when it is not zero.
3. No limit.

**Observed.** If `Max-Buffer-Length` is zero or less, the server sends the
answer headers and the test pattern, and then it sends nothing more
(`src/tcp_server.cpp:730`). The comment calls this behavior convenient for unit
tests.

---

### 9.1 A chunk decides the size of one socket write

**Captured.** The same 120 samples, read from a raw socket, with three values
of `Max-Chunk-Length`:

| `Max-Chunk-Length` | Reads | Mean bytes per read |
|---|---|---|
| 0 | 1 | 165 |
| 1 | 121 | 18.2 |
| 16 | 8 | 258.6 |

One sample of two `float32` channels with a transmitted timestamp measures 17
bytes. A chunk length of 1 therefore writes one sample per call, and a length
of 16 writes about 16.

**Normative.** The push-through flag overrides the chunk length. A writer that
sets it forces a write on every sample, whatever the chunk length says
(`src/tcp_server.cpp:782`).

The first capture of this table set the flag on every sample, and all three
rows then looked the same. The flag wins.

**Normative.** A reader still cannot see a chunk. The bytes carry no header, no
length, and no delimiter. Only the size of a socket read changes, and a network
can split or join those freely.

### 9.2 A chunk with one timestamp is dated from its last sample

**Normative.** `push_chunk` with a single timestamp treats that value as the
time of the **last** sample, not the first
(`src/stream_outlet_impl.h:247-274`):

```cpp
if (timestamp == 0.0) timestamp = lsl_clock();
if (info().nominal_srate() != IRREGULAR_RATE)
    timestamp = timestamp - (num_samples - 1) / info().nominal_srate();
push_sample(buffer, timestamp, ...);
for (k = 1; k < num_samples; k++)
    push_sample(&buffer[k * num_chans], DEDUCED_TIMESTAMP, ...);
```

An application that acquires a block of samples and stamps it when the block
arrives therefore gets the right time for every sample in it.

**Normative.** A stream with no rate skips the subtraction, because there is no
period to count backward with.

**Normative.** Only the first sample carries a timestamp on the wire. Every
later sample of the chunk carries the deduced tag, and the reader rebuilds the
value from the rate. SPEC.md 8.1.

**Normative.** The push-through flag applies to the last sample of the chunk
only. The samples before it never force a write.

**Normative.** A buffer whose length is not a whole number of samples is an
error (`src/stream_outlet_impl.h:251`).

### 9.3 The blocking mode does not change the wire

**Normative.** `transp_sync_blocking` (`include/lsl/common.h:172`) removes the
queue between the caller and the socket. The outlet writes each push before it
returns.

**Normative.** The socket moves to the blocking writer **after** the status
line, the headers, and the two test pattern samples have gone out
(`src/tcp_server.cpp:733`). Everything before that point is the same exchange.

**Normative.** The layout of a sample is the same. A tag comes first, then a
timestamp when the tag calls for one, then the channel values
(`src/sync_serialization.h`, the note on the buffer sequence). A consumer
cannot tell the two modes apart.

**Normative.** The back-dating of a chunk happens before the two modes divide
(`src/stream_outlet_impl.h:259-261`), so section 9.2 holds for both.

**Normative.** A stream of strings cannot use the mode
(`src/stream_outlet_impl.cpp:34-38`). A string has no fixed width, and the mode
writes the memory of the caller directly.

**Measured.** `oracle/interop.py --sync` opens the outlet in the blocking mode.
Every cell passes, in both directions, for float32, double64, int32, and int16.

**What changes is the loss.** A queue drops its oldest sample when it is full
(section 8.7). A blocking outlet has no queue, so a consumer that reads slowly
holds up the caller instead.

## 10. Teardown

**Normative.** A UID mismatch in the answer makes the client treat the stream as
lost (`src/data_receiver.cpp:242`). The client then resolves again.

**Normative.** A status code of 404 has the same effect
(`src/data_receiver.cpp:208`).

### 10.1 Recovery after the source restarts

**Captured.** An inlet with recovery enabled reconnects on its own. An outlet
published 300 samples, stopped after 98 had arrived, and started again with the
same name and the same source identifier. The inlet received the remaining
samples with no call from the application.

| Measure | Value |
|---|---|
| Identifier before the restart | `a8cfdc43-15bf-4360-94a4-addbf45252f0` |
| Identifier after the restart | `02dbab71-852e-4594-9c93-b07312f6e5db` |
| Samples before the stop | 98 |
| Samples while the source was down | 98, so none arrived |
| Samples after the restart | 300 |

**Normative.** The instance identifier changes across a restart. The name and
the source identifier stay the same, and those two are what a recovery resolve
matches on.

**Normative.** A stream with an empty source identifier cannot be recovered.
liblsl warns at connection time (`src/inlet_connection.cpp:40`).

**Observed.** Recovery is not immediate. The watchdog checks every 15 seconds
by default, and it acts only after the same period without data
(`src/api_config.cpp:302-303`).

**Observed.** A connection-level error makes the client call
`try_recover_from_error` (`src/data_receiver.cpp:331-333`).

**Observed.** The outlet ignores an unknown UDP method and waits for the next
datagram (`src/udp_server.cpp`, `handle_receive_outcome`). It sends no error.

**Observed.** The outlet catches an exception during request processing, logs it
as a hiccup, and continues (`src/udp_server.cpp`, `handle_receive_outcome`).

---

## 11. Protocol 1.00

**Observed.** Protocol 1.00 does not use the layout in section 7. It uses
`eos::portable_archive`, a Boost serialization binary format
(`src/sample.cpp:330-350`).

**Normative.** In protocol 1.00 the server sends the short description as an
archive string before the test pattern (`src/tcp_server.cpp:685-689`). The
client reads it and compares the UID (`src/data_receiver.cpp:262-272`).

**Normative.** A 1.00 request line carries no version and no UID. The client
then sends one line with the buffer length and the chunk length
(`src/data_receiver.cpp:256-258`, `src/tcp_server.cpp:679-680`).

**Normative.** A server answers `505 Version not supported` only when the
**major** version differs (`src/tcp_server.cpp:576`). A request for 1.00 against
a 1.10 server is one that liblsl serves, so 505 is the wrong answer for it.

**Normative.** A server can agree to a version below the one that the request
named. `Data-Protocol-Version` in the answer carries the version that the data
will use, and a client that ignores it reads the bytes of an archive as 1.10
samples.

**Measured.** `oracle/refuse100.py` drives a peer that asks for 1.00.

| Pair | What happens |
|---|---|
| liblsl on both sides | the feed works, which shows the configuration took effect |
| our inlet, a 1.00 server | it stops in 0.5 s and names the version |
| a 1.00 inlet, our outlet | the outlet closes, and the peer stops on its own timeout |

The implementation plan leaves protocol 1.00 out of scope. The conformance suite
keeps it. Correct refusal is a conformance property, and the table above is the
measurement of it.

---

## 12. The description tree

An application attaches channel labels, units, and device names to a stream.
liblsl keeps them in one pugixml document, together with every field of section
2. `oracle/descxml.cpp` and `oracle/descread.cpp` build trees through real
liblsl and record what it writes. The files are in `captures/desc/`.

### 12.1 The short answer never carries the tree

**Normative.** `to_shortinfo_message` builds a new document and writes an empty
`<desc />` into it (`src/stream_info_impl.cpp:160-168`). A discovery answer
therefore holds no tree, even when the application added one.

**Normative.** `to_fullinfo_message` writes the live document, with the tree
(`src/stream_info_impl.cpp:178`).

### 12.2 The tree travels on the data port

**Normative.** A client opens its own TCP connection to the data port. It
writes `LSL:fullinfo\r\n` and reads to the end of the stream
(`src/info_receiver.cpp:60-67`). The server writes the document and closes the
connection. Nothing else marks the end (`src/tcp_server.cpp:508-513`).

**Normative.** The server builds both answers once, as it starts
(`src/tcp_server.cpp:377-378`). A change to the tree after the outlet exists
never reaches a consumer.

**Normative.** The same port answers `LSL:shortinfo`, with the query on the
next line. A stream that does not match the query gets no answer at all
(`src/tcp_server.cpp:537-550`).

### 12.3 The layout

**Captured.** `captures/desc/`. The document uses one tab for each level.

| Case | What liblsl writes | Capture |
|---|---|---|
| A tag with no children | `<x />` | `desc_empty` |
| A tag with one text child | `<x>v</x>`, on one line | `desc_flat` |
| A tag with an empty text child | `<x></x>` | `desc_blank` |
| Text beside a tag | `<v>a<b />c</v>`, on one line | `desc_reader` |
| Text | `&`, `<`, and `>` escaped, nothing else | `desc_escapes` |
| An attribute value | `&`, `<`, `"`, tab, and line feed escaped | `desc_reader` |
| A tab in an attribute | `&#09;`, with two digits | `desc_reader` |

One rule explains the third row and the fourth row. After liblsl writes text
inside a tag, it writes no more indents in that tag. The closing tag then sits
on the same line.

**Normative.** A double `f64` writes with 16 significant digits and keeps every
trailing zero (`src/util/cast.cpp:9`). A rate of 100 writes as
`100.0000000000000`, and a rate of 0 writes as `0.000000000000000`.

### 12.4 What a read drops

**Captured.** `captures/desc/desc_reader.txt`. liblsl loads the document with
the default parse flags and never changes them
(`src/stream_info_impl.cpp:186`).

| Input | What comes back |
|---|---|
| Text that holds only spaces | dropped, and the tag becomes `<x />` |
| A comment | dropped |
| An attribute | kept, and written back in double quotes |
| A section of character data | kept as a section of character data |
| `&amp;` with no semicolon | kept as it stands |
| `&#65;&#x42;` | `AB` |
| A carriage return | becomes a line feed |

**The last row is a loss.** `captures/desc/desc_reparse.hex` measures it: the
value `one\ntwo\rthree\tfour` returns as `one\ntwo\nthree\tfour`. A
description that carries a carriage return is not the same after it crosses the
wire.

**The first row is also a loss, and it matters more.** A sender writes
`<blank></blank>` for an empty value and `<hollow />` for a tag with no child.
A reader turns both into `<hollow />`. The two forms are apart in the sender
and together in the receiver.

### 12.5 A description that is rejected

**Normative.** `read_xml` throws, resets every field, and writes the reason
into the name as `(invalid: TEXT)` (`src/stream_info_impl.cpp:150-155`). `lsl_get_xml` then returns the blank document, whose `<name>` field is empty.
`lsl_get_name` returns the reason at the same moment.

**Captured.** `captures/desc/desc_nouid.name` holds
`(invalid: The UID of the given stream info is empty.)`. A `stream_info` that
no outlet published carries an empty `<uid>`, so `lsl_streaminfo_from_xml`
refuses its own output.

### 12.6 The tree calls of the C ABI

`src/lsl_xml_element_c.cpp` wraps pugixml. A reader of the header cannot guess
the behavior of three of its lines.

**Normative.** `lsl_append_child_value` returns the **parent**, not the new tag
(`src/lsl_xml_element_c.cpp:74-79`).

**Normative.** `lsl_value` returns the value of the node itself, which is empty
for a tag. The text sits in a child node, and `lsl_child_value` reads it
(`src/lsl_xml_element_c.cpp:38`).

**Normative.** `lsl_set_value` fails on a tag, because only a text node carries
a value (`src/lsl_xml_element_c.cpp:47`). `lsl_set_child_value` sets the value
of the **first** child, of any kind. If that child is a tag, the call fails
(`src/lsl_xml_element_c.cpp:69`).

**Normative.** `lsl_empty` asks whether the pointer names a node, and
`lsl_is_text` asks whether the type is not an element
(`src/lsl_xml_element_c.cpp:35-36`).

**Captured.** `oracle/desctree.c` drives all of this and prints 120 lines.
`oracle/abitree.sh` runs it against both libraries and compares.

### 12.7 The session identifier arrives at the outlet

**Normative.** The server sets the session identifier from the configuration as
it starts (`src/tcp_server.cpp:328`). A description that no outlet published
reports an empty value, not `default`.

### 12.8 A pull starts the reader

**Normative.** `try_get_next_sample` starts the reader thread if it is not
running (`src/data_receiver.cpp:88`). `lsl_open_stream` is therefore optional,
and a pull on a fresh inlet must report "no sample" after its timeout, not a
lost stream.

---

## 13. The configuration file

liblsl reads one file at the first call that needs a value from it
(`src/api_config.cpp:326`). It keeps the result for the life of the process and
never reads another. The file decides the ports, the multicast groups, the
session, and every tuning value. **A site that edits `lsl_api.cfg` edits the
wire.**

### 13.1 Where the library looks

**Normative.** `src/api_config.cpp:57-110`, in this order. The first readable
file wins, and no later one is read.

1. Text that `lsl_set_config_content` supplied. liblsl uses it once and drops
   it.
2. The name that `lsl_set_config_filename` supplied, when the file is
   readable.
3. The file that `LSLAPICFG` names. **This wins over the name in step 2**,
   because it is put in front of the list (`src/api_config.cpp:84`).
4. `lsl_api.cfg` in the working directory.
5. `~/lsl_api/lsl_api.cfg`.
6. `/etc/lsl_api/lsl_api.cfg`.

**Normative.** A file that does not parse leaves every default in place
(`src/api_config.cpp:105-107`).

### 13.2 How a line is read

**Normative.** `src/util/inireader.cpp:5-40`.

| Line | What happens |
|---|---|
| starts with `;` | a comment |
| blank | skipped |
| `[section]` | opens a section. The key is then `section.Key` |
| holds no `=` | **an error. The whole file is thrown away** |
| repeats a key | **an error. The whole file is thrown away** |
| `Key = Value` | the space around each side is dropped |

**A `#` does not start a comment.** The line then holds no `=`, the reader
throws, and every default applies. `oracle/configcheck.sh` measures this: a
file whose first line is `# this is not a comment` leaves `BasePort` at 16572
even though the file names 17400.

**Normative.** A yes or no value is true only for the exact text `1`.
`src/util/inireader.hpp` reads it with `from_string<bool>`, and
`src/util/cast.hpp:10` defines that as `str == "1"`.
**`ForceDefaultTimestamps = true` therefore means no.** Measured in
`oracle/configcheck.sh`.

**Normative.** A set has the form `{a, b, c}`. Text without both braces is an
empty set (`src/api_config.cpp:40-46`).

### 13.3 The keys that reach the wire

**Normative.** `src/api_config.cpp:168-322`.

| Key | Default | What it changes |
|---|---|---|
| `ports.MulticastPort` | 16571 | where a multicast query goes |
| `ports.BasePort` | 16572 | the first port of the unicast range |
| `ports.PortRange` | 32 | how many ports the range holds |
| `lab.SessionID` | `default` | the session field of every query |
| `lab.KnownPeers` | `{}` | addresses that a resolver always tries |
| `multicast.ResolveScope` | `site` | which groups a query reaches |
| `multicast.AddressesOverride` | `{}` | replaces the whole group list |
| `multicast.TTLOverride` | -1 | replaces the hop count |
| `tuning.WatchdogTimeThreshold` | 15.0 | how long a quiet source stays connected |
| `tuning.TimeUpdateInterval` | 2.0 | the gap between two clock bursts |
| `tuning.TimeUpdateMinProbes` | 6 | how many answers an estimate needs |
| `tuning.TimeProbeCount` | 8 | how many probes one burst sends |
| `tuning.TimeProbeInterval` | 0.064 | the gap between two probes |
| `tuning.TimeProbeMaxRTT` | 0.128 | how long a burst waits |
| `tuning.SmoothingHalftime` | 90.0 | the half-time of the smoothing filter |
| `tuning.ForceDefaultTimestamps` | off | replaces every pushed timestamp with the clock |
| `tuning.ContinuousResolveInterval` | 0.5 | the gap between two waves |
| `tuning.UseProtocolVersion` | 110 | the version an outlet writes, held at 110 or below |

**Normative.** The scope decides both the group list and the hop count
(`src/api_config.cpp:211-247`). `machine` reaches 127.0.0.1 with a hop count of
0. `link` adds the broadcast address, 224.0.0.1, and 224.0.0.183, with a hop
count of 1. `site` adds 239.255.172.215, with a hop count of 24.

**Normative.** `UseProtocolVersion` never rises above the version that the
library speaks (`src/api_config.cpp:296`).

---

## 14. Index of observed behavior

A conformance suite must not assert these values against another
implementation.

| Item | Why |
|---|---|
| `query_id` value | `std::hash` differs between standard libraries |
| `Endian-Performance` value | A measured benchmark result |
| `wave_id` value | Drawn with `std::rand()` |
| Stream UID value | A random identifier |
| `created_at` value | A clock reading |
| Ephemeral port numbers | Assigned by the operating system |
| Hostname | A property of the machine |
| Header line buffer size | An implementation limit |
| Query cache behavior | Not visible on the wire |

---

## 15. Open questions

These items stay open. Each one needs a source read, a capture, or both.

1. **String width 2 on read.** No liblsl outlet writes it. Confirm the decoder
   path with a hand-built stream.
2. **`Value-Size` for a string format.** The client sends 0
   (`src/data_receiver.cpp:181-182`). The downgrade rule skips the string case
   (`src/tcp_server.cpp:643`). Confirm the full truth table with an experiment.
3. ~~**`can_convert_endian`.**~~ **Closed in M3.** `src/util/endian.hpp:23-30`.
   A value size of 1 always passes. Any other size needs the requested order to
   be `1234` or `4321`, and the local order to be one of the two. Section 5.2
   carries the rule.
4. **The comment-before-colon input.** Section 4.3 describes a parser path with
   no test. M3 recorded a transcript with a trailing comment, which liblsl reads
   correctly. A comment that removes the colon stays untested, because the
   oracle reads past the end of its own buffer there. `labstream-proto` refuses that
   input instead.
5. **IPv6.** Every capture used IPv4. The short description carries `v6` fields.
6. ~~**`LSL:fullinfo`.**~~ **Closed.** The answer is the whole document, and the
   close of the connection ends it. Section 12 carries the rule and the
   captures.
7. ~~**Sync mode.**~~ **Closed.** The wire is the same. Section 9.3 carries the
   rule and the measurement.

---

## 16. What changed after M0

The M0 capture found three facts that a source read alone missed.

1. The answer carries a status line, `LSL/110 200 OK`. Section 4.5.
2. The byte order header is named `Byte-Order`. Section 4.6.
3. The feed starts with two test pattern samples. Section 6.

The third fact is the most important. An implementation that skips those samples
loses the whole stream. The reader then treats the first real sample as a test
pattern, and the comparison fails.

Read this next to section 6.5. The same mechanism that traps a wrong
implementation also tests a correct one on every connection.
