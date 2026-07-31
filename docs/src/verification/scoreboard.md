# Scoreboard

Real output, updated as the port moves. Nothing here is a target.

## Original suite

| Binary | Passed | Failed | Notes |
|---|---:|---:|---|
| `tst_encoder` | 2 | 1,594 | |
| `tst_parser` | 17 | 2,489 | |
| `tst_tojson` | 2 | 825 | |
| `tst_c90` | 1 | 0 | header compiles under `-std=c90 -pedantic` |
| `tst_cpp` | — | — | inapplicable, see [decision 2](../reference/decisions.md) |
| **Total** | **21** | **4,908** | of 4,929 reachable |

Every entry point is currently a stub returning `CborErrorInternalError`. The 21 passing
rows are rows that expect an error and happen to get one — the honest floor for a library
that does nothing yet.

## ABI

| Check | Result |
|---|---|
| Exported symbols | 44 / 44 |
| `nm` diff vs upstream | **empty** |
| Struct sizes and offsets | asserted, passing |
| C90 header cleanliness | passing |

## Budget

| | |
|---|---|
| `unsafe` blocks | 0 |
| Third-party dependencies | 0 |
| Differential fuzz | not yet run |
| Decision log entries | 11 |

## Reading this honestly

A 21/4,929 pass rate is what a correctly-wired scaffold looks like, not a working library.
The value of publishing it is that the number has a floor and a ceiling that are both known,
the measurement is one command, and every subsequent claim is a delta against a figure that
was already public.

If the port ends below 100%, the failing tests stay failing and each gets a paragraph in the
[decision log](../reference/decisions.md). A reproducible 94% is worth more than a 100% that
required editing the suite.
