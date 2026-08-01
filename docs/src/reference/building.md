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

Nothing else. The Rust side has no dependencies at all.

## Targets

| | |
|---|---|
| `make` | library plus the four test binaries |
| `make test` | run the original suite, print per-binary pass/fail |
| `make symbols` | diff exported symbols against upstream's library |
| `make lint` | `clippy`, warnings denied |
| `make fmt` | `rustfmt --check` |
| `make clean` | remove `build/` and `target/` |

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
