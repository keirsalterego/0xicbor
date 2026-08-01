# Layout parity

`cbor.h` declares 103 public functions. Only 44 of them are real exported symbols. The other
59 are `static inline` and read struct fields directly:

```c
CBOR_INLINE_API CborType cbor_value_get_type(const CborValue *value)
{
    return (CborType)value->type;
}
```

That function compiles into *the caller*, which is the Qt test binary, not into
`libtinycbor.a`.
No amount of correct Rust can influence what it returns. If `CborValue::type` is at a
different offset in the Rust struct than the C header believes, the test reads a different
byte and the failure looks like a parser bug rather than a layout bug.

So the struct layouts are not an interface this port gets to design.

## Pinning the numbers

At kickoff a throwaway C program dumped every size, alignment and offset. It was run
*outside* the repository so no C landed in the tree, and its output was committed to
`crates/cbor-ffi/abi-layout.txt`:

```
sizeof CborEncoder  = 32
alignof CborEncoder = 8
offsetof CborEncoder . data      = 0
offsetof CborEncoder . end       = 8
offsetof CborEncoder . remaining = 16
offsetof CborEncoder . flags     = 24

sizeof CborParser   = 16
sizeof CborValue    = 24
offsetof CborValue . parser    = 0
offsetof CborValue . source    = 8
offsetof CborValue . remaining = 16
offsetof CborValue . extra     = 20
offsetof CborValue . type      = 22
offsetof CborValue . flags     = 23
```

Those numbers are then asserted in a Rust test, so the tripwire trips at `cargo test`
rather than three hours into debugging a phantom parser failure.

## The union question

All three structs contain a C union:

```c
union {
    uint8_t *ptr;
    ptrdiff_t bytes_needed;
    CborEncoderWriteFunction writer;
} data;
```

Rust has `union`, and using it would be the more literal translation. This port uses a
single pointer-sized word instead:

```rust
#[repr(transparent)]
pub struct Word(pub *mut c_void);
```

Every member of every one of these unions is exactly pointer-sized and pointer-aligned, so
the layouts are identical, and the layout test proves it rather than assuming it.

The reason to prefer the word is the [unsafe budget](unsafe-budget.md). Reading any field
of a Rust `union` is `unsafe`, whether or not it can actually misbehave, because the
compiler cannot know which member is live. Since the `unsafe` count in this port is a
published number, spending blocks on field reads that carry no real risk would inflate it
without buying any safety. Which member is live is decided by the owning struct's `flags`,
exactly as it is in C.
