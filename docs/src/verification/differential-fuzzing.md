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
driven as a **subprocess**: bytes in on stdin, diagnostic-notation rendering on stdout,
the `CborError` on stderr. Nothing about it is linked into `libtinycbor.a`. The cost is
roughly 2,000 executions per second instead of tens of thousands, which is a real
price, and it is worth paying to keep the claim checkable. This is
[decision 5](../reference/decisions.md).

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

Two clean runs before a real bug is the whole argument for running it longer than the
minimum. Sixty seconds of differential fuzzing is enough to claim you did it. It is not
enough to find anything past the shallow water. The bug that was there took seven
minutes and four hundred thousand executions to surface, and both earlier runs had
already reported success.

The final run went 3.6x further than the one that found the bug, on a corpus the
earlier runs had grown, and stayed clean.

## What it found

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

Is the same arm missing anywhere else? No. `cbortojson.c` and `cborvalidation.c` have
the same `case CborInvalidType: return CborErrorUnknownType;`, and this port's
`tojson.rs` and `validation.rs` already had it. The pretty printer was the one that
did not.

## Running it

```console
$ ./fuzz/run.sh              # 60 seconds, stops at the first divergence
$ ./fuzz/run.sh 900          # 15 minutes
$ KEEP_GOING=1 ./fuzz/run.sh # keep going past divergences, collect them all
```

Everything is teed to `fuzz/log.txt`. A divergence leaves a reproducer in
`fuzz/artifacts/pretty_diff/` and exits non-zero.
