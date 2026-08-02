# Differential fuzzing

The original test suite proves this port satisfies the cases Intel thought to write.
It cannot prove anything about the cases nobody thought of. That is what the fuzzer is
for: generate arbitrary bytes, feed the identical bytes to this port and to upstream's
C, and compare what comes back.

## The oracle runs in another process

The obvious way to build this is to link upstream's C into the fuzz binary and call it
directly. It would be faster and simpler, and it would quietly make the central claim
of this port, that the shipped library contains no C, false.

So `fuzz/oracle/cbor-oracle` is a standalone executable built from upstream's sources,
driven as a **subprocess**: bytes in on stdin, the rendering on stdout, the `CborError`
on stderr. Nothing about it is linked into `libtinycbor.a`. The cost is roughly 2,000
executions per second instead of tens of thousands, which is a real price, and it is
worth paying to keep the claim checkable. This is
[decision 5](../reference/decisions.md).

## Four targets

`pretty_diff` compares `cbor_value_to_pretty_advance`. `json_diff` compares
`cbor_value_to_json_advance`, and it exists because the two share a parser and very
little else.

JSON has to refuse things diagnostic notation renders happily: a map key that is not a
string, an integer too large to survive a double, a byte string with no tag saying how
to encode it. It has its own escaping rules, its own base64 and base16, and a `$cbor`
metadata sidecar. Fuzzing the printer reaches none of that.

`CborToJsonFlags` also changes the output substantially, turning tags into objects and
byte strings into base64url and map keys into strings, so `json_diff` takes the bitmask
from the first byte of each input and passes the same value to both sides. A harness
that only ever sent the default would leave most of the converter dark.

`validate_diff` compares `cbor_value_validate`, which is a predicate rather than a
renderer: there is no output to diff, so the error code is the whole answer, and that
makes it the strictest of the four. Its `CborValidationFlags` is a 27-bit matrix where
almost every bit gates a separate check: shortest-form integers and floats, sorted
maps, unique keys, tag use, UTF-8, finite floats, unknown simple types. So the first
four bytes of each input are the bitmask and the rest is the document.

`encode_diff` is the odd one, and it is the one that mattered most.

The first three read CBOR. Half of tinycbor *writes* it, and nothing differential had
ever touched that half, for a reason that looks like a good one until you look twice:
an encoder takes calls, not bytes, so there is no input to hand it.

So the fuzzer's bytes become the calls. Each input is a little program: a two-byte
output buffer size, then a stream of opcodes with their operands, interpreted twice,
once against this port and once against the oracle, covering every encoder entry point
including the container stack. What is compared is the error from every single call, the
bytes each side wrote, and how much more room each says it needed.

The buffer size is the operand that matters. Upstream's encoder does not stop when it
runs out of room: it turns its union from a write pointer into a byte counter, keeps
walking the calls, and reports at the end what a big enough buffer would have taken.
That bookkeeping is most of the non-obvious code in `cborencoder.c`, and it is
unreachable with a buffer that always fits. So the size comes out of the input, small,
and most programs overrun on purpose.

Bit 15 of that size word picks `cbor_encoder_init_writer` instead: no buffer at all, a
callback per fragment. It is a separate branch through the same `append()` with its own
idea of running out of room, and the `CborEncoderAppendType` it hands the callback is ABI
surface that nothing else checks, so the callback records that argument alongside the
bytes, and refuses everything past the same size limit so the error travelling back *out*
through the encoder gets exercised too. That one bit was worth 40 edges.

The cost of this design is a second interpreter. The program format is specified once,
in a comment above `run_encoder_program` in the oracle, and implemented twice. The two
have to agree about the program before they can disagree about the encoder, which means
an early divergence is much more likely to be the two readers than the two encoders.
Thirty hand-written seed programs covering every opcode were replayed through both sides
before the fuzzer ran at all.

```console
$ ./fuzz/run.sh                      # the printer, the default
$ TARGET=json_diff ./fuzz/run.sh     # the converter
$ TARGET=validate_diff ./fuzz/run.sh # the validator
$ TARGET=encode_diff ./fuzz/run.sh   # the encoder
```

