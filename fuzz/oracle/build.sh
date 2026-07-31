#!/bin/sh
# Builds cbor-oracle against the upstream C tinycbor checkout.
#
# The oracle is the only C in this repository and it is never linked into the
# shipped Rust library -- see the header comment in cbor-oracle.c. This script
# exists so that separation is enforced by the build, not by convention: it
# reaches into the upstream tree directly and produces a standalone executable.
#
# Override the checkout with TINYCBOR=/path/to/tinycbor.

set -eu

TINYCBOR="${TINYCBOR:-$HOME/tinycbor-upstream}"
CC="${CC:-cc}"
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

lib="$TINYCBOR/build-ref/libtinycbor.a"
[ -f "$lib" ] || { echo "no upstream library at $lib" >&2; exit 1; }

$CC -O1 -g -std=c99 -Wall -Wextra \
    -I"$TINYCBOR/src" -I"$TINYCBOR/build-ref" \
    "$here/cbor-oracle.c" -o "$here/cbor-oracle" \
    "$lib" -lm

echo "built $here/cbor-oracle"
