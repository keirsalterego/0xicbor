<div align="center">

<img src="assets/banner.png" alt="0xicbor — intel/tinycbor, C to Rust" width="820">

### Concise Binary Object Representation (CBOR) Library

**A line-for-line port of [intel/tinycbor](https://github.com/intel/tinycbor) from C to Rust,
linkable as a drop-in `libtinycbor.a`.**

The original Qt test suite runs against it unmodified.

[Documentation](https://keirsalterego.github.io/0xicbor/) ·
[Decisions](decisions.md)

</div>

---

## What this is

tinycbor is Intel's CBOR (RFC 8949) implementation: about 6,500 lines of C, aimed at
microcontrollers, with no allocations on the common path. This is that library rewritten in
Rust, keeping the C ABI byte for byte so existing callers do not notice the swap.

The proof is the test suite. `tests/original/` holds upstream's Qt tests copied verbatim,
with their SHA-256 hashes pinned at kickoff. They are compiled and linked against the Rust
static library with no edits. Whatever they report is what this port scores.

```
make        # builds libtinycbor.a and the original test binaries
make test   # runs them, prints per-binary pass/fail
```

## Status

This port is in progress. The numbers below are the real output of `make test`,
not a target.

| | |
|---|---|
| **Original suite** | **1,908 passed, 3,021 failed** of 4,929 reachable rows |
| **Symbol parity** | **44 / 44, zero `nm` diff** against upstream's `libtinycbor.a` |
| **ABI layout** | asserted against C-dumped sizes and offsets, passing |
| **`unsafe` blocks** | **46** |
| **Differential fuzz** | not yet run |

The 21 passing rows pass by coincidence: every entry point is currently a stub returning
`CborErrorInternalError`, and some tests expect an error. That is the honest floor. Each
module that lands moves the number, and the log of it lives in [decisions.md](decisions.md).

For scale on the `unsafe` count: uv ships 73 `unsafe` blocks, Bun ships 13,044. The budget
here is whatever `cbor-ffi` genuinely needs to dereference caller-supplied pointers, and it
gets published as it grows.

## How it fits together

```
crates/cbor-core/   no_std + alloc, #![forbid(unsafe_code)] — the actual port
crates/cbor-ffi/    staticlib, #[repr(C)] types — every unsafe block lives here
crates/cbor-ffi/include/   the C headers callers compile against
tools/              cbordump, json2cbor — pure Rust rewrites
tests/original/     upstream's suite, verbatim, hash-pinned, never edited
tests/port/         property tests written for this port
fuzz/               differential targets against an out-of-process C oracle
bench/              methodology, results, and the upstream reference build
```

Two constraints hold the design together.

**Layout parity.** `cbor.h` implements 59 of its 103 public functions as `static inline`
accessors that read struct fields directly. Those compile into the *caller*, not into the
library, so `CborValue`, `CborParser` and `CborEncoder` must match the C layout exactly or
the tests read garbage. The numbers were dumped from a C program at kickoff into
[`crates/cbor-ffi/abi-layout.txt`](crates/cbor-ffi/abi-layout.txt) and are asserted in a
Rust test.

**Symbol parity.** The remaining 44 functions are real exported symbols. `make symbols`
diffs `nm -g --defined-only` on this library against upstream's. The target is an empty
diff, and it currently is one.

## The C question

The shipped library contains no C. There is no `cc` crate, no `bindgen`, and nothing links
against upstream.

What *is* C, stated plainly:

- **The headers** in `crates/cbor-ffi/include/` are upstream's, vendored so a fresh clone
  builds without tinycbor checked out next door. They are the ABI contract. The `static
  inline` accessors in them compile into whatever program includes them — never into
  `libtinycbor.a`.
- **The fuzz oracle** is upstream's C library built as a separate binary. The differential
  fuzzer talks to it over a pipe as a subprocess. It is not FFI and it is not linked in.
- **`tests/original/`** is upstream's C++ test code, which is the entire point.

## Building

Needs a Rust toolchain, a C++ compiler, and Qt6 Test for the original suite.

```
make          # library + test binaries
make test     # run the original suite
make symbols  # diff exported symbols against upstream
make lint     # clippy, warnings denied
make fmt      # rustfmt check
```

To confirm the tests really are untouched:

```
cd tests/original && sha256sum -c hashes.txt
```

## License

MIT, matching upstream. Files under `tests/original/` remain
Copyright (C) 2025 Intel Corporation under their original MIT license.
