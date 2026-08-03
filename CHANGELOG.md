# Changelog

This project follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Windows support in `labstream-net`. `clock()` reads the performance counter,
  which is the clock that MSVC gives `steady_clock` and that liblsl reads. The
  crate did not build on Windows before this.
- `counter_seconds` and five tests of it. Every platform runs the tests, so the
  arithmetic of the Windows clock has a check on Linux as well.
- `.gitattributes`. Git on Windows changed a line ending on checkout, and six
  tests that compare against recorded output of liblsl failed there.

### Changed

- `docs/conformance.md` gives the result for each platform. Version 0.1.0 said
  that the library builds and the tests pass on macOS and Windows. That
  statement was wrong.
- CI runs `cargo test --no-fail-fast`. One run now reports every failure.
- `libc` is a dependency of Unix alone.

### Known limitations

- macOS: a second outlet does not hold the multicast port. Another machine sees
  one of two streams from one program, and not both. `docs/conformance.md`
  gives what a correction needs.
- Windows: one live test passes in one run and fails in the next. It waits five
  seconds for a sample over loopback.
- No measurement compares this library against liblsl on macOS or Windows.

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

### Verified

- 165 of 165 C symbols export, and none is a stub.
- 21 of 21 example programs of liblsl link and agree.
- 42 of 42 XPath queries return what the C++ library returns.
- One recording of 7.81 hours held 8.4 million samples with no loss.
- 226 tests pass in this repository.

### Refused

- Protocol 1.00. That version carries every sample in a Boost archive. An
  inlet that asks for 1.00 gets a refusal, not a wrong read.

[Unreleased]: https://github.com/rednayan/labstream-core/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/rednayan/labstream-core/releases/tag/v0.1.0
