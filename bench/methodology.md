# Benchmark methodology

This measures the Rust port of tinycbor against the upstream C library on the
same workload, through the same C header, in the same process shape. It is meant
to be re-runnable by someone who has never seen this repository.

**Headline, stated up front so nobody has to dig for it: the Rust port is faster
than the C on all sixteen throughput measurements, mean ratio 0.292, which is 3.4x.
Parsing is 2.8x (every corpus file between 2.0x and 3.8x) and pretty-printing is
4.5x. It is still slower to start a process, 1.11x on the minimum from a binary
119x larger, and it still uses more memory. Both halves are in `results.json`;
neither was dropped.** See [Results](#results).

Parsing was 1.23x-1.83x *slower* on all eight files when this benchmark was first
run, and that number was published here for a day. Everything since is in
[Where the time went](#where-the-time-went), and every earlier figure is kept in the
history table rather than quietly overwritten.

---

## What is compared

| | C reference | Rust port |
|---|---|---|
| archive | `bench/reference/libtinycbor-upstream.a` | `target/release/libtinycbor.a` |
| provenance | upstream `intel/tinycbor` @ `9441b2ca`, built by upstream's own CMake | `cargo build --release` in this repo |
| flags | `-O3 -DNDEBUG` (`CMAKE_BUILD_TYPE=Release`) | `lto = true`, `codegen-units = 1`, `panic = "abort"` |

Both are static archives exporting the same 44 `cbor_*` symbols. `make symbols`
in the repository root diffs our exports against
`bench/reference/symbols-upstream.txt` and prints nothing when they match.

## Why the two drivers are genuinely comparable

There is one driver source, `bench/driver.c`. `bench/build.sh` compiles it twice
with identical flags and identical include paths, changing only which `.a` goes
on the link line:

```
cc -O2 -Wall -Wextra -std=c99 -I crates/cbor-ffi/include bench/driver.c \
   -o bench/build/driver-c    bench/reference/libtinycbor-upstream.a -lm
cc -O2 -Wall -Wextra -std=c99 -I crates/cbor-ffi/include bench/driver.c \
   -o bench/build/driver-rust target/release/libtinycbor.a -lm -lpthread -ldl
```

The include path is ours, not upstream's, and that is safe because the two are
byte-identical: `crates/cbor-ffi/include/{cbor.h,cborjson.h,cborinternal_p.h,
compilersupport_p.h,tinycbor-export.h,tinycbor-version.h}` all `cmp` clean
against upstream's `src/` and generated `build-ref/`. `build.sh` re-checks
`cbor.h` and `cborjson.h` on every build and refuses to link if they have
drifted, because a header difference would silently make the comparison invalid.

`-lpthread -ldl` on the Rust line are the Rust standard library's own link
requirements. They are the only asymmetry, and they are not optional.

## Output equivalence gate

Before any timing is believed, `bench/run.py` runs both binaries in `dump` mode
over every corpus file and compares SHA-256 of the pretty-printed output. A build
that prints fewer bytes would look faster for a reason that has nothing to do
with speed. Results for any file that fails this check are still recorded but
marked `identical: false` in `results.json`, and must not be quoted as a
comparison.

At the time of the recorded run all eight files match byte for byte.

> Historical note, kept because it is the reason the gate exists: an earlier
> build of the Rust library appended a stray `_` indefinite-length indicator to
> strings in `indefinite.cbor`, emitting 301,486 bytes where C emitted 291,486.
> The gate caught it. It has since been fixed in the library.

## What is measured

Three things, and one deliberate omission.

**Startup**: `hyperfine -N --warmup 10 --runs 200` over
`driver parse corpus/small_ints.cbor 1`. The 58-byte input makes the CBOR work
negligible, so what is left is `execve`, the loader, runtime init, and one
trivial parse. `-N` skips the intermediate shell. Percentiles are computed from
hyperfine's per-run times in its `--export-json` output, not from the mean it
prints on screen.

**Throughput**: the driver times each repetition individually with
`CLOCK_MONOTONIC` and prints every sample, so `run.py` can take percentiles over
real measurements. Two modes:

- `pretty`: `cbor_parser_init` then `cbor_value_to_pretty_advance(devnull, &it)`.
  This is the workload the task calls for. It touches every input byte and
  produces output, so its MB/s figure is a true bytes-consumed rate.
- `parse`: `cbor_parser_init` then `cbor_value_advance(&it)`, which walks the
  whole document structurally with no output. **This is a structural traversal
  rate, not a bytes-consumed rate:** `cbor_value_advance` skips over text and
  byte string payloads by adding their length rather than reading them. For
  `text_utf8.cbor` and `bytes_heavy.cbor` the resulting `p50_mb_per_s` is
  physically meaningless (it implies tens of GB/s) and only the *ratio* between
  the two builds means anything. It is kept because it is the only view that
  isolates the parser from `stdio`.

**Peak RSS**: `VmHWM` from `/proc/self/status`, read after the timed loop.

`/usr/bin/time -v` is not installed on this machine, so the obvious substitute
was `getrusage(RUSAGE_SELF).ru_maxrss`. **That was tried and rejected as wrong.**
On Linux `ru_maxrss` is inherited across `fork()` and is not reset by `execve()`,
so a driver spawned from a large parent reports the parent's high-water mark. The
same driver, on the same input, doing identical work:

```
$ bench/build/driver-c parse bench/corpus/small_ints.cbor 5      # from a shell
getrusage kib: 2504
$ python3 -c "...allocate 32 MB, then subprocess the same command..."
getrusage kib: 43128
```

A 17x swing that has nothing to do with the program being measured. The first
full run of this benchmark reported RSS figures climbing from 22 MB to 38 MB
across the corpus, identical for both builds, which was the Python harness's own
footprint, not tinycbor's. `VmHWM` lives in the `mm`, which `execve` replaces, so
it is immune; it reads 1640 KiB from a shell and 1632 KiB from the fat parent.

Two caveats on the figure that remain: it includes the input file buffer, and it
includes the driver's per-rep timing array (`8 * reps` bytes, up to 400 KB on
high-rep files). Both are identical for the two builds, so the C-vs-Rust delta is
meaningful even though the absolute number is an upper bound.

**Not measured: `cbor_value_to_json_advance`.** It was unimplemented in the Rust
port when this harness was specified, so benchmarking it would have measured a
stub. It has since landed (`8dbfff1 feat(tojson): port the JSON converter`) and
covering it is now a two-line change: add a `json` branch beside `pretty` in
`bench/driver.c` and add `"json"` to `MODES` in `bench/run.py`. That was left
undone deliberately rather than by oversight.

## Iterations, warmup, outliers

- Rep count per block is calibrated per (file, mode) to about 300 ms of work,
  clamped to [50, 50000], and **derived from whichever build is slower** so both
  sides run the same number of repetitions.
- The first `max(2, 10%)` reps of every block are discarded as warmup: cold
  i-cache, first-touch page faults on the sample array, branch predictors cold.
  The discarded count is recorded per entry as `warmup_discarded`.
- Each (file, mode) runs **4 rounds**, alternating C block, Rust block, C block,
  Rust block… Samples are pooled across rounds. Alternating means frequency and
  thermal drift over the run lands on both sides equally instead of penalising
  whichever went last.
- **No outliers are removed.** Reporting p50 and p99 instead of a mean *is* the
  outlier handling: p50 ignores occasional scheduler interference by
  construction, and p99 is there precisely so that interference is visible rather
  than averaged away. `min_ns` and `max_ns` are also recorded per entry.
- Percentiles are nearest-rank, no interpolation, so every published number is a
  measurement that actually occurred.
- The benchmark process is pinned with `taskset -c 2` (override with
  `BENCH_CPU=<n>`).

### Sample counts

Sample counts are recorded per entry as `samples`. They vary enormously by design:
180,000 for `parse` on the 58-byte file, 180 for `pretty` on the large ones,
because a rep there costs 20-30 ms. **p99 on the slow `pretty` entries therefore
rests on ~180 samples and is the second-worst observation, not a smooth tail
estimate.** p50 is solid everywhere; treat the slow-file p99s as indicative.

### Timer overhead

The driver measures the cost of one back-to-back `clock_gettime` pair (best of
1000) and reports it as `timer_overhead_ns`, about 14 ns here. On
`small_ints.cbor`, where a `parse` rep is ~180 ns, that is a ~7% tax. It applies
identically to both builds so it never flips a winner, but it does compress the
ratio toward 1.0. Treat `small_ints.cbor` as a latency-floor probe rather than a
throughput result.

## Environment of the recorded run

Recorded automatically into `results.json` under `environment`:

- CPU: **Intel Core i5-10200H @ 2.40 GHz**, 8 logical / 4 physical
- Scaling governor: **`performance`** (`/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor`)
- Turbo: **enabled** (`/sys/devices/system/cpu/intel_pstate/no_turbo` = `0`).
  Disabling it needs root and was not done; it is a source of variance,
  and it is the main reason p99 is noisier than p50 here.
- Kernel `7.0.12+kali-amd64`, `cc (Debian 15.3.0-1) 15.3.0`, `rustc 1.97.1`
- **1-minute load average during the recorded run: 2.63 on 8 cores.** This box
  was not idle: a browser and other user processes were running throughout. The
  p50 ratios reproduced to within a few percent across two independent full runs
  against the same archive, so the conclusions hold, but the absolute nanosecond
  figures and especially the p99s would be better on a quiet machine. If you
  reproduce this on an idle box, expect the same ratios and lower absolute
  numbers.

### Attributing results to an exact library build

The Rust archive was rebuilt three times *during* this benchmarking session by
work happening in parallel, and the parse regression measurably widened as a
parser refactor landed. Since the drivers link statically, a `results.json` is
only meaningful if you know which bytes went into the binaries.

`bench/build.sh` therefore writes `bench/build/link-manifest.json` at link time,
recording the SHA-256 of both archives as they were linked. `bench/run.py` copies
that into `results.json` as `environment.libs_as_linked`, and additionally
re-hashes the archives on disk and sets `matches_current_tree`. If the archive
moved after the drivers were built, run.py prints

```
!! rust archive changed since link; rerun bench/build.sh to measure the current tree
```

and records `matches_current_tree: false`. The recorded run has `true` for both.
**Always run `bench/build.sh` immediately before `bench/run.py`.**

## Reproducing

From the repository root, with `hyperfine` on `PATH`:

```sh
cargo build --release          # produces target/release/libtinycbor.a
bench/gen_corpus.py            # deterministic; regenerates bench/corpus/
bench/build.sh                 # builds both drivers, asserts header parity
bench/run.py                   # writes bench/results.json
```

Total runtime is about 40 seconds. `bench/run.py` prints a table as it goes and
finishes by listing every measurement where Rust is slower.

If your upstream checkout is not at `/home/keir/tinycbor-upstream`, set
`TINYCBOR_UPSTREAM`. It is used only for the header parity assertion, since the
reference archive itself is vendored at `bench/reference/libtinycbor-upstream.a`.

Individual measurements can be run by hand:

```sh
# one pretty pass, output to stdout, for eyeballing or diffing
bench/build/driver-rust dump bench/corpus/map_heavy.cbor 1 | head

# 500 timed pretty reps, JSON with every per-rep duration
taskset -c 2 bench/build/driver-c pretty bench/corpus/bytes_heavy.cbor 500

# startup only
hyperfine -N --warmup 10 --runs 200 \
  'bench/build/driver-c parse bench/corpus/small_ints.cbor 1' \
  'bench/build/driver-rust parse bench/corpus/small_ints.cbor 1'
```

## Corpus

Eight files, 58 B to 628 KB, generated deterministically by `bench/gen_corpus.py`
from a fixed seed. Per-file shapes are in `bench/corpus/README.md` and
`bench/corpus/manifest.json`.

The corpus was written before any timing was taken and has not been edited since.
It was not adjusted after seeing results. `flat_array`, `deep_nest`, `map_heavy`,
`tagged`, `indefinite`, `text_utf8` and `bytes_heavy` each isolate a different
decode path; `bytes_heavy` and `text_utf8` in particular were included because
they are the pretty-printer's worst cases, not because they favour anyone.

## Results

Recorded run: `results.json`, linked at `2026-08-01T07:31:16Z`,
`environment.libs_as_linked.rust.sha256` = `76e80984d6bf9161...`, both archives
`matches_current_tree: true`. 1-minute load average during the run: **2.61**
on 8 cores. `rust_vs_c_p50` is `rust / c`, so **greater than 1.0 means the Rust
port is slower**.

### Where the time went

**`parse` mode -- structural traversal, no output. Faster on all eight.**

| file | C p50 | Rust p50 | p50 ratio | p99 ratio |
|---|---:|---:|---:|---:|
| `small_ints.cbor` | 197 ns | 98 ns | **0.50x faster** | 0.48x |
| `flat_array.cbor` | 1,010,526 ns | 463,657 ns | **0.46x faster** | 0.51x |
| `bytes_heavy.cbor` | 23,962 ns | 10,199 ns | **0.43x faster** | 0.38x |
| `text_utf8.cbor` | 6,317 ns | 2,306 ns | **0.36x faster** | 0.27x |
| `deep_nest.cbor` | 2,096,361 ns | 660,661 ns | **0.32x faster** | 0.33x |
| `indefinite.cbor` | 437,918 ns | 136,608 ns | **0.31x faster** | 0.31x |
| `map_heavy.cbor` | 900,937 ns | 236,861 ns | **0.26x faster** | 0.25x |
| `tagged.cbor` | 162,261 ns | 41,435 ns | **0.26x faster** | 0.23x |

Mean p50 ratio across the eight: **0.362**, which is 2.8x faster.

#### Nesting depth, which used to be the whole problem

`deep_nest.cbor` is 4,000 chains of 40 nested arrays, and for most of this port's life
it was the file it lost on. Holding the size at 190 KB and varying only the depth used
to show a clean slope: 0.94x at depth 4, crossing over around 16, 1.25x by 56. It was
not doing more work at depth. Under callgrind at depth 80 the two ran 48.0 M and 47.6 M
instructions, a gap of 0.83%, for a 23% difference in wall clock; at depth 8 the
instruction gap was *larger* and this port was 9% *faster*. Branch mispredicts were
0.5% against 0.3% and D1 misses were negligible on both, so it was neither of those.
It was the recursion itself.

The subtree scan does not recurse, and the slope is gone:

| nesting depth | `parse` p50 ratio, before | after |
|---:|---:|---:|
| 4 | 0.94 | 0.33 |
| 8 | 0.91 | 0.33 |
| 16 | 1.01 | 0.31 |
| 24 | 1.08 | 0.30 |
| 32 | 1.15 | 0.30 |
| 40 | 1.19 | 0.30 |
| 56 | 1.25 | 0.30 |
| 80 | 1.23 | 1.23 |

Depth 80 is the honest edge of it. The scan carries 64 levels and hands anything deeper
to the recursive path, so past that the old number comes straight back. That boundary is
a deliberate trade: the level array is initialised on every call, and raising it to 256
cost `small_ints` 0.49x to 0.59x and `deep_nest` 0.30x to 0.33x for a case no real
document reaches. Documents nested more than 64 deep are fuzzer output, and they are
still correct, just no faster than they were.

#### How this number moved, and why

It was much worse. Measured against successive builds of the archive on the same
machine and the same corpus:

| archive | `parse` p50 ratio range |
|---|---|
| `2883526a...` | 1.00x - 1.30x, 6 of 8 files slower |
| `0eed29dd...` (after `3064ad2 feat(parser): callback-driven sources`) | 1.21x - 1.50x, 8 of 8 slower |
| `9687b6a7...` | 1.17x - 1.83x, 8 of 8 slower |
| `a936412f...` | 1.23x - 1.83x, 8 of 8 slower |
| `cc003191...` (after `39e5be5 perf(parser): monomorphise on the byte source`) | 0.87x - 1.22x, 5 of 8 slower |
| `56f98a76...` (after `e8a3e44` and `ec8300f`, below) | 0.84x - 1.23x, 4 of 8 slower |
| `4428fd94...` (after `b952b7d`, below) | 0.82x - 1.19x, 3 of 8 slower |
| `76e80984...` (after `8b71557` and `ab98578`, below) | **0.26x - 0.50x, 0 of 8 slower** |

The callback-driven source refactor named above as "the obvious suspect" was in
fact the cause. It made every read test `parser->flags & ExternalSource` first,
which upstream also does -- the difference is that GCC at `-O3` clones the callers
and folds that test away. Two flags confirm which pass:

| upstream build | `parse` p50 vs stock `-O3` |
|---|---|
| `-O3 -fno-strict-aliasing` | 0.99x (no effect) |
| `-O3 -fno-ipa-cp-clone` | 1.17x slower |
| `-O2 -finline-functions` | 1.34x slower |

So it is interprocedural constant propagation with cloning, not type-based alias
analysis and not inlining alone. Rust has no such pass, but it has generics, which
are the same specialisation decided by the type system instead of the optimiser.
Making the byte source a type parameter took the mean from 1.492 to 1.038 for
4,128 bytes of extra `.text`. See `decisions.md` entry 13.

Three later changes took it from 1.038 to 0.984, all found by reading the generated
code or the profile rather than by guessing, and a fourth took it to 0.362:

- `e8a3e44` -- `advance_recursive` opened with `is_fixed_type` then `is_container`,
  which LLVM compiled into a rotate and a bit-table lookup: thirteen instructions
  and three branches. The two classes it is testing for are each a pair of major
  types differing only in bit 5, so masking that bit off makes each one a single
  comparison. Five instructions, two branches, and it is what GCC emits from the
  same source.
- `ec8300f` -- the string branch's six-argument call needed registers that
  `advance_recursive` then had to save and restore on every level of the descent.
  Behind `#[inline(never)]` the frame drops from 48 bytes to 32.
- `b952b7d` -- the same specialisation problem as the byte source, one level down.
  `iterate_string_chunks` took measure-or-copy-or-compare as a runtime enum, so the
  branch sat inside the chunk loop. Upstream passes it as a value too and GCC clones
  the walker per call site; the profile shows `iterate_string_chunks.constprop.0`
  doing exactly that. Three types implementing one trait moved the choice to the
  call site.
- `8b71557` and `ab98578` -- the one that stopped competing on the C's terms.
  `cbor_value_advance` decodes every item it passes into a `CborValue` and then
  discards all of it, because the only things that outlive the walk are the final
  cursor and any error. Skipping the subtree with a flat scan instead of a recursive
  descent took parsing from 0.984 to 0.362 and made every corpus file faster than the
  C. It hands back to the recursive path on anything unusual, so every error the API
  reports still comes from the original code. See `decisions.md` entry 16.

**Startup is still a small loss:**

| | C | Rust | ratio |
|---|---:|---:|---:|
| p50 | 836,736 ns | 873,082 ns | **1.04x slower** |
| p99 | 1,519,075 ns | 1,466,449 ns | 0.97x |
| min | 699,660 ns | 774,243 ns | **1.11x slower** |
| binary size | 43,856 B | 5,207,064 B | **119x larger** |

This is the only measurement of the seventeen where the C still wins, and it is not
about CBOR. The minimum is the column to read for a fixed cost like this, and across
five runs on different machine load it has said 1.09x, 1.10x, 1.10x, 1.11x and
1.11x. One of those runs had
p50 and p99 showing Rust *faster*, which was the C side catching more of the
machine's noise, not a result.

Linking a Rust staticlib pulls in the Rust standard library: ~75 us more to start
and a binary two orders of magnitude bigger. Irrelevant for a long-lived process;
not irrelevant for a CLI invoked in a loop, or for firmware, which is a
substantial part of tinycbor's actual audience.

**Peak RSS is also a loss,** on every file in both modes:

| | C | Rust |
|---|---:|---:|
| `parse`, range across corpus | 1,824 - 2,652 KiB | 2,228 - 3,092 KiB |
| `pretty`, range across corpus | 1,864 - 2,456 KiB | 2,640 - 4,108 KiB |
| `pretty`, worst ratio (`text_utf8.cbor`) | 2,384 KiB | 4,020 KiB (**1.69x**) |

Both figures include the input buffer and the driver's timing array, so the
absolute numbers are upper bounds -- but the delta is real and one-directional.
The `pretty` gap is the memory price of the speed win below.

### Where Rust wins

**`pretty` mode** -- parse plus `cbor_value_to_pretty_advance` to `/dev/null`,
output verified byte-identical by the equivalence gate.

| file | C p50 | Rust p50 | ratio |
|---|---:|---:|---:|
| `bytes_heavy.cbor` | 27,930,719 ns | 866,311 ns | 0.03x (**32x faster**) |
| `tagged.cbor` | 4,642,327 ns | 698,775 ns | 0.15x |
| `indefinite.cbor` | 8,020,005 ns | 1,240,775 ns | 0.15x |
| `deep_nest.cbor` | 22,109,538 ns | 4,117,853 ns | 0.19x |
| `map_heavy.cbor` | 19,109,038 ns | 4,667,256 ns | 0.24x |
| `flat_array.cbor` | 8,328,553 ns | 2,179,569 ns | 0.26x |
| `small_ints.cbor` | 2,646 ns | 763 ns | 0.29x |
| `text_utf8.cbor` | 15,768,159 ns | 7,273,479 ns | 0.46x |

**This win should be understood before it is quoted. It is not evidence that the
Rust decoder is 32x faster -- the `parse` table above puts the decoder
at 2.8x, not 32x.**

Upstream's pretty printer routes every token through a `CborStreamFunction`
callback that `cborpretty_stdio.c:29` implements as `vfprintf`, and its hex dump
calls that callback **once per byte**:

```c
/* tinycbor-upstream/src/cborpretty.c:181 */
static CborError hexDump(CborStreamFunction stream, void *out, const void *ptr, size_t n)
{
    const uint8_t *buffer = (const uint8_t *)ptr;
    CborError err = CborNoError;
    while (n-- && !err)
        err = stream(out, "%02" PRIx8, *buffer++);
    return err;
}
```

On `bytes_heavy.cbor` that is roughly 600,000 `vfprintf` calls, each re-parsing
the format string `"%02x"`. The Rust port does not pay that per-byte varargs
cost. The gap is dominated by output formatting strategy, not by CBOR decoding.

`text_utf8.cbor` at 0.48x is the honest shape of the win where the C side is not paying the
per-byte penalty -- text escaping goes through the callback per run of characters
rather than per byte -- and the advantage shrinks by more than half accordingly.
Read 0.48x, not 0.03x, as the representative number.

### Summary

| dimension | winner |
|---|---|
| Parsing throughput (`parse`, 8/8 files) | **C**, by 23-83% |
| Process startup | **C**, by 8% |
| Binary size | **C**, by 119x |
| Peak RSS | **C**, by 11-68% |
| Pretty-printing throughput | **Rust**, by 2x-32x, mechanism above |

The port is currently a performance regression everywhere except output
formatting, and the parsing regression has been growing. `parse` mode on
`map_heavy.cbor` is the profile to open first.
