# Where the C ends

"No C in the shipped artifact" is a claim, and claims about absence are the easiest ones to
get quietly wrong. This page states exactly what is and is not C, so it can be checked
rather than believed.

## The library contains no C

`libtinycbor.a` is compiled entirely from Rust. There is no [`cc`][cc] crate, no
[`bindgen`][bindgen], no `build.rs` invoking a C compiler, and nothing links against
upstream. The dependency graph of `cbor-ffi` is one entry long and it is `cbor-core`, which
itself has none.

```console
$ cargo tree
cbor-ffi v0.1.0
└── cbor-core v0.1.0
```

[cc]: https://crates.io/crates/cc
[bindgen]: https://crates.io/crates/bindgen

## There is no third-party CBOR crate either

Not `ciborium`, not `serde_cbor`, not `minicbor`, and not `half`. Wrapping an existing crate
is not a port. `cbor-core` is `#![no_std]` with `alloc`, and the IEEE 754 binary16
conversions are written out by hand — about thirty lines that would otherwise have been a
dependency.

## What *is* C, stated plainly

**The headers**, in `crates/cbor-ffi/include/`. These are upstream's `cbor.h`,
`cborjson.h`, `cborinternal_p.h`, `compilersupport_p.h` and two generated headers, vendored
so a fresh clone builds without tinycbor checked out beside it. They are the ABI contract.
The 59 `static inline` accessors in `cbor.h` compile into whatever program includes them —
the test binary — and never into the library. `cborinternal_p.h` is present only because
`tst_tojson.cpp` includes it for its own `encode_half`/`decode_half` reference, which
likewise runs in the test binary.

**The fuzz oracle.** Differential fuzzing needs upstream's implementation to compare
against. It is built as a standalone `cbor-oracle` binary and driven as a **subprocess** —
bytes in on stdin, pretty output and exit code out. An in-process oracle over FFI would have
been faster to write and would have made this whole page false.

**`tests/original/`.** Upstream's C++ test code, verbatim and hash-pinned. That is the point
of the exercise.

## Checking it yourself

```console
$ grep -rn "bindgen\|cc = \|build.rs" crates/       # nothing
$ find crates -name '*.c'                           # nothing
$ cargo tree                                        # two crates, no dependencies
```

The one C file that ever existed in this project was the throwaway program that dumped
[struct offsets](layout-parity.md) at kickoff. It was run outside the repository and only
its *output* was committed.
