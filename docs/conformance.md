# Conformance

This page gives what was measured, how it was measured, and what was not
measured.

## The oracle

There is no written specification of the LSL protocol. The C++ library is the
specification.

Every measurement on this page compares this library against `sccn/liblsl` at
commit `e651023c`. That build is the oracle. A result counts only when the two
libraries agree.

## The result

| Evidence | Measure |
|---|---|
| Tests in this repository | 226 |
| C symbols exported | 165 of 165, none a stub |
| Golden vectors, transcripts, time sequences | 42, 22, and 10 with 96,008 samples |
| Interop cells | 28 default, 16 blocking, 12 IPv6, and more |
| XPath queries answered as the oracle answers them | 42 of 42 |
| Example programs of liblsl that link and agree | 21 of 21 |
| Longest recording | 7.81 hours, 8.4 million samples, no loss |
| Memory after warm up | flat, below liblsl |
| Third-party applications | pylsl, LabRecorder, and Phasic, none of them changed |

In scope: 8,888 lines of liblsl, about 88 percent built.

Out of scope by choice: protocol 1.00 and the sync-mode byte swap, 966 lines.

## The methods

Each method below can fail. Each one was made to fail on purpose at least once.

| Method | What it compares |
|---|---|
| Golden vectors | the bytes of a sample, against recorded output of the oracle |
| Mutant tests | the golden data itself. A changed codec must fail at least one case |
| Transcript replay | a recorded handshake, message by message |
| Interop matrix | two live processes over loopback, in every direction |
| XPath battery | 42 queries, against both libraries |
| ABI tree walk | the description tree, call by call |
| Example programs | the 21 example programs of liblsl, unchanged |
| Recording | a real recorder on a second machine, read back from the file |
| Memory soak | what each process holds over 15 minutes |

The interop matrix runs this library against the oracle in both roles. The
oracle sends and this library receives. Then the roles change.

## What the measurements found

Four defects reached this library and passed every test that existed:

1. A pushed timestamp of zero.
2. A hardcoded loopback address.
3. A flag that was accepted and dropped. `lsl_create_outlet_ex` took the flags
   and ignored them.
4. A multicast port that only the first outlet held.

Each one produced output that looked correct. None of them was found by a test
written from the C++ source. Disagreement with the oracle found them, or real
software on a real network found them.

This is the finding that matters most for future work. A test in this
repository comes from a reading of the C++ source. A wrong reading gives a
wrong test and a wrong library together. Both agree, and both are wrong.

## The instrument can be wrong

Three times a failure was the measuring tool and not the library:

1. A configuration probe that raced its own consumer.
2. An XPath battery that read streams left over from an earlier run.
3. A memory soak that mistook a warm up for a leak.

Each one was a false accusation without a control. Every tool in the workbench
therefore carries a control that is known to pass. A tool that never failed is
not trusted.

## Protocol 1.00

This library refuses protocol 1.00. The refusal is a decision, not a gap.

Protocol 1.00 carries every sample in a Boost archive. No liblsl of the last
decade asks for it. A wrong read of that format gives wrong numbers with no
error, so the library refuses it instead.

The refusal is measured. An inlet that asks for 1.00 gets a clear refusal.

## Platforms

The workflow in `.github/workflows/ci.yml` builds and tests on three
platforms. The three results are not the same.

| Platform | Builds | Tests | Compared against liblsl |
|---|---|---|---|
| Linux | yes | 231 pass | the full workbench |
| macOS | yes | 1 open case | no |
| Windows | yes | 1 open case | one field |

Linux carries the full measurement. Windows carries one measurement of one
field, which the section below gives. macOS carries none, and runs the tests of
this repository and nothing more.

A test here comes from a reading of the C++ source, and this page gives four
defects that such a test did not catch. A fifth one follows, and a test caught
that one first.

### The port of each family. Answered

