# Changelog

This project follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- `docs/conformance.md` gives a third open case.
  `an_outlet_reports_a_port_for_each_protocol` asserts that the IPv4 data port
  and the IPv6 data port differ. macOS and Windows can give one number for
  both, because `bind_udp_in_range` does not set `IPV6_V6ONLY` and those
  platforms default to one family for each socket. The page names the
  measurement that says whether the library or the test is wrong.

## [0.1.0] - 2026-08-03

The first release. The library speaks protocol 1.10 of the Lab Streaming
Layer.

### Added

- `labstream-wire`: the sample codec for protocol 1.10. This crate holds no input
  and no output.
- `labstream-proto`: the handshake, the discovery messages, and the time sync. This
  crate holds no input and no output.
- `labstream-time`: the timestamp filter. The result matches the C++ filter bit for
  bit.
- `labstream-net`: sockets, outlet, inlet, resolver, configuration file, and XPath
  queries.
- `labstream-capi`: the C ABI. It exports 165 symbols and builds as `liblsl.so`.
- `SPEC.md`: the protocol document. Every claim cites the C++ source.
- `docs/conformance.md`: the conformance result and the method.
- `docs/versioning.md`: what the version number covers, and how to release.

### Platforms

- Linux, macOS, and Windows each build the workspace.
- `clock()` reads the performance counter on Windows. That is the clock that
  MSVC gives `steady_clock`, and liblsl reads `steady_clock`
  (`src/common.cpp:20`). `counter_seconds` holds the arithmetic, and every
  platform runs the five tests of it.
- `.gitattributes` holds one line ending for the golden data. Git on Windows
  changed a line ending on checkout, and six tests that compare against
  recorded output of liblsl failed there.
- `libc` is a dependency of Unix alone.

### Verified

- 165 of 165 C symbols export, and none is a stub.
- 21 of 21 example programs of liblsl link and agree.
- 42 of 42 XPath queries return what the C++ library returns.
- One recording of 7.81 hours held 8.4 million samples with no loss.
- 231 tests pass on Linux.

### Known limitations

- Only Linux carries a measurement against liblsl. macOS and Windows run the
  tests of this repository and nothing more.
- macOS: a second outlet does not hold the multicast port. Another machine sees
  one of two streams from one program, and not both. `docs/conformance.md`
  gives the measurement that decides the correction.
- Windows: one live test passes in one run and fails in the next. It waits five
  seconds for a sample over loopback.

### Refused

- Protocol 1.00. That version carries every sample in a Boost archive. An
  inlet that asks for 1.00 gets a refusal, not a wrong read.

[Unreleased]: https://github.com/rednayan/labstream-core/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/rednayan/labstream-core/releases/tag/v0.1.0
