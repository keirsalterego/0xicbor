# Scoreboard

Real output, updated as the port moves. Nothing here is a target.

## Original suite

| Binary | Passed | Failed | Notes |
|---|---:|---:|---|
| `tst_encoder` | 1,596 | 0 | complete |
| `tst_parser` | 1,134 | 1,372 | |
| `tst_tojson` | 2 | 825 | |
| `tst_c90` | 1 | 0 | header compiles under `-std=c90 -pedantic` |
| `tst_cpp` | — | — | inapplicable, see [decision 2](../reference/decisions.md) |
| **Total** | **2,732** | **2,197** | of 4,929 reachable |

The encoder is complete. The parser navigates correctly and the diagnostic printer works,
which is what moved `tst_parser` from 17 to 1,134. What is still stubbed is `cbor_error_string`,
`cbor_value_validate`'s strict modes, `_cbor_value_dup_string`, and the whole JSON converter —
which is why `tst_tojson` has barely moved.

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
| `unsafe` blocks | 46 |
| Third-party dependencies | 0 |
| Differential fuzz | not yet run |
| Decision log entries | 11 |

## Reading this honestly

A 2,732/4,929 pass rate is a library that encodes correctly and parses correctly, with its
JSON half not yet written. The value of publishing it is that the number has a known ceiling,
the measurement is one command, and every subsequent claim is a delta against a figure that
was already public.

If the port ends below 100%, the failing tests stay failing and each gets a paragraph in the
[decision log](../reference/decisions.md). A reproducible 94% is worth more than a 100% that
required editing the suite.
