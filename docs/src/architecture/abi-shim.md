# Why an ABI shim

The requirement was to run the original test suite unmodified. That single constraint
decides the whole architecture, so it is worth walking through why.

## The problem

Upstream's tests are Qt/C++ files that call the C API directly:

```cpp
CborParser parser;
CborValue value;
cbor_parser_init(data, len, 0, &parser, &value);
QCOMPARE(cbor_value_get_type(&value), CborIntegerType);
```

There are three ways to make that compile and run against Rust, and two of them are wrong.

**Rewrite the tests to call a Rust API.** This is the tempting one, and it is the one that
makes the score meaningless. A test suite you rewrote is a test suite you can make pass.
Every disagreement between your port and upstream becomes a test edit rather than a bug.

**Wrap the Rust in a hand-written C translation layer.** Better, but it introduces a second
implementation between the tests and the port — one that can absorb bugs, paper over
mismatches, and quietly encode the behaviour you expected rather than the behaviour upstream
had.

**Hand the tests a `libtinycbor.a` they cannot distinguish from the real one.** This is what
0xicbor does. `cbor-ffi` is a `crate-type = ["staticlib"]` exporting the same symbols with
the same signatures against the same headers. The tests are compiled exactly as upstream
compiles them and linked against a library that happens to be Rust.

## What that costs

The port carries the shape of a C API at its boundary. `cbor_value_get_int_checked` returns
its result through an out-parameter because callers expect that:

```c
CborError cbor_value_get_int_checked(const CborValue *value, int *result);
```

Inside `cbor-core` the same operation is what you would write if nobody were watching:

```rust
pub fn get_int_checked(&self) -> CborResult<i32>
```

The out-parameter convention exists in exactly one place — the shim — and `cbor-core` never
sees an integer error code. That split is the recurring shape of this port: idiomatic Rust
in the middle, C conventions only at the edge, and one thin layer translating between them.

## What it buys

Two properties become mechanically checkable rather than matters of trust:

- [**Layout parity**](layout-parity.md) — the structs are byte-identical to C's, proven by
  a test against numbers dumped from a C program.
- [**Symbol parity**](symbol-parity.md) — `nm` on this library and on upstream's produce
  the same sorted list, and the diff is empty.

Neither of those is an argument. They are commands you can run.
