# Step 2: match the ABI

If the original tests call your library through a C header, then some of that header
is not calls at all. It is code that reads your structs directly, compiled into the
*test binary*, where your Rust has no say.

In tinycbor, 59 of 103 public functions are like this:

```c
CBOR_INLINE_API CborType cbor_value_get_type(const CborValue *value)
{
    return (CborType)value->type;
}
```

If `CborValue::type` sits at a different byte offset in your Rust struct than the
header believes, that returns a different byte. The failure surfaces as a wrong parse
result three layers away, and you will go looking for it in your parser.

So don't guess the layout. Measure it.

## Get the numbers from C, once

Write a throwaway C program. Run it **outside your repository** so no C ends up in
your tree, and commit only its output:

```c
#define O(t, f) printf("offsetof %-12s . %-10s = %zu\n", #t, #f, offsetof(t, f))
    printf("sizeof CborValue = %zu\n", sizeof(CborValue));
    O(CborValue, parser);
    O(CborValue, source);
    O(CborValue, remaining);
```

```
sizeof CborEncoder  = 32   sizeof CborParser = 16   sizeof CborValue = 24
offsetof CborValue . parser    = 0
offsetof CborValue . source    = 8
offsetof CborValue . remaining = 16
offsetof CborValue . extra     = 20
offsetof CborValue . type      = 22
offsetof CborValue . flags     = 23
```

Then assert every one of those in a Rust test, so the tripwire trips at `cargo test`
instead of during a debugging session:

```rust
assert_eq!(size_of::<CborValue>(), 24);
assert_eq!(offset_of!(CborValue, extra), 20);
```

## What to do about C unions

All three public structs contain something like this:

```c
union { uint8_t *ptr; ptrdiff_t bytes_needed; CborEncoderWriteFunction writer; } data;
```

Rust has `union`, and using it is the literal translation. This port uses a single
pointer-sized field instead:

```rust
#[repr(transparent)]
pub struct Word(pub *mut c_void);
```

Every member of that C union is exactly pointer-sized and pointer-aligned, so the two
layouts are identical, and the layout test *proves* that rather than assuming it.

The reason to prefer the plain word is the unsafe budget. Reading any field of a Rust
`union` requires `unsafe`, whether or not it can actually misbehave, because the
compiler cannot know which member is live. If you are publishing your `unsafe` count
as a quality signal, you do not want a dozen blocks that exist only to read a pointer
you already knew was there.

Which member is live is decided by the struct's own `flags` field, exactly as it is
in C. You have not lost any safety, because there was never any to lose.

## Prove the symbols match

```console
$ nm -g --defined-only target/release/libtinycbor.a \
    | awk '$2 ~ /^[A-TV-Z]$/ {print $3}' | sort -u \
    | diff reference-symbols.txt -
```

An empty diff means any program that linked against the original links against yours.
That is the strongest statement a drop-in replacement can make, and it is one command.

Do this at the *stub* stage, before any real code exists. Symbol parity is a property
of the interface, so it costs nothing to establish up front and then guards every
commit afterwards. Finding a missing symbol on the last day is much worse than finding
it on the first.
