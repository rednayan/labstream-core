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
| Tests in this repository | 218 |
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

## What is not measured

- Protocol 1.00.
- A second pair of machines. One pair was measured.
- Any network other than the one measured here.
- Windows and macOS at the protocol level. The library builds and the tests
  pass on both. The interop matrix ran on Linux.

Do not read a claim into this page that the table does not hold.

## The workbench

The tools that made these measurements live in a separate repository. That
repository holds the C++ library, the comparison tools, and the recorded
output.

The workbench stays separate for one reason. It needs a C++ toolchain, cmake,
Python, and a network. This repository needs none of them.

The source of this library cites workbench files as `captures/…`,
`artifacts/…`, and `oracle/…`. Those paths name files in the workbench.

If you change a protocol rule, ask for a conformance run in your pull request.
`CONTRIBUTING.md` explains when a run is necessary.
