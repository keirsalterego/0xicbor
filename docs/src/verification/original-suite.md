# Running the original suite

The tests under `tests/original/` are upstream's, byte for byte. They are never edited,
never regenerated into, and their hashes are checked at ship time.

```console
$ cd tests/original && sha256sum -c hashes.txt
./CMakeLists.txt: OK
./c90/tst_c90.c: OK
./cpp/tst_cpp.cpp: OK
./encoder/data.cpp: OK
./encoder/tst_encoder.cpp: OK
./parser/data.cpp: OK
./parser/tst_parser.cpp: OK
./tojson/tst_tojson.cpp: OK
...
```

## How they get linked against Rust

Upstream drives its suite from CMake through a `tinycbor_add_qtest()` helper. Reproducing
that would mean carrying a slice of upstream's build system for no gain — the only things
the helper does that matter here are run `moc` and link Qt Test, which is four lines of
makefile:

```make
$(BUILD)/tst_%: $(ORIG)/$$*/tst_%.cpp $(LIB)
	@mkdir -p $(BUILD)
	$(MOC) $(QT_CFLAGS) -I$(INCLUDE) $< -o $(BUILD)/tst_$*.moc
	$(CXX) $(CXXFLAGS) -I$(BUILD) -I$(ORIG)/$* $< -o $@ $(LIB) $(QT_LIBS)
```

The test sources are inputs and nothing else. `moc` output goes into `build/`, which is why
`tests/original/` still hash-verifies after a full build.

## The test that cannot pass

`tst_cpp.cpp` opens like this:

```cpp
#include "../../src/cborencoder.c"
#include "../../src/cborparser.c"
#include "../../src/cborvalidation.c"
```

It is not a test of the library. It is a test that upstream's C sources compile cleanly as
C++. A Rust port has no `.c` files for it to include, so the test is inapplicable by
construction rather than failing on behaviour.

It contributes 2 rows, which is why the reachable total is **4,929** rather than upstream's
4,931. Leaving it out of the makefile is a presentation choice; the reason is recorded so
the missing rows are not mistaken for a gap.

## The test that survived

`tst_c90.c` includes only `cbor.h` and compiles under `-std=c90 -pedantic -Wall`. It tests
the *header*, which this port still ships, so it remains a live constraint: the vendored
headers have to stay C90-clean.

## What the runner prints

```console
$ make test
== original suite, linked against the Rust libtinycbor.a ==
  tst_encoder   1596 passed      0 failed
  tst_parser    1134 passed   1372 failed
  tst_tojson       2 passed    825 failed
  tst_c90          1 passed      0 failed
  ---
  TOTAL         2732 passed   2197 failed   (upstream: 4929)
```

Per binary, not just a total, because the binaries fail for different reasons and watching
one of them move while the others do not is how you tell which module you actually just
finished.
