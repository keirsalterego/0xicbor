# One command: `make` builds the library and the original test suite.
# `make test` runs the suite and prints the per-binary pass/fail counts.
#
# The tests under tests/original/ are upstream's, byte for byte, and are never
# edited or generated into. Everything this makefile produces lands in build/.
#
# Upstream drives its suite from CMake via a tinycbor_add_qtest() helper. We
# compile the same .cpp files directly instead: reproducing upstream's CMake
# package would mean carrying a chunk of upstream's build system, and the only
# thing the helper does that matters here is run moc and link Qt Test.

CARGO      ?= cargo
MOC        ?= /usr/lib/qt6/libexec/moc
PKG_CONFIG ?= pkg-config

BUILD   := build
LIB     := target/release/libtinycbor.a
ORIG    := tests/original
PORT    := tests/port
INCLUDE := crates/cbor-ffi/include
REF     := bench/reference/libtinycbor-upstream.a

QT_CFLAGS := $(shell $(PKG_CONFIG) --cflags Qt6Test)
QT_LIBS   := $(shell $(PKG_CONFIG) --libs Qt6Test)

CXXFLAGS := -std=c++20 -O2 -I$(INCLUDE) $(QT_CFLAGS)

# tst_cpp is deliberately absent: it #includes upstream's seven .c files
# directly, so it tests C sources we do not have. See decisions.md.
QTESTS := encoder parser tojson

.PHONY: all lib test test-port test-tools clean fmt lint symbols fuzz bench

all: lib $(QTESTS:%=$(BUILD)/tst_%) $(BUILD)/tst_c90

lib: $(LIB)

$(LIB):
	$(CARGO) build --release

# Each Qt test needs its moc output next to it on the include path. The .moc is
# written into build/ so tests/original/ stays read-only.
.SECONDEXPANSION:
$(BUILD)/tst_%: $(ORIG)/$$*/tst_%.cpp $(LIB)
	@mkdir -p $(BUILD)
	$(MOC) $(QT_CFLAGS) -I$(INCLUDE) $< -o $(BUILD)/tst_$*.moc
	$(CXX) $(CXXFLAGS) -I$(BUILD) -I$(ORIG)/$* $< -o $@ $(LIB) $(QT_LIBS)

# Compile-only: proves the header is still valid C90, which is a property of
# the port even though no Rust runs.
$(BUILD)/tst_c90: $(ORIG)/c90/tst_c90.c $(LIB)
	@mkdir -p $(BUILD)
	$(CC) -std=c90 -pedantic -Wall -I$(INCLUDE) $< -o $@ $(LIB) -lm

# The one entry point upstream's suite never calls, so it gets a test of ours.
# The same source is built twice, once against each archive, and the two
# transcripts are diffed -- so there is no expected-output file to drift.
$(BUILD)/tst_dup_string-rust: $(PORT)/tst_dup_string.c $(LIB)
	@mkdir -p $(BUILD)
	$(CC) -std=c99 -Wall -Wextra -I$(INCLUDE) $< -o $@ $(LIB) -lm -lpthread -ldl

$(BUILD)/tst_dup_string-c: $(PORT)/tst_dup_string.c $(REF)
	@mkdir -p $(BUILD)
	$(CC) -std=c99 -Wall -Wextra -I$(INCLUDE) $< -o $@ $(REF) -lm

