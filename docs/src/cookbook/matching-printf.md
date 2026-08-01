# Step 4: match printf exactly

Once the parser worked, 2,196 tests still failed. Almost none of them were parser
bugs. The tests stringify what they parsed and compare it to a fixture, and the
stringifier was still a stub, so every one of them reported:

```
FAIL!  : tst_Parser::arrays(0) '!err' returned FALSE. Got error ""; decoded stream:
```

An empty error message and an empty stream. Two stubs, thousands of failures. Worth
remembering when a failure count looks catastrophic: **cluster the failures before you
start fixing them.** One missing function can account for half your red.

## The part Rust cannot do for you

C prints doubles with `%.17g`. Rust has no equivalent, and the difference is not
cosmetic. It is compared against fixtures character by character.

The rule, from C99 §7.19.6.1, is short. Let `X` be the base-10 exponent:

- If `-4 <= X < precision`, print `%f`-style with `precision - 1 - X` digits after
  the point.
- Otherwise print `%e`-style with `precision - 1` digits.
- Either way, strip trailing zeros, and the point if nothing survives.

The trick that makes it easy in Rust is that `{:e}` hands you `X` for free:

```rust
let sci = format!("{:.*e}", precision - 1, v);
let (mantissa, exponent) = sci.split_once('e').unwrap();
let x: i32 = exponent.parse().unwrap();
```

Then follow the rule literally. Don't be clever; the rule is the specification.

## Do not write the expected values from memory

This is the actual lesson of this chapter. My first test looked like this:

```rust
assert_eq!(format_g(1e-4, 17), "0.00010000000000000000");
```

It failed. I assumed my implementation was wrong. It was not. My expectation was.
`%.17g` of `1e-4` is `0.0001`, because trailing zeros come off after the `%f`
formatting, not before.

I got three of these wrong in a row (`1e-4`, `3.14159`, `9.8765`) before doing the
obvious thing:

```console
$ python3 -c "print('%.17g' % 1e-4)"
0.0001
```

Python's `%` formatting delegates to the same C rules. Thirty seconds of generating
ground truth would have saved three rounds of second-guessing correct code.

**Generate your fixtures from the thing you are trying to match.** Not from your
memory of what it does. If you are porting from C, you have a C compiler and a shell
right there.

The switch points are where the interesting cases live, so pin those specifically:

```rust
assert_eq!(format_g(1e16, 17), "10000000000000000"); // %f side
assert_eq!(format_g(1e17, 17), "1e+17");             // %e side
assert_eq!(format_g(1e-4, 17), "0.0001");            // %f side
assert_eq!(format_g(1e-5, 17), "1.0000000000000001e-05");
```

## The other classic: UTF-8 escaping

The second half of this module escapes strings, and it has one detail worth stealing.
Upstream's condition for "printable, emit as-is" is:

```c
if (uc < 0x7f && uc >= 0x20 && uc != '\\' && uc != '"')
```

That is `< 0x7f`, not `<= 0x7f`. So `DEL` gets escaped to ``. It is the kind of
boundary you will reproduce wrongly if you translate the *intent* ("printable ASCII")
instead of the *code*.

Translate the code. Write the comment about intent next to it.

```rust
// Note 0x7f is escaped too: the condition upstream is `< 0x7f`, not `<= 0x7f`.
_ if (0x20..0x7f).contains(&uc) => out.push(ch),
```

Astral characters are the other one: upstream emits a surrogate pair, so `😀` becomes
`😀` rather than one `ὠ0`. Rust's `char` is a full code point, so you
have to split it back apart yourself:

```rust
let hi = (uc >> 10) + 0xd7c0;
let lo = (uc % 0x400) + 0xdc00;
```

Neither of these is hard. Both are invisible until a fixture disagrees with you.
