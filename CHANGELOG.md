# Changelog

This project follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-08-03

The first release. The library speaks protocol 1.10 of the Lab Streaming
Layer.

### Added

- `lsl-wire`: the sample codec for protocol 1.10. This crate holds no input
  and no output.
- `lsl-proto`: the handshake, the discovery messages, and the time sync. This
  crate holds no input and no output.
- `lsl-time`: the timestamp filter. The result matches the C++ filter bit for
  bit.
- `lsl-net`: sockets, outlet, inlet, resolver, configuration file, and XPath
  queries.
- `lsl-capi`: the C ABI. It exports 165 symbols and builds as `liblsl.so`.
- `SPEC.md`: the protocol document. Every claim cites the C++ source.
- `docs/conformance.md`: the conformance result and the method.

### Verified

- 165 of 165 C symbols export, and none is a stub.
- 21 of 21 example programs of liblsl link and agree.
- 42 of 42 XPath queries return what the C++ library returns.
- One recording of 7.81 hours held 8.4 million samples with no loss.
- 218 tests pass in this repository.

### Refused

- Protocol 1.00. That version carries every sample in a Boost archive. An
  inlet that asks for 1.00 gets a refusal, not a wrong read.

[Unreleased]: https://github.com/rednayan/lsl-rustlang/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/rednayan/lsl-rustlang/releases/tag/v0.1.0