`an_outlet_reports_a_port_for_each_protocol` asserted that the IPv4 data port
and the IPv6 data port differ. The test passed on Linux and failed on macOS and
on Windows.

The assertion was wrong. A measurement on Windows read liblsl through pylsl,
which carries a build of liblsl for that platform. One outlet reported:

| Field | Value |
|---|---|
| `v4data_port` | 16572 |
| `v6data_port` | 16572 |

liblsl gives one number for both families there. This library gives the same
number, so it already matches. The assertion has gone, and the test keeps the
check that each family reports a port.

The mechanism is the default of the platform. liblsl opens one acceptor for
each family and scans the port range for each one
(`src/tcp_server.cpp:333-353`). Neither liblsl nor this library sets
`IPV6_V6ONLY`, so:

| Platform | Default | Result |
|---|---|---|
| Linux | dual stack | The IPv6 bind meets the IPv4 socket on that port, so the scan moves to the next one. The two ports differ. |
| macOS, Windows | IPv6 only | The two families do not meet, so both take the base port. |

Linux was the one platform measured when this test was written. The test
therefore held an accident of one platform as a rule of the protocol. This is
the fifth defect of that class on this page, and the first one that a test
caught before a recording did.

macOS holds no measurement of this field. Both platforms take the same default,
and the source above explains both, so the reason to expect a difference is
gone.

### The multicast port of a second outlet, on macOS

`both_outlets_hold_the_multicast_port` fails. A second outlet in one program
does not hold the multicast port. Another machine then sees one of the two
streams and not both.

`SO_REUSEADDR` lets two sockets share a wildcard port on Linux. BSD needs
`SO_REUSEPORT` for the same result. liblsl sets only `reuse_address`
(`src/udp_server.cpp:60`), and no measurement says what liblsl itself does on
macOS. Do not change the socket option before a measurement answers that
question. liblsl can hold the same difference, and then the answer is to match
it and not to correct it.

This question stays open. No maintainer has a Mac, so no measurement can
answer it now.

A person with a Mac can answer it. Build liblsl on that machine, and run one
program that opens two outlets. Then send a multicast query from a second
machine on the same network. Count the streams that answer:

- Two streams answer. liblsl holds the port for both outlets, and this library
  must do the same. `SO_REUSEPORT` is then the correction.
- One stream answers. liblsl holds the same difference, and this library
  already matches it. The test is then wrong, not the library.

Report the count in an issue. That number decides the change.

### Windows

`clock()` reads the performance counter, which is what MSVC gives
`steady_clock` and therefore what liblsl reads (`src/common.cpp:20`). The
arithmetic has its own tests, and every platform runs them.

`no_stage_leaves_a_timestamp_alone` passes in one run and fails in the next.
It waits five seconds for one sample over loopback. A run that fails reports no
sample, and not a wrong one, so the evidence points at the time limit and not
at the protocol.

One measurement covers Windows, and the section above gives it. It reads one
field of one outlet. Use Linux for a measured result.

## What is not measured

- Protocol 1.00.
- A second pair of machines. One pair was measured.
- Any network other than the one measured here.
- Windows and macOS at the protocol level. The interop matrix ran on Linux.
  The section above gives what each platform does.

Do not read a claim into this page that the table does not hold.

## The workbench

The tools that made these measurements live in a separate repository, the
conformance workbench. That repository holds the vendored C++ library, the
comparison tools, and the recorded output. It is not published yet.

The workbench stays separate for one reason. It needs a C++ toolchain, cmake,
Python, and a network. This repository needs none of them.

The source of this library cites workbench files as `captures/…`,
`artifacts/…`, and `oracle/…`. Those paths name files in the workbench. A
citation of the C++ source, such as `src/tcp_server.cpp:328`, names a file
below `liblsl/` in the workbench. That directory holds the pinned oracle.

If you change a protocol rule, ask for a conformance run in your pull request.
`CONTRIBUTING.md` explains when a run is necessary.
