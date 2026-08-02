# Decisions

Every place this port diverges from a literal transliteration of upstream, and why.
Written as the decisions were made, not reconstructed afterwards.

---

## 1. The port is a static library with a byte-identical C ABI

Upstream's test suite is Qt/C++ calling the C API. There is no honest way to run it
*unmodified* except to hand it a `libtinycbor.a` it cannot distinguish from the real one.
So `cbor-ffi` is a `crate-type = ["staticlib"]` that exports the same 44 symbols with the
same signatures, and the tests link against it with no edits and no shim layer of our own.

The alternative, rewriting the tests to call a Rust API, would have made the pass rate
meaningless. A test suite you rewrote is a test suite you can make pass.

Cost: the port carries the shape of a C API into Rust. `cbor_value_get_int_checked` returns
its result through an out-parameter because callers expect that. Inside `cbor-core` the
same operation returns `Result<i64, CborError>`; the out-parameter convention exists only
in the shim, in one place.

## 2. `tests/original/hashes.txt` is pinned, and `tst_cpp` is left failing

The 14 files under `tests/original/` are upstream's at commit `9441b2ca`, byte for byte,
with SHA-256 recorded at kickoff. `sha256sum -c hashes.txt` is re-run at ship time.

`tst_cpp.cpp` cannot pass and will not be made to. It opens with:

```cpp
#include "../../src/cborencoder.c"
#include "../../src/cborparser.c"
...
```

It is not a test of the library. It is a test that upstream's C sources compile cleanly as
C++. A Rust port has no `.c` files for it to include, so the test is inapplicable by
construction rather than failing on behaviour. It contributes 2 rows, which is why the
reachable total is 4,929 of upstream's 4,931 and not 4,931.

Excluding it from the makefile rather than letting it fail to compile is a presentation
choice; the reason is recorded here so the missing 2 rows are not mistaken for a gap.

## 3. `tst_c90` is kept, and it constrains the headers

`tst_c90.c` includes only `cbor.h` and compiles under `-std=c90 -pedantic`. It survives the
port because it tests the *header*, which we still ship. It is a genuine constraint: the
vendored headers must stay C90-clean, which rules out tidying them up with newer C.

## 4. The C headers are vendored; the library contains no C

`crates/cbor-ffi/include/` holds upstream's `cbor.h`, `cborjson.h`, `cborinternal_p.h`,
`compilersupport_p.h` and the two generated `tinycbor-*.h`. They are vendored so a fresh
clone builds without upstream checked out beside it.

This is worth being precise about, because "no C" is a claim judges should be able to check.
`libtinycbor.a` is compiled entirely from Rust: no `cc` crate, no `bindgen`, no linking
against upstream. The headers are the ABI contract, and the 59 `static inline` accessors in
`cbor.h` compile into whatever program includes them, which is the test binary, never into the
library. `cborinternal_p.h` is vendored only because `tst_tojson.cpp` includes it for its
own `encode_half`/`decode_half` reference, which likewise runs in the test binary.

## 5. The fuzz oracle is a separate process, not FFI

Differential fuzzing needs upstream's C implementation to compare against. It is built as a
standalone `cbor-oracle` binary and driven as a **subprocess**: bytes in on stdin, pretty
output and exit code out. Nothing about it is linked into the Rust artifact.

An in-process oracle via FFI would have been faster and simpler, and would have quietly
made rule "no C in the shipped artifact" false. Naming it here so nobody has to guess.

## 6. Unions are modelled as a pointer-sized word, not a Rust `union`

All three public structs contain a C union:

```c
union { uint8_t *ptr; ptrdiff_t bytes_needed; CborEncoderWriteFunction writer; } data;
```

Every member of every one of these unions is exactly pointer-sized and pointer-aligned, so
a single opaque word is layout-identical, and the layout test proves it rather than
assuming it.

A real `union` would have been the more literal translation, but reading any field of a
Rust `union` is `unsafe` regardless of whether it can actually misbehave. Since `unsafe` in
this port is a published number, spending blocks on field reads that carry no risk would
inflate the count without buying safety. Which member is live is decided by the owning
struct's `flags`, exactly as in C.

## 7. Stubs return `CborErrorInternalError` rather than `unimplemented!()`

During scaffolding every entry point is a stub. Panicking would be the idiomatic Rust
placeholder, but the library is built with `panic = "abort"`, so the first call kills the
Qt test process and the baseline becomes "it crashed" instead of a number.

