#!/usr/bin/env bash
# Runs this port's cbordump and json2cbor against upstream's, on the same
# inputs with the same flags, and diffs stdout, stderr and the exit status.
#
#   tests/port/tools_diff.sh                       # corpus under tests/ and bench/
#   tests/port/tools_diff.sh fuzz/corpus/pretty_diff/*   # anything else
#   UPSTREAM=/path/to/build/tools tools_diff.sh     # a different reference build
#
# The tools are the part of upstream that is a program rather than a library,
# so the original Qt suite does not touch them and neither do the differential
# fuzzers, which call the C ABI directly. They are also where a user first
# meets the port. That leaves the argument parsing, the flag combinations, the
# usage text and the exit codes with no coverage at all, which is what this
# closes.
#
# Both tools are rewritten in Rust here rather than being C that links the
# library, so agreeing with upstream is a claim about the rewrite and not
# something the ABI gets for free.

set -uo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$here/../.." && pwd)
UPSTREAM="${UPSTREAM:-$HOME/tinycbor-upstream/build-test/tools}"

up_dump="$UPSTREAM/cbordump/cbordump"
up_json="$UPSTREAM/json2cbor/json2cbor"
our_dump="$root/target/release/cbordump"
our_json="$root/target/release/json2cbor"

for bin in "$up_dump" "$up_json" "$our_dump" "$our_json"; do
  if [ ! -x "$bin" ]; then
    echo "missing $bin" >&2
    echo "  upstream: cmake+make in \$TINYCBOR, or set UPSTREAM=" >&2
    echo "  ours:     cargo build --release -p cbordump -p json2cbor" >&2
    exit 2
  fi
done

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Every flag the CBOR dump path recognises, and every combination of the four
# that the JSON path does. -M -O -S -U gate separate branches of the converter
# and interact -- -M and -O both add structure, -S and -U both change how a
# non-text value is spelled -- so a run with each set alone would miss the
# pairs.
dump_flags=(-c -cf -cn -cfn -j -jM -jO -jS -jU -jMO -jMS -jMU -jOS -jOU -jSU
            -jMOS -jMOU -jMSU -jOSU -jMOSU)

# Same for the reverse direction; json2cbor has the one flag.
json_flags=('' -M)

# `diff` on the two transcripts is the check. Each line carries the exit status
# and a hash of each stream, so a difference names the file and the flag rather
# than dumping two renderings.
#
# Both streams go to files rather than through a command substitution: json2cbor
# writes CBOR to stdout and cbordump renders NUL bytes, and $(...) drops those.
run_pair() {                    # run_pair BIN FLAG FILE -> "rc out_hash err_hash"
  local rc
  "$1" $2 "$3" > "$work/out" 2> "$work/err"; rc=$?
  printf '%d %s %s' "$rc" \
    "$(sha256sum < "$work/out" | cut -c1-16)" \
    "$(sha256sum < "$work/err" | cut -c1-16)"
}

inputs=("$@")
if [ ${#inputs[@]} -eq 0 ]; then
  inputs=("$here"/corpus/*.cbor "$root"/bench/corpus/*.cbor)
fi

echo "== tools: same inputs and flags, both implementations, transcripts diffed =="
echo "   ${#inputs[@]} inputs x ${#dump_flags[@]} cbordump flags"

: > "$work/dump-up"
: > "$work/dump-our"
for f in "${inputs[@]}"; do
  name=${f##*/}
  for flag in "${dump_flags[@]}"; do
    printf '%-28s %-6s %s\n' "$name" "$flag" "$(run_pair "$up_dump"  "$flag" "$f")" >> "$work/dump-up"
    printf '%-28s %-6s %s\n' "$name" "$flag" "$(run_pair "$our_dump" "$flag" "$f")" >> "$work/dump-our"
  done
done

status=0
if diff -u "$work/dump-up" "$work/dump-our" > "$work/dump.diff"; then
  printf '  %-12s %5s cases  %5s differ\n' cbordump "$(wc -l < "$work/dump-our")" 0
else
  echo "  cbordump: DIVERGES FROM UPSTREAM"
  head -40 "$work/dump.diff"
  status=1
fi

# json2cbor eats JSON, so its inputs are what cbordump just produced. Round
# tripping through -jM is the pair the two tools are documented to form: the
# metadata exists so the conversion back can be exact.
echo "   round trip: cbordump -jM | json2cbor -M"
: > "$work/json-up"
: > "$work/json-our"
for f in "${inputs[@]}"; do
  name=${f##*/}
  # Only inputs upstream can render as JSON at all are round-trippable; the
  # rest exit non-zero with nothing on stdout and there is nothing to feed.
  "$up_dump" -jM "$f" > "$work/in.json" 2>/dev/null || continue
  [ -s "$work/in.json" ] || continue
  for flag in "${json_flags[@]}"; do
    printf '%-28s %-3s %s\n' "$name" "${flag:--}" "$(run_pair "$up_json"  "$flag" "$work/in.json")" >> "$work/json-up"
    printf '%-28s %-3s %s\n' "$name" "${flag:--}" "$(run_pair "$our_json" "$flag" "$work/in.json")" >> "$work/json-our"
  done
done

if diff -u "$work/json-up" "$work/json-our" > "$work/json.diff"; then
  printf '  %-12s %5s cases  %5s differ\n' json2cbor "$(wc -l < "$work/json-our")" 0
else
  echo "  json2cbor: DIVERGES FROM UPSTREAM"
  head -40 "$work/json.diff"
  status=1
fi

exit $status
