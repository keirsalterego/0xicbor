# Scoreboard

Real output, updated as the port moves. Nothing here is a target.

## Original suite

| Binary | Passed | Failed | Notes |
|---|---:|---:|---|
| `tst_encoder` | 1,596 | 0 | |
| `tst_parser` | 2,506 | 0 | |
| `tst_tojson` | 827 | 0 | |
| `tst_c90` | 1 | 0 | header compiles under `-std=c90 -pedantic` |
| `tst_cpp` | — | — | inapplicable, see [decision 2](../reference/decisions.md) |
| **Total** | **4,929** | **0** | of 4,929 reachable |

Everything that can pass, passes. `_cbor_value_dup_string` is the one entry point still
stubbed; nothing in the suite exercises it.

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
| Parse speed vs C | **1.04x slower** (mean p50, 8 files; 5 slower, 3 faster) |
| Pretty-print vs C | 2x–32x faster, but see [methodology](../../bench/methodology.md) |
| `unsafe` blocks | 74 (all in `cbor-ffi`) |
| Third-party dependencies | 0 |
| Differential fuzz | 252,830 execs / 121s, **zero divergences** |
| Decision log entries | 13 |

## Reading this honestly

4,929/4,929 is every row that can apply to a port. It is not 4,931 because two rows test that
upstream's C compiles as C++, which a Rust port cannot satisfy by construction.

The number that matters more is the one below it: parsing is still **slower than the C** on
five of eight corpus files. A 100% pass rate and an honest regression is a more useful
artifact than either alone. It was 1.48x for a day, and the history table in the
[methodology](../../bench/methodology.md) keeps that number rather than erasing it.

If the port ends below 100%, the failing tests stay failing and each gets a paragraph in the
[decision log](../reference/decisions.md). A reproducible 94% is worth more than a 100% that
required editing the suite.
