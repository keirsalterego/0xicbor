# Step 5: when the C is faster than your Rust

The suite went green at 4,929 of 4,929 and the port was 1.49x slower than the library
it replaced. Not on one pathological input — on all eight benchmark files, from 1.23x
to 1.83x, with p99 tracking p50 so it was systematic rather than tail noise.

This chapter is about closing that gap, and mostly about the two hypotheses that were
wrong on the way.

## First: is it one thing or everything?

A 1.5x spread across eight structurally different documents usually means one shared
cost, not eight separate ones. The cheapest way to test that is to delete the suspect
and see what happens, even if the deletion is not something you could ship.

The suspect was the source dispatch. Upstream lets a caller supply their own reader
instead of a flat buffer, and every read checks which it is:

```rust
fn can_read(it: &CborValue, len: usize) -> bool {
    if external(it) {
        return (ops(it).can_read_bytes)(it.source.0, len);
    }
    (end(it) as usize).saturating_sub(ptr(it) as usize) >= len
}
```

`external` is `(*it.parser).flags & EXTERNAL_SOURCE != 0`. So: hardcode it to `false`,
rebuild, re-measure, throw the build away.

| corpus | with the branch | branch compiled out |
|---|---:|---:|
| `map_heavy` | 1.83x | **1.10x** |
| `tagged` | 1.58x | **1.07x** |
| `text_utf8` | 1.70x | **1.11x** |
| `flat_array` | 1.18x | **0.99x** |

One branch, essentially the whole regression. That is the answer to *where*. It is not
the answer to *why*, and the difference matters, because upstream has the identical
branch in the identical place and is not slow.

## Wrong hypothesis one: strict aliasing

The obvious story: C has type-based alias analysis. `it->source.ptr` is a `uint8_t *`
and `it->parser->flags` is a `uint32_t`, different types, so a C compiler may assume a
write to one cannot touch the other and hoist the flag load out of the loop. Rust
emits no TBAA metadata at all, so LLVM must reload after every cursor write.

It is a tidy story. It is also testable in about three minutes: rebuild upstream with
`-fno-strict-aliasing` and re-run the same benchmark.

```
mean ratio: 0.994
```

Nothing. Not a small effect — no effect. If the story had been right, taking TBAA away
should have cost C most of its advantage.

**Test the explanation, not just the fix.** The fix would have been the same either
way, and I would have shipped a confident, plausible, wrong paragraph about it.

## Wrong hypothesis two: it is just inlining

Next observation, found by accident: a plain `-O2` build of upstream is 1.66x slower
than the `-O3` build the benchmark compares against. Whatever C is getting, it is
getting from `-O3`. `-O3` mostly means more aggressive inlining, so — inlining, then.

Two more builds:

| upstream build | `parse` p50 vs stock `-O3` |
|---|---|
| `-O2 -finline-functions` | 1.34x slower |
| `-O3 -fno-unroll-loops -fno-tree-loop-vectorize` | 1.03x |
| `-O3 -fno-ipa-cp-clone` | **1.17x slower** |

Inlining alone does not recover it. Unrolling and vectorisation are worth almost
nothing here. But `-fipa-cp-clone` — interprocedural constant propagation *with
cloning* — is worth 17% on its own.

That pass makes a specialised copy of a function for a call site where an argument is
constant, then folds the constant through the copy. Applied to a parser whose reads
all branch on one flag, it produces a version of the read path with the branch gone.

Which is monomorphisation. GCC was doing it in the optimiser.

## The Rust version of that pass is the type system

rustc will not speculate on a runtime value, and it should not — that is a heuristic,
and heuristics are exactly what a language with generics does not need. The
transformation is available; it just has to be asked for.

So the four source operations became a trait, with the two sources as types:

```rust
trait Source {
    fn can_read(it: &CborValue, len: usize) -> bool;
    unsafe fn read(it: &CborValue, offset: usize, len: usize) -> u64;
    fn advance(it: &mut CborValue, n: usize);
    fn transfer_string(...) -> c_int;
}

struct Buffer;   // CborValue::source is the cursor, CborParser::source is the end
struct Reader;   // CborValue::source is a token, CborParser::source is a vtable
```

Every internal function took `S: Source`, and each exported entry point picked an
instantiation exactly once:

```rust
macro_rules! dispatch {
    ($it:expr, $f:ident($($arg:expr),* $(,)?)) => {
        if external($it) { $f::<Reader>($($arg),*) } else { $f::<Buffer>($($arg),*) }
    };
}
```

The `Buffer` instantiation now contains no branch and no indirect call anywhere in it.
Mean ratio 1.492 to 1.038, three of eight files faster than C, for 4,128 bytes of
extra `.text`.

Note what did *not* change: the flag still lives in `CborParser::flags`, at the offset
the C header specifies, set by the same function that always set it. The ABI cannot
tell the difference. Specialising on a value that has to stay where C put it is fine
as long as you only read it where C can see you reading it.

## And one more wrong idea, measured before believed

`advance_recursive` recurses once per nesting level, and at each level it builds a
`CborValue` that `enter_container` immediately overwrites in full. Obvious waste, and
the fix is the *more* idiomatic Rust: return the value instead of filling an
out-parameter. It is what the rest of this port does, and it is in the decision log as
a principle.

On the nesting-heavy corpus file it went from 1.25x to 1.68x.

Three words returned by value go through the return slot on every one of 160,000
calls; filled in place, they were already where the next call wanted them. The
out-parameter stayed, and the reasoning is
[decision 14](../reference/decisions.md) so that nobody — including me in a week —
tidies it back.

## What to take from this

- Delete the suspect before optimising it. A build you throw away answers *where* in
  minutes.
- A hypothesis that explains the numbers is not the same as the one that is true. Both
  of mine did, and both were wrong. Compiler flags are a cheap way to interrogate the
  other language's implementation directly.
- When C beats you, ask which pass is doing it. `-fno-` your way through the ones that
  fit the shape of the code — the answer is often a specific, nameable transformation
  rather than "C is closer to the metal".
- Idiomatic is a prior, not a proof. Measure the idiomatic version too.

And publish the regression while it is still a regression. This port shipped a 1.48x
number in its README for a day. That is what made it worth fixing.
