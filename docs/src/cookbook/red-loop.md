# Step 1: get to a red test loop

The goal of the first few hours is not working code. It is a command that runs the
original test suite against your empty library and prints a real number.

Once that exists, the rest of the project is turning red into green, and you always
know exactly where you are. Without it, you write code for two days and then find out
whether any of it works.

## Pin the tests before you touch anything

Copy the upstream test directory in verbatim, then record what you copied:

```console
$ cp -a ../tinycbor-upstream/tests/. tests/original/
$ cd tests/original
$ find . -type f -not -name hashes.txt | sort | xargs sha256sum > hashes.txt
```

Now `sha256sum -c hashes.txt` proves at any moment that you did not quietly "fix" a
test that was inconvenient. Run it at the end. Judges, reviewers, and future-you all
care about the difference between a suite you passed and a suite you edited.

Two details that matter more than they look:

- Generated files must land somewhere else. Qt's `moc` wants to write next to the
  source; point it at `build/` instead, or your hash check fails on your own output.
- Do this **first**. Pinning hashes after you have been working for a day proves
  nothing about the day you already had.

## Find out what "all tests pass" even means

Build upstream once, unmodified, and run its own suite. That number is your ceiling:

```console
$ tst_encoder   1596 passed
$ tst_parser    2506 passed
$ tst_tojson     827 passed
$ tst_cpp          2 passed
                ————
                4931
```

Then read the tests and find the ones that *cannot* apply to a port. In this project
`tst_cpp.cpp` opens with:

```cpp
#include "../../src/cborencoder.c"
#include "../../src/cborparser.c"
```

That is not a test of the library. It is a test that the C sources compile as C++. A
Rust port has no `.c` files for it to include, so the honest ceiling is 4,929, not
4,931, and the two missing rows get written down as a decision rather than silently
dropped.

Find these on day one. Discovering at hour 60 that your target was never reachable is
a bad hour.

## Link against nothing

Create the library crate and export every public symbol as a stub. Do not implement
anything. The point is to make the linker happy:

```console
$ nm -g --defined-only libtinycbor.a | wc -l
44
```

Then make the test binaries compile against your headers and link against your
archive. When `make test` runs and prints failures, you are done with step one.

**One thing worth getting right:** make the stubs *return an error*, not panic. A
staticlib built with `panic = "abort"` will kill the test process on the very first
call, and your baseline becomes "it crashed" instead of a number you can watch move.

```rust
// Not unimplemented!(): that ends the process and the baseline with it.
const STUB: c_int = c_int::MAX; // CborErrorInternalError
```

## The number you are looking for

```
tst_encoder      2 passed   1594 failed
tst_parser      17 passed   2489 failed
tst_tojson       2 passed    825 failed
TOTAL           21 passed   4908 failed   (of 4929)
```

Twenty-one passing with a library that does nothing at all. Those are tests that
expect an error and coincidentally get one. That is the honest floor.

That is a good first day. You have written no logic, and you have a measurement.
