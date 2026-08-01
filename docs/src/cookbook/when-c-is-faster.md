# Step 5: when the C is faster than your Rust

The suite went green at 4,929 of 4,929 and the port was 1.49x slower than the library
it replaced. Not on one pathological input, but on all eight benchmark files, from 1.23x
to 1.83x, with p99 tracking p50 so it was systematic rather than tail noise.

It ended up 2.8x faster on all eight. This chapter is how, and it is mostly about the
hypotheses that were wrong on the way, because there were more of those than there were
fixes.

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

Nothing. Not a small effect. No effect at all. If the story had been right, taking TBAA away
should have cost C most of its advantage.

**Test the explanation, not just the fix.** The fix would have been the same either
way, and I would have shipped a confident, plausible, wrong paragraph about it.

## Wrong hypothesis two: it is just inlining

Next observation, found by accident: a plain `-O2` build of upstream is 1.66x slower
than the `-O3` build the benchmark compares against. Whatever C is getting, it is
getting from `-O3`. `-O3` mostly means more aggressive inlining, so: inlining, then.

Two more builds:

| upstream build | `parse` p50 vs stock `-O3` |
|---|---|
| `-O2 -finline-functions` | 1.34x slower |
| `-O3 -fno-unroll-loops -fno-tree-loop-vectorize` | 1.03x |
| `-O3 -fno-ipa-cp-clone` | **1.17x slower** |

Inlining alone does not recover it. Unrolling and vectorisation are worth almost
nothing here. But `-fipa-cp-clone`, interprocedural constant propagation *with
cloning*, is worth 17% on its own.

That pass makes a specialised copy of a function for a call site where an argument is
constant, then folds the constant through the copy. Applied to a parser whose reads
all branch on one flag, it produces a version of the read path with the branch gone.

Which is monomorphisation. GCC was doing it in the optimiser.

## The Rust version of that pass is the type system

rustc will not speculate on a runtime value, and it should not, because that is a heuristic,
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
extra `.text`. Three more rounds follow.

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
[decision 14](../reference/decisions.md) so that nobody, including me in a week,
tidies it back.

## Round two: read the generated code, then the profile

Mean 1.038 was better than 1.492 and still not a win. Two more rounds got it to
0.984, and neither came from having another idea. Both came from looking.

**The disassembly.** `advance_recursive` opened with the obvious spelling:

```rust
if is_fixed_type(it.type_) { return advance_internal::<S>(it); }
if !is_container(it.type_) { /* string */ }
```

LLVM compiled `is_fixed_type` into a rotate and a bit-table lookup against a
constant. Thirteen instructions and three branches, before any work, in a function
that runs 504,000 times on the nesting corpus. GCC compiled the same logic into
three instructions and one branch, because it noticed what I had not: the classes
being tested are each a *pair* of major types differing only in bit 5. Byte string
is 0x40 and text is 0x60; array is 0x80 and map is 0xa0. Mask that bit off and each
pair collapses to one comparison.

```rust
match it.type_ & !MAJOR_PAIR_BIT {
    TYPE_BYTE_STRING => …,   // both string types
    TYPE_ARRAY => …,         // both container types
    _ => return advance_internal::<S>(it),
}
```

Five instructions, two branches. The clearer source was the slower one, and the
faster one is arguably clearer once the comment explains the bit.

While in there: the string branch sets up a six-argument call, and every register
that call needs is one `advance_recursive` must push on entry and pop on exit. On a
function that recurses once per nesting level, a branch taken only at the leaves was
charging its register pressure to the whole descent. Behind `#[inline(never)]` the
frame went from 48 bytes to 32.

**The profile.** callgrind on the map-heavy corpus put 44% of parse time in
`iterate_string_chunks`, where we ran 16.4 M instructions to the C's 13.6 M. The C
symbol was named `iterate_string_chunks.constprop.0`.

That suffix again. The walker takes what-to-do-with-each-chunk as an argument, so
the branch lives inside the chunk loop, and GCC had cloned it per call site. Exactly
the same mistake as the byte source, one level down, and it had been sitting there
the whole time. Three types implementing one `Op` trait, and the choice moves to the
call site.

That last one took the mean under 1.0.

| | mean | files faster than C |
|---|---:|---:|
| before any of this | 1.492 | 0 of 8 |
| monomorphise the source | 1.038 | 3 of 8 |
| fix the dispatch, outline the string branch | 1.014 | 4 of 8 |
| monomorphise the chunk operation | **0.984** | **5 of 8** |

