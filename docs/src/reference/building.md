# Building

One command. It builds the static library and the original test binaries.

```console
$ make
```

## Requirements

- A Rust toolchain (2021 edition, `offset_of!` needs 1.77 or newer)
- A C++ compiler with C++20, for upstream's Qt tests
- Qt6 Test, discovered through `pkg-config`
- `moc`, from the same Qt installation

Nothing else. The Rust side has no dependencies at all, which you can check the same way
anyone else would:

```console
$ cargo tree
```

Qt is only needed to *run upstream's tests*. If all you want is the library,
`cargo build --release` produces `libtinycbor.a` on a machine with no Qt at all.

## Targets

| | |
|---|---|
| `make` | library plus the original test binaries |
| `make test` | the original suite plus this port's own differential tests |
| `make test-tools` | `cbordump` and `json2cbor` against upstream's binaries |
| `make symbols` | diff exported symbols against upstream's library |
| `make lint` | `clippy` with warnings denied, plus the unsafe budget check |
| `make fmt` | `rustfmt --check` |
| `make fuzz` | differential fuzz; `DURATION=900 TARGET=encode_diff` to pick |
| `make bench` | rebuild the drivers and rewrite `bench/results.json` |
| `make clean` | remove `build/` and `target/` |

`make test` needs Qt6. Everything else runs from a fresh clone with nothing beside it:
`make bench` and `make fuzz` both fall back to `bench/reference/libtinycbor-upstream.a`,
which is committed, so the C side of every comparison is already here.

`make test-tools` is the exception. It compares against upstream's `cbordump` and
`json2cbor` *binaries*, and binaries are not vendored, so without a checkout it says so and
skips rather than fails. [Troubleshooting](troubleshooting.md) has the exact commands if you
want that comparison too.

## Overriding tools

```console
$ make CXX=clang++ MOC=/usr/lib/qt6/libexec/moc
```

## Verifying the suite is untouched

```console
$ cd tests/original && sha256sum -c hashes.txt
```

This should stay clean after a full build. Generated `moc` output goes into `build/`,
never next to the test sources.

## Fresh clone

The repository builds without tinycbor checked out beside it. The C headers the tests
compile against are vendored, and upstream's reference library in `bench/reference/` is only
a comparison target.

```console
$ git clone https://github.com/keirsalterego/0xicbor && cd 0xicbor && make test
```

## Next steps

- [Using the library](../using/index.md) once it has built.
- [Troubleshooting](troubleshooting.md) if it has not.
- [Contributing](contributing.md) before you change anything.

---

*Verified 2026-08-02 on Debian-based Linux x86-64, Qt 6, rustc 1.97.*
