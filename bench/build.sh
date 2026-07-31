#!/usr/bin/env bash
# Build both benchmark drivers from the one driver.c.
#
#   bench/build.sh
#
# Deliberately not wired into the root makefile: the benchmark is a side
# artefact, not part of `make` or `make test`.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
bench=$root/bench
out=$bench/build
inc=$root/crates/cbor-ffi/include
upstream_inc=${TINYCBOR_UPSTREAM:-/home/keir/tinycbor-upstream}/src

lib_c=$bench/reference/libtinycbor-upstream.a
lib_rust=$root/target/release/libtinycbor.a

CC=${CC:-cc}
CFLAGS=${CFLAGS:--O2 -Wall -Wextra -std=c99}

# The claim under test is that one header + one .c file links against either
# archive.  If our header has drifted from upstream's the comparison is not
# apples to apples, so refuse to build rather than publish a quiet lie.
for h in cbor.h cborjson.h; do
    if [ -f "$upstream_inc/$h" ] && ! cmp -s "$inc/$h" "$upstream_inc/$h"; then
        echo "bench/build.sh: $h differs from upstream -- drivers would not be comparable" >&2
        exit 1
    fi
done

if [ ! -f "$lib_rust" ]; then
    echo "bench/build.sh: $lib_rust missing; run 'cargo build --release' first" >&2
    exit 1
fi

mkdir -p "$out"

# Rust staticlibs need libc's threading/dl/math bits; the C archive needs -lm.
# shellcheck disable=SC2086
$CC $CFLAGS -I"$inc" "$bench/driver.c" -o "$out/driver-c"    "$lib_c"    -lm
# shellcheck disable=SC2086
$CC $CFLAGS -I"$inc" "$bench/driver.c" -o "$out/driver-rust" "$lib_rust" -lm -lpthread -ldl

# The Rust archive is a moving target while the port is in progress, and the
# drivers link it statically -- so the bytes that ended up inside driver-rust are
# the only ones a result can honestly be attributed to. Record them at link time;
# run.py copies this into results.json rather than stat-ing the archive later and
# possibly fingerprinting a build that landed after the drivers were made.
{
    printf '{\n'
    printf '  "linked_at_utc": "%s",\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf '  "c_reference": {"path": "%s", "bytes": %s, "sha256": "%s"},\n' \
        "$lib_c" "$(stat -c %s "$lib_c")" "$(sha256sum "$lib_c" | cut -d' ' -f1)"
    printf '  "rust": {"path": "%s", "bytes": %s, "sha256": "%s"}\n' \
        "$lib_rust" "$(stat -c %s "$lib_rust")" "$(sha256sum "$lib_rust" | cut -d' ' -f1)"
    printf '}\n'
} > "$out/link-manifest.json"

echo "built $out/driver-c and $out/driver-rust"