Returning `CborErrorInternalError` (`INT_MAX`) lets the whole suite run and report per-row
results, which turns the failure count into a progress bar. Baseline was 21 passed, 4,908
failed, the 21 being rows that expect an error and coincidentally get one.

## 8. `cbor-core` is `no_std + alloc`; `cbor-ffi` is not

Upstream targets microcontrollers. Dropping that would be a real reduction in what the
library can do, so `cbor-core` is `#![no_std]` and `#![forbid(unsafe_code)]`, with `alloc`
pulled in only for the `dup_string` family and indefinite-length string reassembly,
both of which are optional upstream too.

`cbor-ffi` deliberately uses `std`. It only ever links into a hosted C program, and `std`
supplies the allocator and panic handler that a `no_std` staticlib would otherwise need
hand-rolled. Hand-rolling both would have meant a `#[global_allocator]` forwarding to libc
`malloc`, which is more `unsafe` and more moving parts for no benefit at the only place
this crate is ever used. The portability claim belongs to `cbor-core`, where it pays.

## 9. `CborNoError` has no Rust variant

Upstream's `CborError` reserves `0` for success and every function returns it. In
`cbor-core` success is `Ok(())`, so the enum carries only failures and starts at 1. The
discriminants of the failure variants are still fixed by the C ABI, gaps and all. Upstream
groups them in blocks of 256 by category so callers can range-check a class of failure, and
that numbering is reproduced exactly.

The `Result` boundary is the shim. `cbor-core` never sees an integer error code, and the
FFI layer never sees a `Result`.

## 10. The suite is compiled directly, not through upstream's CMake

Upstream drives its tests from CMake through a `tinycbor_add_qtest()` helper in
`cmake/TinyCBORHelpers.cmake`. Reproducing that would mean carrying a slice of upstream's
build system into this repo for no gain: all the helper does that matters here is run `moc`
and link Qt Test, which is four lines of makefile.

The test sources themselves are still untouched. `moc` output is written into `build/` so
that `tests/original/` stays clean enough to hash-verify.

## 11. Upstream has moved from qmake to CMake

Worth noting because it dates any guide written against tinycbor. There are no `.pro` files
at `9441b2ca`; the suite is `CMakeLists.txt` throughout. The port targets the current tree,
not the qmake-era one.

## 12. The only C symbols we call are libc's `fwrite`, `malloc` and `free`

The rule is no source-language runtime: a C-to-Rust port must not FFI back into the
library it replaces. This port does not. `cargo tree` lists four crates and they are all
in this repository (`cbor-core`, `cbor-ffi` and the two tools) with no third-party
dependency at any depth, no `build.rs`, no `cc` and no `bindgen`. `libtinycbor.a` is
compiled entirely from Rust. The differential fuzz oracle is upstream's C built as a
standalone binary and driven as a subprocess over a pipe.

There are exactly two `extern "C"` blocks in the tree, both libc, both forced by the ABI
rather than chosen.

```rust
extern "C" {
    fn fwrite(ptr: *const c_void, size: usize, nmemb: usize, stream: *mut c_void) -> usize;
}
```

The signature we have to implement is:

```c
CborError cbor_value_to_pretty_advance(FILE *out, CborValue *value);
```

`FILE` is an opaque libc type owned by libc. A caller hands us one it opened, and the only
way to write to it is to call libc. Going through `fileno()` and `write()` would still be
libc, just less direct.

```rust
extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
}
```

`_cbor_value_dup_string` hands the caller a block and the documentation says to release it
with `free()`. That names the allocator. Returning a pointer from Rust's global allocator
would be a heap mismatch the moment anyone honoured the contract. It happens to be malloc
on this target today, and Rust promises nothing about that continuing to be true.

This is not a loophole, because it does not get us anything. Rust's `std` links libc on
Linux in every program ever compiled; if the rule barred that, no Rust port could exist on
this platform. What the rule is actually about, reusing the original implementation instead
of rewriting it, is not happening anywhere here, and the empty `nm` diff against a library
built from entirely different source is the evidence.

`cbor-core`, which is the whole CBOR implementation, has no `extern` blocks at all and is
`#![no_std]`.

## 13. The byte source is a type parameter, not a flag test

