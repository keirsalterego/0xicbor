# Contributing

The rules here are unusual, because the project is unusual. It is a port judged against a
test suite it is not allowed to touch, and most of what follows exists to keep that claim
honest.

## The four things that are not negotiable

**`tests/original/` is never edited.** Not to fix a test, not to skip one, not to adjust
whitespace. The hashes are pinned in `tests/original/hashes.txt` and verified at ship. If a
test genuinely cannot pass, it stays failing and gets an entry in
[the decision log](decisions.md). A 94% pass rate you can reproduce beats a 100% claim you
cannot.

**The shipped library contains no C.** No `cc` crate, no `bindgen`, no `build.rs` compiling
anything, nothing under `crates/` linking against upstream. The fuzz oracle is upstream's C
in a separate executable spoken to over a pipe. If you find yourself wanting to link it in
for speed, the answer is no, and [Where the C ends](../architecture/the-c-question.md)
explains why.

**No third-party CBOR crate.** Not `ciborium`, not `serde_cbor`, not `minicbor`, not `half`.
Wrapping an existing crate is not a port. `cbor-core` uses `core` and `alloc` and nothing
else.

**`cbor-core` stays `#![forbid(unsafe_code)]`.** Every `unsafe` block lives in `cbor-ffi`
and carries a `// SAFETY:` line naming the invariant the C caller must uphold. `make lint`
recomputes the count and fails if a block loses its line, so this is enforced rather than
requested.

## Before you open a PR

```console
$ make test     # 4,929 / 4,929, and the port's own differential tests
$ make lint     # clippy with warnings denied, plus the unsafe budget check
$ make fmt      # rustfmt --check
$ make symbols  # 44/44 exported, zero nm diff against upstream
```

All four, green. If you touched the parser or the encoder, also run the fuzzer for longer
than the default minute:

```console
$ TARGET=encode_diff DURATION=900 make fuzz
```

Sixty seconds is enough to say you fuzzed. It is not enough to find anything. Both real
divergences found so far needed either fifteen minutes or a target that had never run
before.

## Changing behaviour

The equivalence claim is against upstream at commit `9441b2ca`. That has a consequence
people find surprising: **if upstream has a bug, this port keeps it.**

Two are documented. `cbor_value_to_json_advance` does not validate UTF-8 although its own
header says it does, which is
[intel/tinycbor#331](https://github.com/intel/tinycbor/issues/331) and
[decision 17](decisions.md). `cbor_encode_simple_value` accepts simple value 24 and emits
`f8 18`, which upstream's own parser then rejects, which is
[decision 22](decisions.md).

Fixing either would turn a passing differential fuzz into a failing one and make the central
claim of the project false. If upstream takes a fix, this follows. Until then, a divergence
would be ours, not theirs.

So: a PR that makes this port *better* than upstream is a PR that has to argue its case in
the decision log first.

## Writing style

The code is read by people, so it is written for them.

- Comments explain **why**, and name the spec section when there is one (`RFC 8949 §3.2.1`).
- A comment that restates the line below it gets deleted.
- Plain engineering voice. No "this function is responsible for", no "robust", no
  "leverages".
- `Result<T, CborError>` and `?`, never a translated errno. Where a C function returns an
  out-parameter plus a status, the Rust one returns the value.

The decision log is the other half of this. Every non-obvious divergence gets an entry **at
the moment you make it**. Reconstructing them later is how you end up with ten empty
bullets, and it shows.

## Commits

Conventional Commits, scoped to the module:

```
fix(parser): leave the cursor where a failed string walk stopped
test(port): differential test for the caller-supplied reader source
docs(decisions): the upstream UTF-8 bug, and why this port keeps it
```

Scopes in use: `parser`, `encoder`, `ffi`, `pretty`, `tojson`, `cbordump`, `json2cbor`,
`port`, `original`, `fuzz`, `bench`, `docs`.

Write the body as though the reader has the diff but not your afternoon. What was wrong,
what it did to a caller, how it was found. The commit log is the best record of the work
this project has.

## Adding a test

Tests for what upstream's suite does not reach go in `tests/port/`, and they follow one
pattern: **the same source is built twice, once against each archive, and the two
transcripts are diffed.**

No expected-output file. An expected-output file is a snapshot of what the code did on the
day you wrote it, and it drifts into being a snapshot of a bug. Diffing against upstream
cannot drift, because upstream is the specification here.

That pattern has found five bugs so far, none of which any fuzzer would have reached, and
[decision 18](decisions.md) is the write-up of the worst of them.

## Next steps

- [Building](building.md) for the target list.
- [Troubleshooting](troubleshooting.md) when a gate fails for environmental reasons.
- [The decision log](decisions.md) before you argue with any of the above.
