# Thin wrapper over the makefile. The makefile is the real build — it is what a
# stranger with no `just` installed will run, and the rule is that one command
# builds everything. This exists because typing `just test` is nicer.

# Build the static library, the tools, and the original test binaries.
default: build

build:
    @make all

# Run upstream's Qt suite against our libtinycbor.a and print pass/fail per binary.
test:
    @make test

# Diff our exported ABI against upstream's. Empty output is the goal.
symbols:
    @make symbols

# Everything a commit has to pass.
check:
    @make fmt
    @make lint
    cargo test --workspace
    @make test

# Prove tests/original/ is byte-for-byte upstream's.
verify-tests:
    cd tests/original && sha256sum -c hashes.txt

# Differential fuzz against the out-of-process C oracle.
fuzz duration="60":
    @make fuzz DURATION={{duration}}

bench:
    @make bench

clean:
    @make clean
