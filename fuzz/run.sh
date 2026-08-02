#!/usr/bin/env bash
# Runs the differential fuzzer for a while and tees everything to fuzz/log.txt.
#
#   ./fuzz/run.sh                    # 60 seconds, stops at the first divergence
#   ./fuzz/run.sh 900                # 15 minutes
#   TARGET=json_diff ./fuzz/run.sh   # the JSON converter instead of the printer
#   TARGET=encode_diff ./fuzz/run.sh # the encoder, driven by a call program
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
#
# encode_diff is not fed CBOR at all -- its input is a program of encoder calls
# -- so it gets its own seeds: a two-byte output buffer size, then opcodes. The
# sizes are deliberately small, because the encoder's overrun bookkeeping is the
# part of it worth reaching.
mkdir -p "$corpus"
if [ "$TARGET" = encode_diff ] && [ -z "$(ls -A "$corpus")" ]; then
  i=0
  # One of each opcode; a buffer too small for anything; nesting; an
  # unbalanced close; indefinite containers; a close_container_checked that
  # cannot be satisfied.
  for s in '\x40\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00\x11\x12' \
           '\x03\x00\x05\x0a0123456789' \
           '\x00\x00\x11\x12\x11' \
           '\x20\x00\x0c\xff\x11\x0d\xff\x12\x12\x0e\x0e' \
           '\x20\x00\x0e\x0f\x0c\x05\x11\x0f' \
           '\x40\x00\x0c\x01\x0c\x01\x0c\x01\x11\x0e\x0e\x0e' \
           '\x10\x00\x06\x20AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA' \
           '\x08\x00\x08\x00\x00\x00\x00\x00\x00\xf0\x7f' \
           '\x08\x00\x0a\x00\x00\x80\x3f' \
           '\x08\x00\x03\xc8' '\x40\x00\x0b\x03\x83\x01\x02'; do
    printf '%b' "$s" > "$corpus/seed$i"
    i=$((i + 1))
  done
  echo "seeded $i programs into $corpus"
fi

# Programs a run already found something with. fuzz/corpus/ is not tracked, so
# these live in the repo and are copied in on every run rather than waiting for
# libFuzzer to rediscover them. Unconditional, so a corpus that already has
# files still gets them back.
if [ "$TARGET" = encode_diff ]; then
  for f in "$here"/../tests/port/corpus/*.encprog; do
    [ -e "$f" ] && cp "$f" "$corpus/$(basename "$f")"
  done
elif [ -z "$(ls -A "$corpus")" ]; then
  i=0
  flags=('' '' '' '')
  case "$TARGET" in
    json_diff) flags=('\x00' '\x01' '\x04' '\x0b') ;;
    # Little-endian u32: Basic, CanonicalFormat, StrictMode, Strictest.
    validate_diff) flags=('\x00\x00\x00\x00' '\xff\x0f\x00\x00' \
                          '\x00\xff\x0f\x00' '\xff\xff\xff\xff') ;;
  esac
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

# This overwrites $log, so without a row here every earlier run is invisible in a
# checkout and the totals in readme.md cannot be checked against anything.
done_line=$(grep -oE 'Done [0-9]+ runs in [0-9]+ second' "$log" | tail -1)
if [ -n "$done_line" ]; then
  [ "$status" -eq 0 ] && result=clean || result=divergence
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$(date -u +%F)" "$TARGET" "$(echo "$done_line" | awk '{print $5}')" \
    "$(echo "$done_line" | awk '{print $2}')" "$(git -C "$here" rev-parse --short HEAD)" \
    "$result" >> "$here/history.tsv"
fi

exit "$status"
