#!/usr/bin/env bash
# Runs the differential fuzzer for a while and tees everything to fuzz/log.txt.
#
#   ./fuzz/run.sh                    # 60 seconds, stops at the first divergence
#   ./fuzz/run.sh 900                # 15 minutes
#   TARGET=json_diff ./fuzz/run.sh   # the JSON converter instead of the printer
#   KEEP_GOING=1 ./fuzz/run.sh       # keep fuzzing past divergences, collect them all
#   TINYCBOR=/elsewhere ./fuzz/run.sh
#
# Each target keeps its own corpus and log. pretty_diff, the default, writes
# log.txt; anything else writes log-<target>.txt.
#
# KEEP_GOING is what you want once a known divergence is outstanding: without
# it the run ends after roughly a second and finds nothing new behind the one
# it already knows about.
#
# Exits non-zero if a divergence (or any crash) was found; the reproducer lands
# in fuzz/artifacts/pretty_diff/ and replays with
#
#   cargo +nightly fuzz run pretty_diff fuzz/artifacts/pretty_diff/<file>

set -euo pipefail

DURATION="${1:-60}"
TARGET="${TARGET:-pretty_diff}"
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
if [ "$TARGET" = pretty_diff ]; then log="$here/log.txt"; else log="$here/log-$TARGET.txt"; fi
corpus="$here/corpus/$TARGET"

# The oracle is upstream C in its own executable, never linked into the Rust
# library. Rebuild it if it is missing or older than its source.
if [ ! -x "$here/oracle/cbor-oracle" ] ||
   [ "$here/oracle/cbor-oracle.c" -nt "$here/oracle/cbor-oracle" ]; then
  "$here/oracle/build.sh"
fi

# libFuzzer will find these on its own, but not inside a minute. Each is a
# distinct corner of the renderer: major types, tags, floats, indefinite
# lengths, a stray break, invalid UTF-8.
#
# json_diff takes its CborToJsonFlags from the first byte of the input, so its
# seeds carry one. Four of them, cycled, so the corpus starts with every flag
# combination represented rather than only the default.
mkdir -p "$corpus"
if [ -z "$(ls -A "$corpus")" ]; then
  i=0
  flags=('' '' '' '')
  if [ "$TARGET" = json_diff ]; then flags=('\x00' '\x01' '\x04' '\x0b'); fi
  for s in '\x00' '\x20' '\x18\xff' '\x3b\xff\xff\xff\xff\xff\xff\xff\xff' \
           '\x43\x01\x02\x03' '\x63\x61\x62\x63' '\x83\x01\x02\x03' \
           '\xa2\x61\x61\x01\x61\x62\x02' '\x9f\x01\x02\xff' \
           '\x5f\x42\x01\x02\x43\x03\x04\x05\xff' '\xbf\x61\x61\xf5\xff' \
           '\xc1\x1a\x51\x4b\x67\xb0' '\xd8\x18\x45\x64\x49\x45\x54\x46' \
           '\xf4' '\xf6' '\xf7' '\xf8\xff' '\xf9\x7c\x00' '\xfa\x47\xc3\x50\x00' \
           '\xfb\x3f\xf1\x99\x99\x99\x99\x99\x9a' '\xff' '\x62\xc3\x28' '\x1c'; do
    printf '%b%b' "${flags[$((i % 4))]}" "$s" > "$corpus/seed$i"
    i=$((i + 1))
  done
  echo "seeded $i inputs into $corpus"
fi

extra=()
if [ -n "${KEEP_GOING:-}" ]; then
  # Fork mode runs each batch in a child, so a divergence kills the child and
  # the parent carries on. Every distinct one lands in fuzz/artifacts/.
  extra=(-fork=1 -ignore_crashes=1)
fi

cd "$here"
echo "== differential fuzz: $TARGET vs upstream C oracle, ${DURATION}s ==" | tee "$log"
status=0
cargo +nightly fuzz run "$TARGET" -- \
  "-max_total_time=$DURATION" -print_final_stats=1 "${extra[@]}" 2>&1 | tee -a "$log" || status=$?

if [ "$status" -ne 0 ]; then
  echo "== DIVERGENCE OR CRASH -- see $log and fuzz/artifacts/$TARGET/ ==" | tee -a "$log"
fi
exit "$status"
