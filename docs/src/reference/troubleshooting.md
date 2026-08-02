# Troubleshooting

Things that go wrong, in roughly the order people hit them.

## `make` says it cannot find Qt6Test

```
Package Qt6Test was not found in the pkg-config search path
```

The original test suite is Qt/C++, so building it needs Qt6 Test and `moc`. On Debian and
Ubuntu:

```console
$ sudo apt install qt6-base-dev qt6-base-dev-tools pkg-config
```

`moc` moves around between distributions. If the makefile cannot find it, point at it:

```console
$ make MOC=/usr/lib/qt6/libexec/moc
```

The library itself does not need Qt for anything. `cargo build --release` gives you
`libtinycbor.a` with no Qt installed at all. Qt is only there so upstream's tests can run
unmodified, which is the entire premise of the project.

## `make test-tools` says it skipped

```
missing /home/you/tinycbor-upstream/build-test/tools/cbordump
  (skipped: no upstream tools to compare against)
```

That check compares this port's `cbordump` and `json2cbor` against upstream's *binaries*.
Binaries are not vendored the way `bench/reference/libtinycbor-upstream.a` is, so you need
upstream checked out and built:

```console
$ git clone https://github.com/intel/tinycbor ~/tinycbor-upstream
$ cd ~/tinycbor-upstream && git checkout 9441b2ca
$ cmake -B build-test && cmake --build build-test
```

Point somewhere else with `UPSTREAM=/path/to/build/tools`.

A skip is not a failure. The rest of `make test` is unaffected, and a real divergence exits
non-zero and stops the build.

## The fuzzer will not start

```
no oracle at .../fuzz/oracle/cbor-oracle -- run fuzz/oracle/build.sh
```

Same cause: the differential oracle is upstream's C compiled into a standalone binary, and
it needs the checkout above. `fuzz/run.sh` builds it for you if the sources are there.

You also need nightly Rust and `cargo-fuzz`:

```console
$ rustup toolchain install nightly
$ cargo install cargo-fuzz
```

The oracle runs as a **subprocess**, never linked in. That is not an implementation detail
you can optimise away, it is the thing that keeps "this library contains no C" checkable.
See [Where the C ends](../architecture/the-c-question.md).

## `make bench` refuses, or warns about a changed archive

```
!! rust archive changed since link; rerun bench/build.sh to measure the current tree
```

The benchmark records the SHA-256 of both archives when it links its drivers. If you rebuild
the library afterwards, the recorded numbers no longer describe the code you have, and the
harness says so instead of quietly publishing stale figures.

```console
$ ./bench/build.sh && python3 bench/run.py
```

Also: run it on an idle machine. The numbers in the repository were re-taken because a
run went out at load 6.4 on 8 cores, and it was visibly wrong. The recorded load average is
in `results.json` so you can tell whether to believe a given run.

## Linker errors about `pthread_*` or `dlsym`

```
undefined reference to `pthread_mutex_lock'
```

Add `-lpthread -ldl`. Linking a Rust `staticlib` pulls in the Rust standard library, which
needs both. This is the one thing that is genuinely different from linking the C, and it is
not optional.

## The suite passes but `sha256sum -c` fails

Something wrote into `tests/original/`. Nothing in this repository is supposed to: `moc`
output goes into `build/`, and the makefile is set up that way deliberately.

```console
$ git status tests/original
$ git checkout tests/original
```

If it keeps happening, you are probably running upstream's CMake inside the checkout rather
than this repository's makefile.

## `cbor_value_*` returns garbage, or crashes

Check the lifetimes first. `CborValue` holds a pointer back to the `CborParser` it came
from, and nothing enforces that the parser outlives it. A parser declared in an inner scope
and a value used after that scope closes is the classic version, and it will usually appear
to work for a while.

This is inherited from the C API and cannot be fixed without changing the ABI, which is the
one thing that cannot change. [Decision 21](decisions.md) says so at more length.

## Everything builds but the numbers do not match the docs

Fair. Every figure in these docs comes from a run on one laptop, and the pages that carry
numbers say when they were verified. Re-run and trust your own output:

```console
$ make test        # 4,929 / 4,929
$ make symbols     # 44/44, zero diff
$ make lint        # includes the unsafe budget count
$ python3 bench/run.py
```

If your numbers differ and you think the docs are wrong rather than your machine, that is
worth an issue.

## Next steps

- [Building](building.md) for the full target list.
- [Contributing](contributing.md) if you are about to change something.
