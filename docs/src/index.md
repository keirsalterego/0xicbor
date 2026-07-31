# 0xicbor

**A port of [intel/tinycbor](https://github.com/intel/tinycbor) from C to Rust, linkable as
a drop-in `libtinycbor.a`.**

tinycbor is Intel's CBOR ([RFC 8949](https://www.rfc-editor.org/rfc/rfc8949)) implementation:
roughly 6,500 lines of C, aimed at microcontrollers, allocation-free on the common path.
This is that library rewritten in Rust with the C ABI preserved byte for byte, so existing
callers do not notice the swap.

## The claim, and how to check it

Any port can say it works. The check here is that upstream's own test suite — Qt/C++,
4,931 assertions — is compiled and linked against the Rust library **with no edits**, and
whatever it reports is the score.

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

These are the real numbers from `make test`, not a target.

| | |
|---|---|
| Original suite | **2,732 passed, 2,197 failed** of 4,929 reachable rows |
| Symbol parity | **44 / 44**, zero `nm` diff against upstream |
| ABI layout | asserted against C-dumped offsets, passing |
| `unsafe` blocks | **46** |
| Differential fuzz | not yet run |

The port is in progress. The encoder passes its suite outright; the parser and the
diagnostic printer work. The JSON converter is not written yet, which is nearly all of the
remaining red. The failure count is the progress bar.

## Where to start

- [Why an ABI shim](architecture/abi-shim.md) — the one architectural decision everything
  else follows from.
- [Where the C ends](architecture/the-c-question.md) — precisely what is and is not C in
  this repository, since "no C" is a claim worth being able to verify.
- [Running the original suite](verification/original-suite.md) — how the unmodified Qt
  tests get linked against Rust.
