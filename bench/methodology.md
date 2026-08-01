# Benchmark methodology

This measures the Rust port of tinycbor against the upstream C library on the
same workload, through the same C header, in the same process shape. It is meant
to be re-runnable by someone who has never seen this repository.

**Headline, stated up front so nobody has to dig for it: at pure parsing the Rust
port is slower than C on five of eight corpus files and faster on three, spanning
0.87x to 1.22x. It starts a process slightly slower (1.09x on the minimum, from a
binary 119x larger) and uses more memory. It is faster at pretty-printing, in
places by a lot, for a reason explained below that is mostly not about decoding.
Both halves are in `results.json`; neither was dropped.** See [Results](#results).

An earlier run of this same benchmark had parsing 1.23x-1.83x slower on all eight
files, and that number was published here for a day. The cause and the fix are in
[Where Rust loses](#where-rust-loses); the old figures are kept below rather than
quietly overwritten.

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
byte-identical — `crates/cbor-ffi/include/{cbor.h,cborjson.h,cborinternal_p.h,
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

**Startup** — `hyperfine -N --warmup 10 --runs 200` over
`driver parse corpus/small_ints.cbor 1`. The 58-byte input makes the CBOR work
negligible, so what is left is `execve`, the loader, runtime init, and one
trivial parse. `-N` skips the intermediate shell. Percentiles are computed from
hyperfine's per-run times in its `--export-json` output, not from the mean it
prints on screen.

**Throughput** — the driver times each repetition individually with
`CLOCK_MONOTONIC` and prints every sample, so `run.py` can take percentiles over
real measurements. Two modes:

- `pretty` — `cbor_parser_init` then `cbor_value_to_pretty_advance(devnull, &it)`.
  This is the workload the task calls for. It touches every input byte and
  produces output, so its MB/s figure is a true bytes-consumed rate.
- `parse` — `cbor_parser_init` then `cbor_value_advance(&it)`, which walks the
  whole document structurally with no output. **This is a structural traversal
  rate, not a bytes-consumed rate:** `cbor_value_advance` skips over text and
  byte string payloads by adding their length rather than reading them. For
  `text_utf8.cbor` and `bytes_heavy.cbor` the resulting `p50_mb_per_s` is
  physically meaningless (it implies tens of GB/s) and only the *ratio* between
  the two builds means anything. It is kept because it is the only view that
  isolates the parser from `stdio`.

**Peak RSS** — `VmHWM` from `/proc/self/status`, read after the timed loop.

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
across the corpus, identical for both builds — that was the Python harness's own
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

Sample counts are recorded per entry as `samples`. They vary enormously by design
— 180,000 for `parse` on the 58-byte file, 180 for `pretty` on the large ones,
because a rep there costs 20-30 ms. **p99 on the slow `pretty` entries therefore
rests on ~180 samples and is the second-worst observation, not a smooth tail
estimate.** p50 is solid everywhere; treat the slow-file p99s as indicative.

### Timer overhead

The driver measures the cost of one back-to-back `clock_gettime` pair (best of
1000) and reports it as `timer_overhead_ns` — about 14 ns here. On
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
  was not idle — a browser and other user processes were running throughout. The
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
`TINYCBOR_UPSTREAM` — it is used only for the header parity assertion, since the
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

Recorded run: `results.json`, linked at `2026-08-01T03:37:03Z`,
`environment.libs_as_linked.rust.sha256` = `cc00319144bf13fa...`, both archives
`matches_current_tree: true`. 1-minute load average during the run: **2.16**
on 8 cores. `rust_vs_c_p50` is `rust / c`, so **greater than 1.0 means the Rust
port is slower**.

### Where Rust loses

**`parse` mode -- structural traversal, no output. Rust is slower on five of the
eight files and faster on three.**

| file | C p50 | Rust p50 | p50 ratio | p99 ratio |
|---|---:|---:|---:|---:|
| `deep_nest.cbor` | 2,106,189 ns | 2,575,468 ns | **1.22x slower** | 1.03x |
| `map_heavy.cbor` | 885,312 ns | 996,951 ns | **1.13x slower** | 1.05x |
| `text_utf8.cbor` | 6,160 ns | 6,826 ns | **1.11x slower** | 1.06x |
| `bytes_heavy.cbor` | 24,162 ns | 25,275 ns | **1.05x slower** | 0.98x |
| `tagged.cbor` | 162,547 ns | 163,145 ns | **1.00x slower** | 0.97x |
| `indefinite.cbor` | 429,126 ns | 414,998 ns | **0.97x faster** | 0.90x |
| `flat_array.cbor` | 972,885 ns | 929,512 ns | **0.96x faster** | 0.99x |
| `small_ints.cbor` | 187 ns | 163 ns | **0.87x faster** | 1.00x |

Mean p50 ratio across the eight: **1.038**.

What remains is concentrated in `deep_nest`, which is the corpus file with the most
container transitions per byte. That points at `advance_recursive`, which builds a
fresh `CborValue` per nesting level, rather than at the per-item decode path.

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

**Startup is still a small loss:**

| | C | Rust | ratio |
|---|---:|---:|---:|
| p50 | 960,361 ns | 919,596 ns | 0.96x |
| p99 | 1,827,085 ns | 1,505,998 ns | 0.82x |
| min | 707,651 ns | 770,811 ns | **1.09x slower** |
| binary size | 43,856 B | 5,202,248 B | **119x larger** |

The p50 and p99 here say Rust starts *faster*, and that is measurement noise, not a
result: process startup is dominated by whatever else the machine is doing, and the
C side caught more of it this run. The minimum is the honest column for a
fixed-cost measurement, and it says Rust is 1.09x slower, consistent with the
earlier run's 1.10x. Reported this way round rather than taking the flattering
number.

Linking a Rust staticlib pulls in the Rust standard library: ~63 us more to start
and a binary two orders of magnitude bigger. Irrelevant for a long-lived process;
not irrelevant for a CLI invoked in a loop, or for firmware, which is a
substantial part of tinycbor's actual audience.

**Peak RSS is also a loss,** on every file in both modes:

| | C | Rust |
|---|---:|---:|
| `parse`, range across corpus | 1,820 - 2,580 KiB | 2,196 - 2,972 KiB |
| `pretty`, range across corpus | 1,936 - 2,392 KiB | 2,556 - 4,116 KiB |
| `pretty`, worst ratio (`bytes_heavy.cbor`) | 2,392 KiB | 4,116 KiB (**1.72x**) |

Both figures include the input buffer and the driver's timing array, so the
absolute numbers are upper bounds -- but the delta is real and one-directional.
The `pretty` gap is the memory price of the speed win below.

### Where Rust wins

**`pretty` mode** -- parse plus `cbor_value_to_pretty_advance` to `/dev/null`,
output verified byte-identical by the equivalence gate.

| file | C p50 | Rust p50 | ratio |
|---|---:|---:|---:|
| `bytes_heavy.cbor` | 29,048,669 ns | 917,972 ns | 0.03x (**32x faster**) |
| `tagged.cbor` | 5,907,177 ns | 994,963 ns | 0.17x |
| `indefinite.cbor` | 9,272,523 ns | 1,644,460 ns | 0.18x |
| `deep_nest.cbor` | 21,908,687 ns | 4,523,782 ns | 0.21x |
| `map_heavy.cbor` | 19,679,142 ns | 4,969,279 ns | 0.25x |
| `flat_array.cbor` | 9,916,169 ns | 2,894,352 ns | 0.29x |
| `small_ints.cbor` | 2,696 ns | 882 ns | 0.33x |
| `text_utf8.cbor` | 15,186,709 ns | 7,282,783 ns | 0.48x |

**This win should be understood before it is quoted. It is not evidence that the
Rust decoder is 32x faster -- the `parse` table above says the C decoder is the
faster one.**

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