# Inputs the differential fuzzer found, replayed on every run so they stay
# found without waiting for libFuzzer to rediscover them.
CORPUS := $(wildcard $(PORT)/corpus/*.cbor)

$(BUILD)/tst_pretty_diff-rust: $(PORT)/tst_pretty_diff.c $(LIB)
	@mkdir -p $(BUILD)
	$(CC) -std=gnu99 -Wall -Wextra -I$(INCLUDE) $< -o $@ $(LIB) -lm -lpthread -ldl

$(BUILD)/tst_pretty_diff-c: $(PORT)/tst_pretty_diff.c $(REF)
	@mkdir -p $(BUILD)
	$(CC) -std=gnu99 -Wall -Wextra -I$(INCLUDE) $< -o $@ $(REF) -lm

$(BUILD)/tst_advance_diff-rust: $(PORT)/tst_advance_diff.c $(LIB)
	@mkdir -p $(BUILD)
	$(CC) -std=gnu99 -Wall -Wextra -I$(INCLUDE) $< -o $@ $(LIB) -lm -lpthread -ldl

$(BUILD)/tst_advance_diff-c: $(PORT)/tst_advance_diff.c $(REF)
	@mkdir -p $(BUILD)
	$(CC) -std=gnu99 -Wall -Wextra -I$(INCLUDE) $< -o $@ $(REF) -lm

$(BUILD)/tst_reader_diff-rust: $(PORT)/tst_reader_diff.c $(LIB)
	@mkdir -p $(BUILD)
	$(CC) -std=gnu99 -Wall -Wextra -I$(INCLUDE) $< -o $@ $(LIB) -lm -lpthread -ldl

$(BUILD)/tst_reader_diff-c: $(PORT)/tst_reader_diff.c $(REF)
	@mkdir -p $(BUILD)
	$(CC) -std=gnu99 -Wall -Wextra -I$(INCLUDE) $< -o $@ $(REF) -lm

test-port: $(BUILD)/tst_dup_string-rust $(BUILD)/tst_dup_string-c \
           $(BUILD)/tst_pretty_diff-rust $(BUILD)/tst_pretty_diff-c \
           $(BUILD)/tst_advance_diff-rust $(BUILD)/tst_advance_diff-c \
           $(BUILD)/tst_reader_diff-rust $(BUILD)/tst_reader_diff-c
	@echo "== port tests: same source, both archives, transcripts diffed =="
	@$(BUILD)/tst_dup_string-c    > $(BUILD)/dup_string-c.out
	@$(BUILD)/tst_dup_string-rust > $(BUILD)/dup_string-rust.out
	@if diff -u $(BUILD)/dup_string-c.out $(BUILD)/dup_string-rust.out; then \
	  printf '  %-12s %5s cases  %5s differ\n' 'dup_string' \
	    "$$(wc -l < $(BUILD)/dup_string-rust.out)" 0; \
	else \
	  echo "  dup_string: DIVERGES FROM UPSTREAM"; exit 1; \
	fi
	@$(BUILD)/tst_pretty_diff-c    $(CORPUS) > $(BUILD)/pretty_diff-c.out
	@$(BUILD)/tst_pretty_diff-rust $(CORPUS) > $(BUILD)/pretty_diff-rust.out
	@if diff -u $(BUILD)/pretty_diff-c.out $(BUILD)/pretty_diff-rust.out; then \
	  printf '  %-12s %5s cases  %5s differ\n' 'pretty_diff' \
	    "$$(wc -l < $(BUILD)/pretty_diff-rust.out)" 0; \
	else \
	  echo "  pretty_diff: DIVERGES FROM UPSTREAM"; exit 1; \
	fi
	@$(BUILD)/tst_advance_diff-c    $(CORPUS) $(wildcard bench/corpus/*.cbor) \
	  > $(BUILD)/advance_diff-c.out
	@$(BUILD)/tst_advance_diff-rust $(CORPUS) $(wildcard bench/corpus/*.cbor) \
	  > $(BUILD)/advance_diff-rust.out
	@if diff -u $(BUILD)/advance_diff-c.out $(BUILD)/advance_diff-rust.out; then \
	  printf '  %-12s %5s cases  %5s differ\n' 'advance_diff' \
	    "$$(wc -l < $(BUILD)/advance_diff-rust.out)" 0; \
	else \
	  echo "  advance_diff: DIVERGES FROM UPSTREAM"; exit 1; \
	fi
	@$(BUILD)/tst_reader_diff-c    $(CORPUS) $(wildcard bench/corpus/*.cbor) \
	  > $(BUILD)/reader_diff-c.out
	@$(BUILD)/tst_reader_diff-rust $(CORPUS) $(wildcard bench/corpus/*.cbor) \
	  > $(BUILD)/reader_diff-rust.out
	@if diff -u $(BUILD)/reader_diff-c.out $(BUILD)/reader_diff-rust.out && \
	    ! grep -q 'same=0' $(BUILD)/reader_diff-rust.out; then \
	  printf '  %-12s %5s cases  %5s differ\n' 'reader_diff' \
	    "$$(wc -l < $(BUILD)/reader_diff-rust.out)" 0; \
	else \
	  echo "  reader_diff: DIVERGES FROM UPSTREAM"; exit 1; \
	fi

# cbordump and json2cbor are rewritten here rather than being C over the
# library, so agreeing with upstream is a claim about the rewrite. Both are
# whole programs, which puts their argument parsing, flag combinations and exit
# codes out of reach of the Qt suite and of the fuzzers alike.
#
# This one needs upstream's tools built, which are binaries and so are not
# vendored the way libtinycbor-upstream.a is. The script exits 2 and says how to
# get them when they are missing, and that is a skip rather than a failure; a
# real divergence exits 1 and stops the build.
test-tools:
	@$(CARGO) build --release -q -p cbordump -p json2cbor
	@$(PORT)/tools_diff.sh; rc=$$?; \
	if [ $$rc -eq 2 ]; then echo "  (skipped: no upstream tools to compare against)"; \
	elif [ $$rc -ne 0 ]; then exit 1; fi

test: all test-port test-tools
	@echo "== original suite, linked against the Rust libtinycbor.a =="
	@total_pass=0; total_fail=0; \
	for t in $(QTESTS); do \
	  line=$$(QT_QPA_PLATFORM=offscreen $(BUILD)/tst_$$t 2>/dev/null | grep '^Totals:'); \
	  p=$$(echo "$$line" | grep -oE '[0-9]+ passed' | grep -oE '[0-9]+'); \
	  f=$$(echo "$$line" | grep -oE '[0-9]+ failed' | grep -oE '[0-9]+'); \
	  printf '  %-12s %5s passed  %5s failed\n' "tst_$$t" "$${p:-0}" "$${f:-0}"; \
	  total_pass=$$((total_pass + $${p:-0})); total_fail=$$((total_fail + $${f:-0})); \
	done; \
	printf '  %-12s %5s passed  %5s failed\n' 'tst_c90' \
	  "$$($(BUILD)/tst_c90 >/dev/null 2>&1 && echo 1 || echo 0)" \
	  "$$($(BUILD)/tst_c90 >/dev/null 2>&1 && echo 0 || echo 1)"; \
	echo "  ---"; \
	echo "  TOTAL        $$total_pass passed  $$total_fail failed   (upstream: 4929)"

# Zero output means the exported ABI matches upstream exactly.
symbols: $(LIB)
	@nm -g --defined-only $(LIB) | awk '$$2 ~ /^[A-TV-Z]$$/ {print $$3}' \
	  | grep -E '^_?cbor_' | sort -u > $(BUILD)/symbols-ours.txt
	@diff bench/reference/symbols-upstream.txt $(BUILD)/symbols-ours.txt \
	  && echo "symbols: 44/44, zero diff"

# Differential fuzz against the out-of-process C oracle. Override the seconds
# with DURATION, and pick the target with TARGET=json_diff, validate_diff or
# encode_diff. Sixty seconds is enough to say you fuzzed and not enough to find
# anything -- both real divergences so far needed longer or a target that had
# never run before.
DURATION ?= 60

fuzz:
	./fuzz/run.sh $(DURATION)

# Rebuild both drivers against the two archives and rewrite bench/results.json.
# Deliberately not part of `make`: it takes minutes and wants a quiet machine.
bench: $(LIB)
	bench/build.sh
	bench/run.py

fmt:
	$(CARGO) fmt --check

lint:
	$(CARGO) clippy --all-targets -- -D warnings
	@$(PORT)/unsafe_budget.py

clean:
	$(CARGO) clean
	rm -rf $(BUILD)
