# labstream-core

A Lab Streaming Layer core library in Rust.

This library speaks the protocol of the C++ library at `sccn/liblsl`. An
unchanged application that was written for liblsl runs on it. pylsl,
LabRecorder, and a third-party recorder each do so.

## Status

This is version 0.1.0. The protocol work is complete and measured. These crates
can change before version 1.0. liblsl fixes the C ABI, so the C ABI will not
change.

| Measure | Result |
|---|---|
| C symbols exported | 165 of 165, none a stub |
| Example programs of liblsl that link and agree | 21 of 21 |
| XPath queries answered as the oracle answers them | 42 of 42 |
| Longest recording | 7.81 hours, 8.4 million samples, no loss |
| Tests in this repository | 226 |

A separate conformance workbench made each measurement against a pinned build
of liblsl at commit `e651023c`. `docs/conformance.md` gives the method and the
full result.

## The crates

| Crate | What it holds | Touches the operating system |
|---|---|---|
| `labstream-wire` | the sample codec | no |
| `labstream-proto` | the handshake, discovery, and time sync | no |
| `labstream-time` | the timestamp filter, bit exact | no |
| `labstream-net` | sockets, outlet, inlet, resolver, configuration, XPath | yes |
| `labstream-capi` | the C ABI, built as `liblsl.so` | yes |

Only `labstream-net` touches the operating system. A protocol rule therefore gets a
unit test with a byte slice. Only the tests of `labstream-net` need a network.

## Add the library to a Rust program

Most programs want [`labstream`](https://github.com/rednayan/labstream) and not
these crates. That crate is the API: it holds the block reads, the channel list,
the query builder, and the error type. It calls the crates here.

Use the crates here directly when a program needs a protocol detail that the API
does not give.

This library is not on crates.io yet. Add it from git:

```toml
[dependencies]
labstream-net = { git = "https://github.com/rednayan/labstream-core" }
labstream-wire = { git = "https://github.com/rednayan/labstream-core" }
```

### Send samples

```rust
use labstream_net::{clock, Outlet, StreamInfo};
use labstream_wire::{Format, Sample, Value};
use std::time::Duration;

fn main() -> std::io::Result<()> {
    let info = StreamInfo::new("BioSemi", "EEG", 8, Format::Float32, 100.0);
    let outlet = Outlet::new(info)?;

    for n in 0..1000 {
        let sample = Sample {
            timestamp: clock(),
            values: (0..8).map(|k| Value::F32((n + k) as f32)).collect(),
        };
        outlet.push(&sample);
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}
```

### Receive samples

```rust
use labstream_net::{resolve, Inlet};
use std::time::Duration;

fn main() -> std::io::Result<()> {
    let found = resolve("type='EEG'", 1, Duration::from_secs(5))?;
    let info = found.into_iter().next().expect("a stream");
    let mut inlet = Inlet::open(&info, Duration::from_secs(5))?;

    while let Some(sample) = inlet.pull(Duration::from_secs(1))? {
        println!("{} {:?}", sample.timestamp, sample.values);
    }
    Ok(())
}
```

A larger example is in `crates/labstream-net/examples/publish.rs`. It publishes a
signal that a recorder on another machine can find and read back.

```sh
cargo run --release -p labstream-net --example publish -- --name RustTest
```

## Use the library from C, C++, or Python

`labstream-capi` builds a shared library named `liblsl.so`. It exports the 165 C
symbols of liblsl.

```sh
cargo build -p labstream-capi --release
```

The result is at `target/release/liblsl.so`. An application finds it the way it
finds any other shared library. A C or C++ program links against it with no
change to its source.

For pylsl, name the file in the environment:

```sh
export PYLSL_LIB=/path/to/target/release/liblsl.so
```

## Build and test

```sh
cargo build --workspace
cargo test --workspace
```

The tests need no network hardware and no C++ toolchain. The tests of `labstream-net`
bind loopback sockets. All 218 tests run in about 10 seconds.

## Protocol 1.00

This library refuses protocol 1.00 on purpose. That version carries every
sample in a Boost archive. No liblsl of the last decade asks for it. An inlet
that asks for 1.00 gets a clear refusal, not a wrong read.

Protocol 1.10 is the version that every current liblsl uses.

## How to read the citations in the source

The source cites three kinds of evidence. Each kind has its own form:

- `src/tcp_server.cpp:328` names a file and line in the C++ liblsl at commit
  `e651023c`. This is the only specification of the protocol.
- `SPEC.md 6.4` names a section of the protocol document in this repository.
  Every claim in that document cites the C++ source.
- `captures/discover.json`, `artifacts/behavior-liblsl.json`, and
  `oracle/descxml.cpp` name files in the conformance workbench. That repository
  holds the measurements and the tools that made them.

The third kind of path does not exist in this repository. `docs/conformance.md`
explains where to find the workbench.

## Documentation

| Document | What it holds |
|---|---|
| `SPEC.md` | the protocol, with a citation for every claim |
| `docs/conformance.md` | what was measured, how, and the result |
| `CONTRIBUTING.md` | how to make a change and how to test it |
| `CHANGELOG.md` | what changed in each version |

For the API reference, build the documentation:

```sh
cargo doc --workspace --no-deps --open
```

## License

MIT. Read `LICENSE`.