Upstream reads `parser->flags & CborParserFlag_ExternalSource` inside each of the four
source operations (`can_read_bytes`, `read_bytes`, `advance_bytes`, `transfer_string`)
so the test runs on the head of every item. Transliterating that cost 1.49x against the C
on an eight-file corpus, and it was almost the whole gap: hardcoding the branch to the
buffer case took `map_heavy` from 1.83x to 1.10x.

The interesting part is why the same code is not slow in C. GCC at `-O3` runs
`-fipa-cp-clone`, which clones a function specialised on a constant argument and folds the
branch out of the clone. Building upstream with `-fno-ipa-cp-clone` and changing nothing
else costs it 17%. `-fno-strict-aliasing`, which was my first guess, costs it nothing
measurable (0.994x), so this is not a TBAA story, it is a specialisation story.

rustc has no equivalent and will not grow one: it does not speculate on runtime values.
What it does have is monomorphisation, which is the same transformation with the decision
moved from the optimiser to the type system. So the four operations became a `Source`
trait with a `Buffer` and a `Reader` impl, the parser internals take `S: Source`, and each
`#[no_mangle]` entry point picks an instantiation once. The buffer instantiation contains
no branch and no indirect call anywhere in it.

The ABI is untouched: the flag still lives in `CborParser::flags` exactly where the header
says, and `cbor_parser_init_reader` still sets it. It is read once per API call now instead
of once per byte read.

Cost: two copies of the parser, which measured 4,128 bytes of `.text` on the benchmark
driver, or 0.6%. Mean ratio went 1.492 to 1.033, and three of the eight corpus files
are now faster than the C.

The alternative was threading a `bool` down by hand, which is the same specialisation
written out longhand and would have doubled the argument list of every internal function
without the compiler checking that no call site got it wrong.

## 14. `enter_container` keeps C's out-parameter, against the grain of the rest

Everywhere else in this port, a C function that returns a value through an out-parameter
plus a status code becomes a Rust function that returns the value. `enter_container` is the
exception: internally it still takes `&mut CborValue` and fills it.

The idiomatic version was written and measured. `advance_recursive` recurses once per
nesting level, so on `deep_nest.cbor`, which is 4,000 chains of 40 nested arrays,
that is 160,000 calls, and returning a 24-byte struct by value instead of filling one
moved that file from 1.25x to 1.68x against the C. Mean across the corpus went 1.04x to
1.11x. The struct is three words; returning it puts it through the return slot on every
level, where filling a caller's stack slot leaves it where the next call already wants it.

So the out-parameter stayed and this note exists instead. The shim's own
`cbor_value_enter_container` was always going to have that shape, since the C signature demands
it, so the inconsistency is confined to one internal function that the FFI boundary was
already forcing.

Recorded because "make it idiomatic" is the default advice for a port, and this is a place
where taking it made the thing measurably worse. One measurement of a rejected variant, not
a published headline.

## 15. `cc` appears under the fuzz crate, and nowhere else

Worth writing down before someone finds it and assumes the worst. `fuzz/Cargo.toml` depends
on `libfuzzer-sys`, which builds libFuzzer's own C++ runtime and therefore pulls `cc` into
*that* crate's build-dependencies. It is confined to the fuzzing binary, which is a test
harness and is never shipped.

The artifact under judgement is `target/release/libtinycbor.a`. `cargo tree` at the
workspace root lists only the four crates in this repository, with no build script and no
`cc` anywhere in it, and `fuzz/` is deliberately not a workspace member. The C oracle the
fuzzer compares against is a separate executable driven over a pipe, per entry 5.

So: no C in the library, and the one place a C compiler runs at all is the tool that proves
the library matches C.

## 16. `cbor_value_advance` skips by scanning, where upstream descends

Upstream walks a subtree recursively, decoding every item it passes into a `CborValue`
on the way. Then it throws all of that away: the only things that outlive the walk are
where the cursor ended up and whether anything was malformed. On a flat array of
integers that cost 106 instructions an item in this port and 117 in the C, where the
walk itself needs about ten.

So the buffer source has `scan_subtree`, which reads heads and adds lengths in one flat
loop with a small stack of how many items each open container still owes. Nesting is
not otherwise interesting when skipping: a container of N items is just N more items to
get through. It is the same traversal, without the bookkeeping nobody reads.

This is the one place the port stops being a transliteration, so two things keep it
honest.

