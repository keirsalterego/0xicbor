# 0xicbor

**A port of [intel/tinycbor](https://github.com/intel/tinycbor) from C to Rust, linkable as
a drop-in `libtinycbor.a`.**

tinycbor is Intel's CBOR ([RFC 8949](https://www.rfc-editor.org/rfc/rfc8949)) implementation:
roughly 6,500 lines of C, aimed at microcontrollers, allocation-free on the common path.
This is that library rewritten in Rust with the C ABI preserved byte for byte, so existing
callers do not notice the swap.

```console
$ make
$ cc -Icrates/cbor-ffi/include yours.c -o yours \
     target/release/libtinycbor.a -lm -lpthread -ldl
```

## The claim, and how to check it

Any port can say it works. The check here is that upstream's own test suite (Qt/C++) is
compiled and linked against the Rust library **with no edits**, and whatever it reports is
the score.

The suite lives in `tests/original/`, copied verbatim from commit `9441b2ca`, with SHA-256
hashes pinned at kickoff:

```console
$ cd tests/original && sha256sum -c hashes.txt
./CMakeLists.txt: OK
./parser/tst_parser.cpp: OK
...
```

If a test cannot pass, it stays failing and gets a paragraph in the
[decision log](reference/decisions.md). A 94% pass rate you can reproduce beats a 100%
claim you cannot.

## Status

Real output of `make test` and `bench/run.py`, not targets.

| | |
|---|---|
| Original suite | **4,929 / 4,929, zero failures** |
| Symbol parity | **44 / 44**, zero `nm` diff against upstream |
| ABI layout | asserted against C-dumped offsets, passing |
| `unsafe` blocks | **80**, all in the shim; `cbor-core` at zero |
| Differential fuzz | **13.5M execs, zero divergences**, four targets |
| Tools vs upstream | **exact**, 4,509 documents by 20 flag combinations |
| Dependencies | **zero** |
| Speed vs C | **3.4x faster** (mean p50 over 16 measurements) |

Every row that can apply to a port passes. The two that cannot are `tst_cpp`, which
`#include`s upstream's `.c` files and therefore tests C sources this port does not have.

Parsing is 2.7x faster than the C on every corpus file and printing 4.4x. It was 1.49x
*slower* on all eight when it was a literal transliteration, and every figure since is kept
in the benchmark's history table rather than quietly replaced. The one measurement the C
still wins is process startup, published as prominently as the rest. See the
[scoreboard](verification/scoreboard.md).

Seven bugs have been found in this port and fixed. Two by the fuzzer, five by tests written
for code that nothing else reached. They are written up rather than summarised away, because
the interesting question about a port is not whether it had bugs but how they were caught.
One bug was found in **upstream** and filed:
[intel/tinycbor#331](https://github.com/intel/tinycbor/issues/331).

## Where to start

**You want to use it.** [Using the library](using/index.md) has a program that compiles and
its real output, then [encoding](using/encoding.md) and [parsing](using/parsing.md).

**You want to know whether to trust it.** The [scoreboard](verification/scoreboard.md) is
every number in one place, and [differential fuzzing](verification/differential-fuzzing.md)
is the honest version, including what it found.

**You want to know how it was built.** [Why an ABI shim](architecture/abi-shim.md) is the
one architectural decision everything else follows from, and
[where the C ends](architecture/the-c-question.md) is precisely what is and is not C here,
because "no C" is a claim worth being able to verify.

**You are porting something yourself.** The [cookbook](cookbook/index.md) is the parts that
generalise: getting to a red test loop, matching an ABI, matching `printf` exactly, and what
to do when the C is faster and you cannot see why.

---

*Verified 2026-08-02, against upstream commit `9441b2ca`, on Linux x86-64.*
