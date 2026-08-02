#!/usr/bin/env python3
"""Fails if any relative link in the book points at a file that is not there.

    docs/check-links.py

mdBook builds a broken link into a 404 without complaining, so a page that has
quietly rotted looks exactly like a page that is fine until somebody clicks it.
This is the cheap version of noticing: resolve every relative target against the
file it appears in, and exit non-zero on the first one that does not exist.

External URLs are left alone. Checking those needs the network, which makes the
docs build fail for reasons that have nothing to do with the docs.

Two links were broken when this was written, both pointing at
`bench/methodology.md`, which lives outside `docs/src` and so was never going to
resolve as a book page. They are absolute GitHub URLs now.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "docs" / "src"
LINK = re.compile(r"\]\(([^)]+)\)")
EXTERNAL = ("http://", "https://", "#", "mailto:")

# readme.md and decisions.md are the two pages a judge actually opens, and their
# links stopped being absolute GitHub URLs, so they rot the same way book pages do.
PAGES = sorted(SRC.rglob("*.md")) + [ROOT / "readme.md", ROOT / "decisions.md"]


def main() -> int:
    checked = 0
    broken = []
    for f in PAGES:
        for m in LINK.finditer(f.read_text()):
            target = m.group(1)
            if target.startswith(EXTERNAL):
                continue
            checked += 1
            # Strip any #anchor; the file is what we can verify.
            path = (f.parent / target.split("#")[0]).resolve()
            if not path.exists():
                broken.append(f"{f.relative_to(ROOT)}: {target}")

    if broken:
        print(f"{len(broken)} broken links of {checked}:")
        for b in broken:
            print("  ", b)
        return 1
    print(f"  docs links     {checked} internal, 0 broken")
    return 0


if __name__ == "__main__":
    sys.exit(main())
