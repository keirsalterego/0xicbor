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
0
```

A bare `grep -rn unsafe crates/` returns 4, and all four are prose: two lines of module
documentation, one sentence in `cbor-core`'s header, and the `#![forbid(unsafe_code)]`
attribute itself. Counting `unsafe {` counts blocks, which is the number that means
something.

Zero, then, because the shim is still stubs and nothing dereferences its arguments yet. This
number will grow as the port lands, and it gets published as it grows rather than at the
end. A count that only moves in the honest direction is not worth much if nobody sees the
intermediate values.

For scale: [uv][uv] ships 73 `unsafe` blocks. [Bun][bun] ships 13,044. Neither number is
damning on its own — a runtime that talks to JavaScriptCore has different obligations than a
package resolver — but they bracket what "a lot" and "a little" look like in shipped Rust.

[uv]: https://github.com/astral-sh/uv
[bun]: https://github.com/oven-sh/bun

## Where the blocks will be

Predictable, and worth naming in advance so growth can be checked against the plan:

**Pointer validation at every entry point.** Each of the 44 exported functions receives raw
pointers from C. The shim converts them to references once, at the boundary, and everything
past that point is safe Rust operating on `&CborValue`.

**`_cbor_value_dup_string`.** It allocates a buffer and hands ownership to the caller, who
frees it with `free()`. That is a genuine cross-language allocation contract and there is no
safe way to express it.

**Nothing else, if the design holds.** The [layout parity](layout-parity.md) page explains
why the C unions are modelled as a pointer-sized word rather than a Rust `union`: reading a
`union` field is `unsafe` regardless of whether it can misbehave, and spending blocks there
would inflate this count without buying any safety.

## Why publish it at all

Because the interesting failure mode of a C-to-Rust port is not "it does not compile". It is
a port that reaches memory safety by writing C in Rust syntax — `unsafe` at every awkward
corner until the borrow checker stops objecting. The count, per crate, with a rule that the
core cannot contain any, is the cheapest available evidence that did not happen.
