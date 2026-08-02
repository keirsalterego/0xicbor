# Scoreboard

Real output, updated as the port moves. Nothing here is a target.

## Original suite

| Binary | Passed | Failed | Notes |
|---|---:|---:|---|
| `tst_encoder` | 1,596 | 0 | |
| `tst_parser` | 2,506 | 0 | |
| `tst_tojson` | 827 | 0 | |
| `tst_c90` | 1 | 0 | header compiles under `-std=c90 -pedantic` |
| `tst_cpp` | n/a | n/a | inapplicable, see [decision 2](../reference/decisions.md) |
| **Total** | **4,929** | **0** | of 4,929 reachable |

Everything that can pass, passes, and nothing is stubbed any more. The last one was
`_cbor_value_dup_string`, which no row in the suite reaches. It has its own
differential test under `tests/port/` instead, run by `make test` against upstream's own archive.

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
| Parse speed vs C | **0.37x**, faster on all 8 files (2.7x) |
| Pretty-print vs C | **0.23x** (4.4x), but see [methodology](../../bench/methodology.md) |
| `unsafe` blocks | 80 (all in `cbor-ffi`) |
| Third-party dependencies | 0 |
| Differential fuzz | 8.4M execs, **zero divergences**, four targets, [history](differential-fuzzing.md) |
| Tools vs upstream | exact on 4,509 documents x 20 flag combinations, [decision 18](../reference/decisions.md) |
| Decision log entries | 18 |

## Reading this honestly

4,929/4,929 is every row that can apply to a port. It is not 4,931 because two rows test that
upstream's C compiles as C++, which a Rust port cannot satisfy by construction.

The speed numbers under it need reading carefully in the other direction. Parsing is faster
than the C on all eight corpus files now, but it was 1.48x *slower* on all eight for a day,
and the history table in the [methodology](../../bench/methodology.md) keeps that figure
rather than erasing it. The print number is 4.4x and most of it is upstream's `vfprintf`
per byte, not decoding, which the methodology says before it quotes it.

If the port ends below 100%, the failing tests stay failing and each gets a paragraph in the
[decision log](../reference/decisions.md). A reproducible 94% is worth more than a 100% that
required editing the suite.
