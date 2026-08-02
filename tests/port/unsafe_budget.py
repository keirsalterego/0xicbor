#!/usr/bin/env python3
"""Checks the two claims the readme makes about `unsafe`, rather than repeating them.

    tests/port/unsafe_budget.py [--count]

1. `cbor-core` contains no `unsafe` at all. That one is really enforced by
   `#![forbid(unsafe_code)]` in its lib.rs, so this is a check that the attribute
   is still there.
2. Every `unsafe { }` in `cbor-ffi` has a `// SAFETY:` line reachable above it --
   either immediately, or on the enclosing function, which is where a whole
   module's worth of blocks sharing one invariant put it.

The readme quotes a block count and says each one carries a SAFETY line. That
was true of 73 of 80 when this script was first run: six had the line on the
enclosing function and one had none at all. A number in a readme that nothing
recomputes drifts, so `make lint` recomputes it.

`--count` prints just the number, which is what the readme quotes.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CORE = ROOT / "crates" / "cbor-core"
FFI = ROOT / "crates" / "cbor-ffi"

# A line that opens a block, not `unsafe fn` or `unsafe impl`.
BLOCK = re.compile(r"\bunsafe\s*\{")
FN = re.compile(r"^\s*(pub\s+)?(extern\s+\"C\"\s+)?(unsafe\s+)?fn\s")


def annotated(lines: list[str], i: int) -> bool:
    """True if a SAFETY marker covers the block on line `i`."""
    if "SAFETY" in lines[i]:
        return True
    # Walk back to the top of the enclosing function. A SAFETY line anywhere in
    # that span, or in the doc comment above it, counts.
    j = i - 1
    while j >= 0:
        if "SAFETY:" in lines[j]:
            return True
        if FN.match(lines[j]):
            # Keep going through the doc comment and attributes above the fn.
            k = j - 1
            while k >= 0 and (
                lines[k].lstrip().startswith(("///", "//", "#["))
                or not lines[k].strip()
            ):
                if "SAFETY:" in lines[k]:
                    return True
                k -= 1
            return False
        j -= 1
    return False


def main() -> int:
    core_hits = [
        f"{f.relative_to(ROOT)}:{i + 1}"
        for f in sorted(CORE.rglob("*.rs"))
        for i, l in enumerate(f.read_text().splitlines())
        if BLOCK.search(l)
    ]

    forbids = (CORE / "src" / "lib.rs").read_text()
    total = 0
    bare = []
    for f in sorted(FFI.rglob("*.rs")):
        lines = f.read_text().splitlines()
        for i, l in enumerate(lines):
            if not BLOCK.search(l):
                continue
            total += 1
            if not annotated(lines, i):
                bare.append(f"{f.relative_to(ROOT)}:{i + 1}: {l.strip()[:70]}")

    if "--count" in sys.argv:
        print(total)
        return 0

    bad = False
    if "#![forbid(unsafe_code)]" not in forbids:
        print("cbor-core has lost #![forbid(unsafe_code)]")
        bad = True
    if core_hits:
        print(f"cbor-core has {len(core_hits)} unsafe blocks: {core_hits}")
        bad = True
    if bare:
        print(f"{len(bare)} unsafe blocks in cbor-ffi have no SAFETY line:")
        for b in bare:
            print("  ", b)
        bad = True
    if not bad:
        print(f"  unsafe budget   {total} blocks, all annotated, cbor-core at 0")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
