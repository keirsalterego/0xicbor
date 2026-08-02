#!/bin/sh
# Builds cbor-oracle against the upstream C tinycbor checkout.
#
# The oracle is the only C in this repository and it is never linked into the
# shipped Rust library -- see the header comment in cbor-oracle.c. This script
# exists so that separation is enforced by the build, not by convention: it
# reaches into the upstream tree directly and produces a standalone executable.
#
# Override the checkout with TINYCBOR=/path/to/tinycbor.
#
# Without one it falls back to what the repository already carries: the archive
# under bench/reference/ and upstream's headers vendored in cbor-ffi. The oracle
# only includes <cbor.h> and <cborjson.h>, both of which are there, so a fresh
# clone can run the differential fuzzer without cloning tinycbor as well. That
# matters more than it looks: a reader who cannot run the fuzzer has to take
# fuzz/log.txt on trust.
#
# The checkout still wins when it is present, because pointing this at a
# different upstream commit is the only way to fuzz against one.

set -eu

TINYCBOR="${TINYCBOR:-$HOME/tinycbor-upstream}"
CC="${CC:-cc}"
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)

lib="$TINYCBOR/build-ref/libtinycbor.a"
if [ -f "$lib" ]; then
    incs="-I$TINYCBOR/src -I$TINYCBOR/build-ref"
    from="checkout at $TINYCBOR"
else
    lib="$root/bench/reference/libtinycbor-upstream.a"
    incs="-I$root/crates/cbor-ffi/include"
    from="committed reference archive"
    [ -f "$lib" ] || { echo "no upstream library at $lib" >&2; exit 1; }
fi

# shellcheck disable=SC2086
$CC -O1 -g -std=c99 -Wall -Wextra $incs \
    "$here/cbor-oracle.c" -o "$here/cbor-oracle" \
    "$lib" -lm

echo "built $here/cbor-oracle ($from)"