Each target keeps its own corpus and its own log.

## What is compared, and what deliberately is not

```rust
if our_err != their_err          { panic!("DIVERGENCE (error code)")  }
if their_err == 0 && our_out != their_out { panic!("DIVERGENCE (stdout)") }
```

Error codes are compared always. Output bytes are compared **only when the render
succeeded**, and that exception is not laziness.

Upstream streams its output through a printf-style callback as it walks, so a render
that fails half way through has already emitted the first half. This port renders into
a `String` and writes once at the end, so a render that fails emits nothing at all. On
one real input that is 2,906 bytes against zero. The two are not comparable and never
will be; what *is* comparable on a failure is which error was reported, and that is
compared strictly.

That design choice is [in the pretty printer's module comment](https://github.com/keirsalterego/0xicbor/blob/main/crates/cbor-ffi/src/pretty.rs)
and it costs nothing a caller can observe on the success path, which is the only path
where the API promises specific bytes.

## Run history

Stated in full, including the run that found something.

| execs | duration | result |
|---:|---:|---|
| 103,703 | 60 s | clean |
| 252,830 | 121 s | clean |
| 420,793 | 900 s | **one divergence**, see below |
| 1,507,421 | 901 s | clean, after the fix |
| 511,224 | 301 s | clean, after the string-walk change |
| 529,508 | 301 s | clean, after the subtree scan landed |
| 922,346 | 601 s | clean, after the scan learned indefinite lengths |

And `json_diff`, which is newer:

| execs | duration | result |
|---:|---:|---|
| 242,469 | 121 s | clean |
| 1,388,985 | 901 s | clean |
| 744,685 | 601 s | clean, after the `CborErrorIO` fix |

It reaches 1,188 edges against the printer target's 765, which is the answer to whether
it was worth adding: about 55% more of the library, none of it previously fuzzed.


`validate_diff`:

| execs | duration | result |
|---:|---:|---|
| 362,801 | 301 s | clean |
| 1,677,406 | 901 s | clean |

`encode_diff`, which found something in its first quarter hour:

| execs | duration | result |
|---:|---:|---|
| 2,653 | first run | **one divergence**, see below |
| 2,032,920 | 1,201 s | clean, after the fix |
| 1,644,600 | 901 s | clean, with the writer callback path added |

`encode_diff` reaches 2,171 features on 433 edges. The edge count is lower than the
parser targets because the encoder is a smaller body of code; the feature count is the
highest of the four, which is what a target that varies both the calls and the room they
have to work in should look like. Adding the writer path took it from 393 edges to 433.

Two clean runs before a real bug is the whole argument for running it longer than the
minimum. Sixty seconds of differential fuzzing is enough to claim you did it. It is not
enough to find anything past the shallow water. The bug that was there took seven
minutes and four hundred thousand executions to surface, and both earlier runs had
already reported success.

The run after the fix went 3.6x further than the one that found the bug, on a corpus
the earlier runs had grown, and stayed clean. Every later entry is a re-verification
after a change to the parser, because a change that touches how bytes are walked is
exactly what this is for.

## What it found

### The printer's missing type

A 1,220-byte input of deeply nested maps and tags. This port returned
`CborErrorUnexpectedEOF` (257); upstream returned `CborErrorUnknownType` (259).

The parser was not the problem. Running `cbor_value_advance` over the same bytes gave
the identical error on both sides. The difference was in `value_to_pretty`, which had
no arm for `CborInvalidType` and fell through to a catch-all that reported the input
had run out. Upstream has an arm for it:

```c
case CborInvalidType:
    err = stream(out, "invalid");
    if (err)
        return err;
    return CborErrorUnknownType;
```

It prints the word `invalid` and reports the *type*, which is the more accurate claim:
the input has not necessarily run out, the item is simply not there. The case is
reachable because `container_to_pretty` checks for the end of a map before rendering
the key and then renders the value unconditionally, so a map that runs out between a
key and its value hands `value_to_pretty` a cursor whose type is `Invalid`.

Getting there needs about 1,024 levels of nesting, which is why no small input
reproduces it. `cargo fuzz tmin` could not shrink the case below 1,199 bytes, and a
brute-force search over every input up to five bytes found nothing. It is exactly the
kind of case a hand-written test suite does not contain.

Reading the same function to fix it turned up a second, quieter gap: upstream calls
`copy_current_position(it, &recursed)` on both error exits from the array/map case, so
a caller inspecting its cursor after a failure sees where the descent actually stopped.
This port returned the error and left the cursor on the opening bracket. Same error
code either way, and nothing would ever have caught it, but the state left behind was
wrong.

Both are fixed, and the input is now a permanent fixture under `tests/port/corpus/`,
replayed by `make test` in a couple of seconds rather than waiting on libFuzzer to
rediscover it.

Is the same arm missing anywhere else? In the library, no. `cbortojson.c` and
`cborvalidation.c` have the same `case CborInvalidType: return CborErrorUnknownType;`,
and this port's `tojson.rs` and `validation.rs` already had it. The pretty printer was
the one that did not. And, it later turned out, so was the *second* pretty printer in
`tools/cbordump/`, which is a separate rewrite nothing was checking. That is
[decision 18](../reference/decisions.md).

### The encoder's off-by-one

`encode_diff` found one at 2,653 executions, on its first run.

`cbor_encode_simple_value` rejected the range 24..=31 as reserved. Upstream's guard is
`value >= HalfPrecisionFloat && value <= Break`, and `HalfPrecisionFloat` is 25:

```c
if (value >= HalfPrecisionFloat && value <= Break)
    return CborErrorIllegalSimpleType;
```

24 is the escape byte that introduces a two-byte simple value, so upstream accepts it
and writes `f8 18`, which upstream's own *parser* then refuses, as a value under 32
written in two bytes. The encoder writes what the parser will not read.

This port now does the same, for the reason [decision 17](../reference/decisions.md)
gives about the UTF-8 bug: the claim being made here is equivalence with a specific
commit, and a port that quietly fixed upstream's asymmetries would make its own
differential fuzzer meaningless.

What is worth noticing is *why* it survived. `tst_encoder` has 1,596 rows and passes
every one of them. None of them asks for simple value 24. A hand-written suite tests the
values a person thought of, and 24 sits in the gap between "obviously fine" and
"obviously reserved", which is exactly the shape of thing a fuzzer walks into in the
first four seconds and a person does not write down in an afternoon.

## Running it

```console
$ ./fuzz/run.sh              # 60 seconds, stops at the first divergence
$ ./fuzz/run.sh 900          # 15 minutes
$ KEEP_GOING=1 ./fuzz/run.sh # keep going past divergences, collect them all
```

Everything is teed to `fuzz/log.txt`. A divergence leaves a reproducer in
`fuzz/artifacts/pretty_diff/` and exits non-zero.

## Where the totals come from

Each run overwrites its target's log, so `fuzz/log*.txt` only ever shows the most recent
run of each. That is fine for reading the last result and useless for anyone checking a
number like "13.5M executions", which sums runs whose logs are gone.

So `fuzz/history.tsv` is the ledger. One row per run, with the commit it ran against:

```
date        target       seconds  execs    commit   result
2026-08-01  pretty_diff  901      1507421  c60944c  clean
2026-08-02  encode_diff  901      1644600  f8077e0  clean
```

`run.sh` appends a row when it finishes, so the file stays true without anyone
maintaining it. The published totals are that file added up:

```console
$ awk -F'\t' 'NR>6 {e+=$4; s+=$3} END {print e, s}' fuzz/history.tsv
13457075 8054
```

Fourteen runs, 13,457,075 executions, 8,054 seconds, nothing outstanding. The two
divergences above are not rows in it: the runs that found them had their logs replaced by
the commits that fixed them, which is exactly the gap the ledger exists to close. Their
reproducers are in `tests/port/corpus/` instead, which is the stronger evidence anyway.

---

*Verified 2026-08-02. Re-run any of it with `TARGET=<name> ./fuzz/run.sh <seconds>`.*
