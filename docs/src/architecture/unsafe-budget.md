# The unsafe budget

`unsafe` is budgeted here, not sprinkled. The rule is structural rather than aspirational:

- **`cbor-core` is `#![forbid(unsafe_code)]`.** Not `deny`, not a lint you can allow at a
  call site — `forbid`, which cannot be overridden further down the tree. The entire CBOR
  implementation lives under it.
- **`cbor-ffi` is where every `unsafe` block lives**, and each one carries a `// SAFETY:`
  line naming the invariant the *caller* must uphold. This is the crate that dereferences
  pointers handed to it by a C program, so the invariants are real and worth writing down.

## Current count

```console
$ grep -rn "unsafe {" crates/ --include='*.rs' | wc -l
75
```

Counting `unsafe {` counts blocks, which is the number that means something. A bare
`grep -rn unsafe crates/` returns 93; the difference is 11 `unsafe fn` signatures and a
handful of lines of prose, including `cbor-core`'s `#![forbid(unsafe_code)]` itself.

All 75 are in `cbor-ffi`:

| file | blocks |
|---|---:|
| `parser.rs` | 37 |
| `encoder.rs` | 18 |
| `tojson.rs` | 11 |
| `pretty.rs` | 6 |
| `validation.rs` | 3 |
| `cbor-core/` (any file) | **0** |

`grep -rn SAFETY crates/` returns 87, so every block is accounted for with room to spare —
some invariants are stated once for a whole module and referred back to.

This started at zero when the shim was stubs and was published at each step rather than at
the end. A count that only ever moves in the flattering direction is not worth much if
nobody sees the intermediate values.

For scale: [uv][uv] ships 73 `unsafe` blocks. [Bun][bun] ships 13,044. Neither number is
damning on its own — a runtime that talks to JavaScriptCore has different obligations than a
package resolver — but they bracket what "a lot" and "a little" look like in shipped Rust.

[uv]: https://github.com/astral-sh/uv
[bun]: https://github.com/oven-sh/bun

## Where the blocks are

The prediction made at kickoff was "pointer validation at every entry point, plus
`_cbor_value_dup_string`, and nothing else if the design holds". That is what happened.

**Pointer validation at the boundary.** Each of the 44 exported functions receives raw
pointers from C. The shim converts them to references once, on entry — `as_ref` and `as_mut`
in the parser, their equivalents elsewhere — and everything past that point is safe Rust
operating on `&CborValue`. This is the bulk of the 75, and it is why `parser.rs` has the
most: it has the most entry points.

**Reading bytes out of the caller's buffer.** `be_load` does a sized unaligned load at a
cursor the caller owns. The bounds check happens first, in safe code, every time; the
`// SAFETY:` line on each call site names which check established it.

**`_cbor_value_dup_string`.** It allocates with libc `malloc` and hands ownership to the
caller, who frees it with `free()`. That is a genuine cross-language allocation contract and
there is no safe way to express it. It is the only place this library allocates on someone
else's behalf, and it is [decision 12](../reference/decisions.md).

**Not the unions.** The [layout parity](layout-parity.md) page explains why the C unions are
modelled as a pointer-sized word rather than a Rust `union`: reading a `union` field is
`unsafe` regardless of whether it can misbehave, and spending blocks there would have
inflated this count without buying any safety.

## Why publish it at all

Because the interesting failure mode of a C-to-Rust port is not "it does not compile". It is
a port that reaches memory safety by writing C in Rust syntax — `unsafe` at every awkward
corner until the borrow checker stops objecting. The count, per crate, with a rule that the
core cannot contain any, is the cheapest available evidence that did not happen.