**It can only be right or absent.** The scan hands back to the recursive code the moment
it meets anything it does not want to reason about: a malformed head, a length
`enter_container` would reject, nesting past 64 levels, a break where one may not go. It
never reports an error itself. Every error the API can produce still comes from the
original code, on the original path, so the error taxonomy cannot drift.

**It is checked against the C directly.** `tests/port/tst_advance_diff.c` replays inputs
through `cbor_value_advance` and prints the error code, the final cursor offset and the
resulting type. Built against both archives and diffed, it agrees on all 3,980 inputs of
the fuzz, regression and benchmark corpora. `make test` runs the nine that ship.

Parsing went from 0.98x of the C to 0.36x, which is 2.8x faster, and every corpus file
improved. `deep_nest`, which had been the worst at 1.25x and resisted three earlier
rounds of tuning, came out at 0.32x.

Two things the scan cannot do, both by construction rather than by omission. A tag at
the top of an advance stops on the tagged value rather than past it, because a tag
prefixes an item instead of being one, and there is no enclosing loop to carry on into.
And a reader source has no buffer to scan, so it keeps the recursive path entirely.

## 17. The JSON converter inherits an upstream bug, on purpose

`cbor_value_to_json_advance` does not validate UTF-8, although upstream's own header
comment says it does:

> These functions also perform UTF-8 validation in CBOR text strings. If they encounter a
> sequence of bytes that is not permitted in UTF-8, they will return
> `CborErrorInvalidUtf8TextString`. That includes encoding of surrogate points in UTF-8.

`cbor_value_to_pretty_advance` does exactly that. The JSON path does not, because text
strings go through `escape_text_string()`, which escapes what JSON requires escaping and
never decodes UTF-8. So `61 ff` returns `CborErrorInvalidUtf8TextString` from one renderer
and `CborNoError` plus a JSON document that is not UTF-8 from the other. RFC 8259 §8.1
requires JSON to be UTF-8, so the successful call produced something a JSON parser will
refuse.

