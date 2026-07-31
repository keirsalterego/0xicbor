# Step 3: write the first module

Pick the smallest self-contained thing in the library and do that first. Not because
it unlocks the most tests — it usually unlocks none — but because it is where you find
out whether your setup works before you have anything invested in it.

Here that was half-precision floats: IEEE 754 binary16, about sixty lines, no
dependencies on anything else in the codebase.

## Why not just add a crate

The `half` crate exists and is good. Using it would have been one line.

The rule for this project was no third-party CBOR or float crates, and the reason
generalises past hackathon rules: **wrapping a crate is not a port.** If the interesting
part of the module is delegated, you have not learned whether you can reproduce the
original's behaviour — you have learned whether two libraries happen to agree.

It is also genuinely small. Sixty lines is not a heroic act.

## The shape of it

A binary16 is 1 sign bit, 5 exponent bits, 10 mantissa bits. Going *up* to `f32` is
always exact, because every half value fits. Three cases:

```rust
match exp {
    0 if mant == 0 => f32::from_bits(sign),          // zero
    0              => { /* subnormal: renormalise */ }
    0x1f           => { /* infinity or NaN */ }
    _              => { /* normal: rebias by 127 - 15 = 112 */ }
}
```

The subnormal case is the only one with a loop: shift the mantissa left until the
implicit bit appears, paying one exponent step per shift.

```rust
let mut m = mant;
let mut shifts = 0;
while m & 0x400 == 0 {
    m <<= 1;
    shifts += 1;
}
f32::from_bits(sign | ((113 - shifts) << 23) | ((m & 0x3ff) << 13))
```

One thing to know before you start: **`core` has no `powi`.** If you are writing
`no_std`, the arithmetic shortcuts (`mant as f32 * 2f32.powi(-24)`) are not available
and everything has to be shifts and masks. Better to know that at the top than to
write it twice.

## Going down is where the bugs are

`f32` to `f16` is lossy, so it needs a rounding mode, and the right one is
round-to-nearest-**even**. Ties go to the even neighbour, not away from zero:

```rust
fn round_up(value: u32, shift: u32) -> u16 {
    let half = 1u32 << (shift - 1);
    let rest = value & ((1u32 << shift) - 1);
    let odd  = (value >> shift) & 1;
    u16::from(rest > half || (rest == half && odd == 1))
}
```

If you get this wrong, almost everything still works. Only exact ties differ, and
exact ties are rare in casual testing and *guaranteed* in a fixture suite.

## The test that makes it free

Half-floats have 65,536 possible values. That is small enough to check all of them:

```rust
#[test]
fn round_trips_exactly() {
    for h in 0u16..=0xffff {
        if is_nan(h) { continue; } // NaN payloads are not preserved bit-for-bit
        assert_eq!(encode(decode(h)), h, "half {h:#06x} did not round-trip");
    }
}
```

This runs in under a millisecond and it is a complete proof for one direction. When
your input domain is small, enumerate it. Property tests and fuzzing are for when you
cannot.

Then pin the boundaries separately, because a round-trip test will not catch a
consistently wrong rounding mode:

```rust
assert_eq!(decode(0x7bff), 65504.0);      // largest finite half
assert_eq!(decode(0x0001), 5.9604645e-8); // smallest subnormal
assert_eq!(encode(2049.0), encode(2048.0)); // tie rounds to even
assert_eq!(encode(70000.0), 0x7c00);      // overflow saturates to infinity
```

Sixty lines of implementation, four tests, and you never think about it again. That is
what you want from the first module.
