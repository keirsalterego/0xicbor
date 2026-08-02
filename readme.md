<div align="center">

<img src="assets/banner.png" alt="0xicbor: intel/tinycbor ported from C to Rust" width="820">

### Concise Binary Object Representation (CBOR) Library

**A line-for-line port of [intel/tinycbor](https://github.com/intel/tinycbor) from C to Rust,
linkable as a drop-in `libtinycbor.a`.**

The original Qt test suite runs against it unmodified.

[Documentation](docs/src/index.md) ·
[Decisions](decisions.md) ·
[Benchmarks](bench/methodology.md)

</div>

---

## What this is

tinycbor is Intel's CBOR (RFC 8949) implementation: about 6,500 lines of C, aimed at
microcontrollers, with no allocations on the common path. This is that library rewritten in
Rust, keeping the C ABI byte for byte so existing callers do not notice the swap.

The proof is the test suite. `tests/original/` holds upstream's Qt tests copied verbatim,
with their SHA-256 hashes pinned at kickoff. They are compiled and linked against the Rust
static library with no edits. Whatever they report is what this port scores.

## Status

The numbers below are the real output of `make test` and `bench/run.py`, not targets.

| | |
|---|---|
| **Original suite** | **4,929 / 4,929, zero failures** |
| **Symbol parity** | **44 / 44, zero `nm` diff** against upstream's `libtinycbor.a` |
| **ABI layout** | asserted against C-dumped sizes and offsets, passing |
| **Differential fuzz** | **13.5M execs, zero divergences**, four targets, [every run logged](fuzz/history.tsv) |
| **Tools vs upstream** | **exact** on 4,509 documents x 20 flag combinations |
| **`unsafe` blocks** | **80**, all in `cbor-ffi`; `cbor-core` is `forbid(unsafe_code)` |
| **Dependencies** | **zero** |
| **Speed vs C** | **3.4x faster** (mean p50 over 16 throughput measurements) |

Two of those need a word about what a fresh clone reproduces. The fuzz total sums runs whose
logs were overwritten, so `fuzz/history.tsv` records each one against the commit it ran on.
The 4,509 documents were the accumulated fuzz corpus, which is generated and not committed,
so `make test` checks the nine that are. Both are spelled out on the
[scoreboard](docs/src/verification/scoreboard.md).

`tst_cpp` is the one exception, and always was: it `#include`s upstream's `.c` files
directly, so it tests C sources a Rust port does not have. That is the 2 rows between 4,929
and 4,931. It stays failing with [its own entry](decisions.md) rather than being quietly
dropped, which is the rule the whole scoreboard follows.

### Where it loses

Parsing is 2.7x faster on all eight corpus files, but it was **1.49x slower on all eight**
for a day, and the [methodology](bench/methodology.md) keeps that figure rather than erasing
it. Printing is 4.4x and most of that is upstream calling `vfprintf` once per byte, not
anything about decoding, so it is not the number to quote.

The C still starts a process faster, 1.17x at p50 and 3.15x at p99, from a binary 119x
smaller, and this port uses 11% to 66% more memory. That is the only one of the seventeen
measurements it loses, and it is here for the same reason the 1.49x is.

### What the checking found

Three separate things, none of them found by the original suite:

- **A bug in upstream, [filed](https://github.com/intel/tinycbor/issues/331).**
  `cbor_value_to_json_advance` is documented to reject text that is not valid UTF-8 and does
  not. This port reproduces it deliberately ([decision 17](decisions.md)): the claim is
  equivalence with a specific commit, and being bug-compatible is what makes the fuzzer mean
  anything.
- **Two bugs of ours, from the fuzzer.** A missing `CborInvalidType` arm in the pretty
  printer, found at 420,793 executions into a 15-minute run after two shorter runs came back
  clean. And `cbor_encode_simple_value` rejecting a value upstream encodes, found four
  seconds into a target that had never run before. Both are in
  [the fuzzing page](docs/src/verification/differential-fuzzing.md), with reproducers under
  `tests/port/corpus/`.
- **Four bugs in the tools**, which are a *second* parser and printer that nothing was
  checking, since the Qt suite tests the library and the fuzzers call the C ABI directly.
  That is [decision 18](decisions.md).

Sixty seconds is enough to claim you fuzzed. It is not enough to find anything.

`make lint` recomputes the `unsafe` count rather than trusting the number above, re-checks
the `forbid` attribute, and fails if a block has lost its `SAFETY:` line. That is how it came
out that seven of the eighty had never had one. For scale: uv ships 73 blocks, Bun ships
13,044.

## How it fits together

```
crates/cbor-core/   the actual port: no_std + alloc, #![forbid(unsafe_code)]
crates/cbor-ffi/    the C ABI shim: staticlib, #[repr(C)], all unsafe lives here
crates/cbor-ffi/include/   the C headers callers compile against
tools/              cbordump and json2cbor, pure Rust rewrites
tests/original/     upstream's suite, verbatim, hash-pinned, never edited
tests/port/         tests for what upstream's suite does not reach, each one
                    built against both archives and diffed rather than against
                    an expected-output file that could drift
fuzz/               differential targets against an out-of-process C oracle
bench/              methodology, results, and the upstream reference build
```

Two constraints hold it together. **Layout parity**, because `cbor.h` implements 59 of its
103 public functions as `static inline` accessors that read struct fields directly, and those
compile into the *caller*: get the layout wrong and the tests read garbage. The sizes and
offsets were dumped from a C program at kickoff into
[`abi-layout.txt`](crates/cbor-ffi/abi-layout.txt) and are asserted in a Rust test. And
**symbol parity** for the other 44, which are real exported symbols; `make symbols` diffs
`nm -g --defined-only` against upstream and wants an empty answer.
[More on both](docs/src/architecture/abi-shim.md).

## The C question

The shipped library contains no C. There is no `cc` crate, no `bindgen`, and nothing links
against upstream.

What *is* C, stated plainly:

- **The headers** in `crates/cbor-ffi/include/` are upstream's, vendored so a fresh clone
  builds without tinycbor next door. They are the ABI contract, and their `static inline`
  accessors compile into whatever program includes them, never into `libtinycbor.a`.
- **The fuzz oracle** is upstream built as a separate binary. The fuzzer talks to it over a
  pipe as a subprocess. Not FFI, not linked in.
- **`tests/original/`** is upstream's C++ test code, which is the entire point.
- **`bench/reference/libtinycbor-upstream.a`** is upstream compiled, committed so `make test`
  can build each differential test twice, once against each archive. It is the thing being
  compared against. Nothing shipped links against it ([decision 23](decisions.md)).

Longer version: [where the C ends](docs/src/architecture/the-c-question.md).

## Building

Needs a Rust toolchain, a C++ compiler, and Qt6 Test for the original suite.

```
make            # library + test binaries
make test       # the original suite, plus this port's own differential tests
make test-tools # cbordump and json2cbor against upstream's binaries
make symbols    # diff exported symbols against upstream
make lint       # clippy, the unsafe budget, and relative links
make fmt        # rustfmt check
make fuzz       # differential fuzz; DURATION=900 TARGET=encode_diff to pick
make bench      # the benchmark in bench/methodology.md
```

Everything above runs from a fresh clone with nothing else installed, `make fuzz` included:
the oracle falls back to the committed reference archive when there is no tinycbor checkout
to build against. The one exception is `make test-tools`, which compares against upstream's
`cbordump` and `json2cbor` binaries; those are not vendored, so without them it says so and
skips rather than fails. To confirm the tests really are untouched:

```
cd tests/original && sha256sum -c hashes.txt
```

[Build troubleshooting](docs/src/reference/troubleshooting.md) covers the rest.

## License

MIT, matching upstream. Files under `tests/original/` remain
Copyright (C) 2025 Intel Corporation under their original MIT license.