## Round three: stop competing on their terms

0.984 is parity, and parity was where three rounds of careful tuning had left it. Each
round had made the same move: find where the C's compiler was doing something rustc
was not, and do it by hand. That works right up until you have caught up, and then it
stops, because you are running the same algorithm.

So the last round asked a different question. Not "why is their code faster here" but
"what is this code actually for".

`cbor_value_advance` skips the item under the cursor and everything nested inside it.
Upstream walks that subtree recursively, decoding each item it passes into a
`CborValue`. Then it discards all of it. Nothing survives the walk except where the
cursor ended up and whether anything was malformed.

Which means the decoding is work nobody reads. And it is not a rounding error: on a
flat array of integers it was 106 instructions an item in this port and 117 in the C,
where the walk itself needs about ten.

The replacement is one flat loop that reads heads, adds lengths, and keeps a small
stack of how many items each open container still owes. Nesting turns out not to
matter when you are only skipping: a container of N items is just N more items to get
through.

| | mean | files faster than C |
|---|---:|---:|
| before any of this | 1.492 | 0 of 8 |
| monomorphise the source | 1.038 | 3 of 8 |
| fix the dispatch, outline the string branch | 1.014 | 4 of 8 |
| monomorphise the chunk operation | 0.984 | 5 of 8 |
| **scan instead of descend** | **0.362** | **8 of 8** |

The three tuning rounds together were worth 1.49x to 0.98x. Asking what the function
was for was worth 0.98x to 0.36x.

### The part that makes it safe to do at all

Rewriting a traversal in a port is exactly the kind of change that quietly breaks
behaviour, because the errors are the hard part. CBOR has a specific taxonomy of them
and the tests check which one comes back, not just that something did.

So the scan never reports an error. It hands back to the recursive code the moment it
meets anything it does not want to reason about: a malformed head, a length the
original would reject, nesting past 64 levels, a break where one may not go. Every
error still comes from the original path, in the original order. The scan can be right
or it can be absent; it cannot be subtly wrong about what went wrong.

That property is what let this ship. It also means the check is easy to state: replay
inputs through `cbor_value_advance` against both libraries, compare the error code, the
final cursor offset and the resulting type. Across the fuzz corpus, the regression
corpus and the benchmark corpus, 3,980 inputs, zero differences.

Building that check found a bug that had nothing to do with the scan, and predated it
by weeks: upstream copies the cursor into the caller's `next` *before* walking a string
and works on that, so a failed walk still says how far it got, while this port walked a
local copy and wrote it back only on success. Thirty-seven inputs disagreed. Same shape
as the pretty printer's missing `copy_current_position` from an earlier chapter: right
error, wrong state left behind, invisible to anything that only checks return values.

Two rounds of that now. When a port is wrong in a way tests do not catch, it is usually
about what it leaves behind rather than what it returns.

## What to take from this

- Delete the suspect before optimising it. A build you throw away answers *where* in
  minutes.
- A hypothesis that explains the numbers is not the same as the one that is true. Both
  of mine did, and both were wrong. Compiler flags are a cheap way to interrogate the
  other language's implementation directly.
- When C beats you, ask which pass is doing it. `-fno-` your way through the ones that
  fit the shape of the code. The answer is often a specific, nameable transformation
  rather than "C is closer to the metal".
- Idiomatic is a prior, not a proof. Measure the idiomatic version too.
- Once you have found one instance of a pattern, grep for the rest of it. The chunk
  walker had the identical defect as the byte source and survived two more rounds of
  tuning because I was looking at the thing I had just changed.
- Read the disassembly of the function you think is hot. Not to hand-optimise it, but
  because the compiler will show you which of your source constructs it did not like,
  and that is a much shorter list than the things you could try.
- Matching the original's algorithm gets you to parity, and then it stops. Parity is
  the ceiling of transliteration. Going past it means asking what the function is for,
  which is a question about the API and not about the code.
- When you do depart from the original, build the departure so it can only be right or
  absent. A fast path that never reports an error cannot get the error taxonomy wrong,
  and that is most of what there is to get wrong in a port.

And publish the regression while it is still a regression. This port shipped a 1.48x
number in its README for a day. That is what made it worth fixing.
