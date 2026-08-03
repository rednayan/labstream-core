# How to contribute

Thank you for your interest in this library. This page gives the rules that
keep the library correct.

## The one rule that matters most

The C++ library at `sccn/liblsl` is the specification. There is no written
standard for the LSL protocol. A claim about the protocol is correct only when
the C++ source supports it.

State a fact about LSL only after you read it in the C++ source. Cite the
source as `file:line`. Do not repeat a claim from documentation of another
project without a check against the source.

## Before you open a pull request

1. Run `cargo fmt --all`.
2. Run `cargo clippy --workspace --all-targets`. Correct every warning.
3. Run `cargo test --workspace`. All 218 tests must pass.
4. Run `cargo doc --workspace --no-deps`. Correct every warning.

If you change a protocol rule, read the next section first.

## A protocol change needs a conformance run

Unit tests in this repository come from a reading of the C++ source. A wrong
reading gives a wrong test and a wrong library at the same time. Both agree,
and both are wrong.

This is not a theory. Four defects reached this library that way. Each one
produced output that looked correct. Each one passed every test that existed.
The list is in `docs/conformance.md`.

A test in this repository cannot catch that class of error. Only a comparison
against real liblsl can. The conformance workbench is a separate repository
that holds the C++ library, the comparison tools, and the recorded
measurements.

If you change `lsl-wire`, `lsl-proto`, `lsl-time`, or the protocol parts of
`lsl-net`, ask for a conformance run in the pull request. A maintainer runs the
workbench against your branch and reports the result.

If you change documentation, an example, or an internal detail with no protocol
effect, a conformance run is not necessary.

## The layers

Each crate has one job. Keep the boundaries:

| Crate | Rule |
|---|---|
| `lsl-wire` | no input and no output. Bytes in, bytes out |
| `lsl-proto` | no input and no output. Bytes and events in, decisions out |
| `lsl-time` | no input and no output. Numbers in, numbers out |
| `lsl-net` | the only crate that opens a socket or reads a clock |
| `lsl-capi` | the only crate that holds a raw pointer |

Do not add a socket to `lsl-wire`, `lsl-proto`, or `lsl-time`. The split is
what makes a protocol rule testable with a byte slice.

`lsl-wire`, `lsl-proto`, and `lsl-time` set `#![forbid(unsafe_code)]`. Keep it.

## Floating point in `lsl-time`

The timestamp filter must give the same result as the C++ filter, bit for bit.
Every operation keeps the order of the C++ source. A different order gives a
different result in the last bits.

If you edit the filter, do not reorder an arithmetic expression. Do not replace
a sequence of operations with a mathematically equal one.

## Tests

| Test kind | Where | What it does |
|---|---|---|
| unit | `src/*.rs` | one rule, with a byte slice or a number |
| golden | `tests/golden.rs` | compares against recorded output of liblsl |
| mutant | `tests/mutants.rs` | makes sure that the golden data catches a defect |
| live | `crates/lsl-net/tests/*_live.rs` | binds a loopback socket |

A mutant test proves that the golden data has value. It changes the code on
purpose and makes sure that at least one recorded case fails.

If you add golden data, add a mutant test with it.

## Documentation style

Documents in this repository follow ASD-STE100 Simplified Technical English.
The rules that matter most:

1. Use only these modal verbs: can, will, must.
2. Write descriptive sentences of 25 words or fewer.
3. Write procedural sentences of 20 words or fewer.
4. Write one instruction per sentence.
5. If a sentence has a condition, put the condition first.
6. Use active voice.
7. Do not use semicolons. Write two sentences.

This applies to documents, code comments, commit messages, and error strings.
It does not apply to a comment in a pull request.

## Commit messages

Write the subject in the imperative. Keep it to 50 characters or fewer.

```
Correct the multicast port for a second outlet
```

If the change corrects a defect, name what the defect produced. A reader needs
to know whether the defect touched their recordings.
