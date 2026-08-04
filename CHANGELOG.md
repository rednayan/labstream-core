# Changelog

This project follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `.github/workflows/release.yml`. A push of a tag that starts with `v` checks
  the tag against the manifest and the changelog, runs the tests, publishes to
  crates.io, builds `labstream-capi` for Linux, macOS, and Windows, and writes
  the GitHub release with the three libraries attached.
- `publish = false` in `labstream-capi`. That crate builds a `cdylib` alone,
  which a Rust program cannot link, so no command can send it to crates.io.

### Changed

- `docs/versioning.md` gives the workflow, the one setting that crates.io needs
  for it, and the way to make a person approve each publish.

## [0.1.1] - 2026-08-04

The first release on crates.io. `labstream-core` is the crate to name, and it
carries the other four.

### Added

- `labstream-core`. This crate holds no code. It names `labstream-wire`,
  `labstream-proto`, `labstream-time`, and `labstream-net`, so a program takes
  one dependency and reaches every part through `labstream_core::wire`,
  `::proto`, `::time`, and `::net`. It does not name `labstream-capi`, which
  builds a shared library for a C program.
- The `net` feature of `labstream-core`, on by default. A program that opens no
  socket can turn it off and keep the three crates that hold no input and no
  output.

### Fixed

- `an_outlet_reports_a_port_for_each_protocol` asserted that the IPv4 data port
  and the IPv6 data port differ. That assertion was wrong, and it failed on
  macOS and on Windows. A measurement of liblsl on Windows read 16572 for both
  fields, so one number for both families is what liblsl gives. The library
  already matched. The assertion has gone, and the test keeps the check that
  each family reports a port.

### Changed

- `docs/conformance.md` records the Windows measurement of the two port
  fields. Windows now carries one measured field. macOS carries none.
- Each crate takes its dependency on another crate here from
  `[workspace.dependencies]`, which holds a version as well as a path.
  `cargo publish` refuses a path with no version.
- `docs/versioning.md` gives the publish order, and it says why
  `labstream-capi` is not in that order. That crate builds a `cdylib` alone,
  which a Rust program cannot link.

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

[Unreleased]: https://github.com/rednayan/labstream-core/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/rednayan/labstream-core/releases/tag/v0.1.1
[0.1.0]: https://github.com/rednayan/labstream-core/releases/tag/v0.1.0