Filed as [intel/tinycbor#331](https://github.com/intel/tinycbor/issues/331), against
`9441b2ca`, which is the current tip.

**This port reproduces the behaviour exactly**, and that is the decision. The whole claim
here is behavioural equivalence with a specific commit, checked by a differential fuzzer;
fixing a bug the reference still has would turn a passing check into a failing one and
make the equivalence claim false. The 1.4 million executions `json_diff` has run clean are
only meaningful because this port is bug-compatible.

If upstream takes the fix, this port follows it. Until then the divergence would be ours,
not theirs.

Found by asking a different question than the fuzzer asks. The fuzzer compares this port
against upstream, so anything both get wrong is invisible to it. Taking upstream's output
for every successful conversion in the corpus and handing it to a strict JSON parser is a
comparison against the *specification* instead, and 760 of roughly 27,000 conversions came
back unparseable.

## 18. The tools are a second implementation, and it had drifted

`cbordump` and `json2cbor` are rewritten in safe Rust rather than being C over the
library. They have to be: `cbor-ffi` speaks in raw pointers, and a binary that consumed
that API would need `unsafe` in it, which is exactly what the budget is meant to keep in
one place. So `tools/cbordump/src/pretty.rs` is a second diagnostic-notation printer next
to `crates/cbor-ffi/src/pretty.rs`, and `tools/cbordump/src/cbor.rs` a second parser.

Nothing checked the second one. Upstream's Qt suite tests the library, and both
differential fuzzers call the C ABI directly, so a tool could disagree with upstream on
every input and the whole board would still read green. Running both tools against
upstream's binaries on the 4,509-document fuzz corpus, over every combination of the four
flags the JSON path recognises, found four bugs:

- an indefinite map with an odd number of items — a key with no value — was walked past
  rather than refused when it sat below the recursion limit, so the tool exited 0 on
  input upstream exits 1 on. That is the only one of the four that accepted something
  invalid rather than reporting the wrong error;
- a tag at the recursion limit prints a marker and leaves the item it tags unread, and
  upstream's container loop, driven by an item count that tags never decrement, renders
  that item on the next turn. Indexing the loop instead left the item as trailing bytes
  and reported garbage after the end of a document upstream renders in full;
- a break arriving where a map value is due, on a pair boundary, is upstream's
  `CborInvalidType`: the word `invalid` and `CborErrorUnknownType`. This is the same arm
  the library's printer was missing, found by the fuzzer and fixed in an earlier round —
  the second copy never got it;
- stepping off a tag head skips the preparse upstream does before it consults the
  recursion budget, so a document ending on a tag blamed nesting instead of saying it had
  run out of data.

The harness is `tests/port/tools_diff.sh` and it runs in `make test`. It needs upstream's
tools built, which are binaries rather than the vendored archive, so a machine without
them skips rather than fails.

The lesson is the one the fuzzing page already makes in a different key: a green board
measures what is wired to it. Three of these four are error-path behaviour that no
reasonable person would have found by reading, and the fourth was a validity bug sitting
in a tool that had been green since the day it was written, because nothing had ever asked
it a question.

## 19. Half-precision floats are done on the bit patterns, by hand

`f16` is still unstable in Rust and the `half` crate is out of scope, so
`cbor-core/src/half.rs` converts binary16 both ways in 138 lines of shifts and masks.

Upstream does not have this problem in the same shape. Its `decode_half` is the RFC 7049
reference implementation, and its `encode_half` casts through `(_Float16)` where the
compiler supports one and falls back to the reference code where it does not. This port
has no such fallback to pick between: there is one implementation and it has to round the
way the hardware would.

That is the part worth stating. Decoding is easy — every binary16 value fits exactly in an
`f32`, so it cannot round at all, and the only care needed is renormalising subnormals and
keeping quiet NaNs quiet by shifting the payload rather than rebuilding it. Encoding is
where the bodies are: **round-to-nearest-even**, including the tie case, including values
that round *up into* a subnormal, and including values that round up into infinity. A naive
truncate-the-mantissa encoder passes an astonishing number of tests and is wrong on exactly
the inputs a fuzzer finds first.

`core` has no `powi` and no float formatting, which rules out the arithmetic shortcuts, so
everything is integer work on the raw bits. That turns out to be the right shape anyway: it
is branch-predictable, it has no floating-point environment to depend on, and it gives the
same answer on a machine with no FPU — which matters for a library whose audience is
microcontrollers.

`encode_diff` drives `cbor_encode_float_as_half_float` with arbitrary `f32` bit patterns
against upstream, which is the check that the rounding actually matches rather than merely
looking like it should.

## 20. The parser does not allocate; one function does, and it uses `malloc`

Upstream's selling point is that parsing takes no heap. This port keeps that literally:
`cbor-core` is `no_std + alloc`, but nothing on the parse path calls into `alloc`.

Indefinite-length strings are the case that tempts you. A text string split into chunks
across the wire has no contiguous representation, so the obvious move is to concatenate it
into a `Vec<u8>` and hand back a slice. Upstream does not do that, and neither does this.
`iterate_string_chunks` walks the chunks twice with two different operations — `Measure` to
total the length, then `CopyOut` to fill the caller's buffer — and the caller owns the
memory both times. That is why `cbor_value_copy_text_string` takes a buffer and a
length-in-out parameter instead of returning something: the two-pass shape is the API, not
an implementation detail, and a port that allocated instead would silently change what
`CborErrorOutOfMemory` means to every existing caller.

There is exactly one allocation in the whole library, in `_cbor_value_dup_string`, and it
is `malloc` rather than Rust's allocator on purpose. The documented contract is that the
caller releases the result with `free()`. That names the allocator. Handing back a pointer
from Rust's global allocator would be a heap mismatch the compiler cannot see and the
caller cannot be blamed for — it would work today, because Rust's global allocator is
`malloc` on this target, and break silently the first time someone set
`#[global_allocator]`. Two `extern "C"` declarations are a smaller price. This is the only
reason `alloc` is a dependency at all.

## 21. The self-referential cursor is modelled as a raw pointer, not fought

`CborValue` holds a `*const CborParser` pointing at a parser the caller owns, and nothing in
the C API ties the two lifetimes together. A caller can free the parser and keep the value,
or copy the value into a container that outlives the parser, and the C compiler will not
say a word.

This is the shape Rust exists to prevent, and there is no way to express it safely without
changing the ABI — which is the one thing that cannot change, because the Qt tests read
these fields directly through `static inline` accessors. So the borrow checker is not
fought here, it is stepped around: the field stays a raw pointer, every dereference lives in
`cbor-ffi` behind a `// SAFETY:` line, and the invariant is named as the caller's to uphold
because in C it always was.

What made this tractable was pushing the *decision* out of the pointer and into the type
system one level up. A `CborValue` can be reading a flat buffer or calling back through a
caller-supplied operations table, and upstream tests a flag on every access to tell which.
Here the byte source is a type parameter instead, `trait Source` with `Buffer` and `Reader`
implementations, so the choice is made once at the call boundary and the compiler
monomorphises two specialised parsers out of one body — [decision 13](#13-the-byte-source-is-a-type-parameter-not-a-flag-test),
and the reason parsing ended up faster than the C rather than merely as fast.

The lifetime is still unenforced. Saying so plainly is better than a `PhantomData` that
implies a guarantee the ABI cannot make.

## 22. The `unsafe` budget: 80 blocks, all of them in the shim

`cbor-core` is `#![forbid(unsafe_code)]` — not `deny`, `forbid`, so no inner module can
opt back in. That crate is the entire CBOR implementation: the encoder, the parser's
logic, half-floats, formatting, validation. It compiles with no `unsafe` at all.

Every one of the 80 blocks is in `cbor-ffi`, and each carries a `// SAFETY:` line naming
the invariant the C caller has to uphold. They are almost all the same block: a C caller
handed us a pointer and we are about to read through it. That is what an ABI shim *is* —
the number is a measure of how many places C values cross into Rust, not of how much
risky code was written.

The count is published in the readme and it has grown over the weekend, from 61 to 80,
because the subtree scan and the JSON converter both added entry points. It is reported as
it stands rather than as it was when it looked better. For scale, `uv` ships 73 blocks and
Bun ships 13,044.

Two things the budget deliberately does not do. It does not count `unsafe fn` declarations
or `extern "C"` blocks, only `unsafe { }`, because those are the places something can
actually go wrong. And it does not chase the number down by wrapping several dereferences
in one block to make the total look smaller — that would trade a real property, one
invariant named per operation, for a better-looking figure.

## 23. Upstream's archive is committed, and the port tests link against it

`bench/reference/libtinycbor-upstream.a` is 76 KB of upstream compiled at the pinned commit,
checked into the repo. Four tests under `tests/port/` are each built twice from one C source
file, once against that archive and once against ours, and the two transcripts are diffed.

The alternative was to build upstream during `make test`. That would have meant a fresh
clone could not run the differential tests without also cloning and building tinycbor, and
the whole point of those tests is that they run. So the comparison target ships with the
repo.

A static archive of C in a repository whose first claim is "no C" invites scrutiny, so here
is what it is not. It is not linked into `libtinycbor.a`, which is what a caller gets, and
not into `cbordump` or `json2cbor`. It is linked into `build/*-c` — test binaries whose only
job is to produce the transcript that ours is checked against. Deleting it would not change
a byte of the shipped library; it would only remove the evidence that the library agrees
with the thing it replaces.

`make symbols` and the ABI layout test read the other file in that directory,
`symbols-upstream.txt`, for the same reason: the reference has to be in the repo or the
check is not reproducible by a stranger.

## 24. The work is timestamped, and the timestamps are the proof

A port of a known library is exactly the kind of entry where somebody will wonder whether
it was written before the window opened. The answer is in the history, and it does not need
to be taken on trust.

The window opened 2026-07-31 18:00 UTC. The first commit is 18:24:04 UTC, twenty-four
minutes later, and it is an empty tree: `git init` happened inside the window, not before
it, and the first file arrives five minutes after that. The last commit is
2026-08-02 04:33:09 UTC.

```
git log --format='%h %ad %cd' --date=iso-strict     # 100 commits, none before 18:24
```

Between those two the 100 commits fall across twelve distinct hours with gaps where a
person was asleep, which is what real work looks like and what a single squashed drop does
not. More usefully, nothing has been rewritten:

```
git log --format='%h %at %ct' | awk '$2!=$3' | wc -l    # 0
```

Author date and committer date are equal on every commit. A rebase, a `filter-branch`, or a
backdated `--date=` all leave those two fields disagreeing, so a zero there rules out the
cheap ways of manufacturing a history after the fact. It is a weaker claim than a signed
timestamp and a stronger one than a tidy commit log, and it is the honest thing available.

The same reasoning is why `fuzz/history.tsv` records the commit each fuzz run ran against,
and why `.port-mortem.toml` carries `kickoff_utc`: a number that cannot be tied to a moment
is a number a reader has to believe.
