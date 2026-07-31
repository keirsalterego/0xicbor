#!/usr/bin/env python3
"""Run the benchmark and write bench/results.json.

    bench/gen_corpus.py && bench/build.sh && bench/run.py

Everything it does is described in bench/methodology.md; the short version:

  * output equivalence is checked first, per corpus file, and a file whose two
    builds disagree is still measured but flagged -- the timings for it are not
    comparable and the report says so
  * process startup comes from hyperfine (100 runs, 10 warmups, no shell)
  * throughput comes from the driver itself, which times each repetition
    separately, so we report p50 and p99 over the pooled samples rather than a
    mean that a single scheduler hiccup can drag around
  * the two builds alternate in short blocks, four rounds, so thermal or
    frequency drift lands on both sides equally instead of on whichever ran last
"""

import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
BUILD = os.path.join(HERE, "build")
CORPUS = os.path.join(HERE, "corpus")

BINARIES = {"c": os.path.join(BUILD, "driver-c"), "rust": os.path.join(BUILD, "driver-rust")}
MODES = ["parse", "pretty"]

ROUNDS = 4  # alternating blocks per (file, mode)
TARGET_BLOCK_NS = 300_000_000  # ~0.3 s of work per block, before warmup
MIN_REPS, MAX_REPS = 50, 50_000
WARMUP_FRAC = 0.1  # leading reps discarded per block (cold caches, first-touch faults)

HYPERFINE_RUNS = 200
HYPERFINE_WARMUP = 10


def pct(sorted_vals, p):
    """Nearest-rank percentile. No interpolation: every reported number is a
    measurement that actually happened."""
    if not sorted_vals:
        return None
    k = max(1, min(len(sorted_vals), -(-p * len(sorted_vals) // 100)))
    return sorted_vals[k - 1]


def run(cmd, **kw):
    return subprocess.run(cmd, check=True, capture_output=True, text=True, **kw)


def cpu_model():
    try:
        with open("/proc/cpuinfo") as f:
            for line in f:
                if line.startswith("model name"):
                    return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return platform.processor() or "unknown"


def read_first(path):
    try:
        with open(path) as f:
            return f.read().strip()
    except OSError:
        return None


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def linked_libs():
    """Read the fingerprints bench/build.sh recorded when it linked the drivers.

    The Rust archive is rebuilt often while the port is in progress and the
    drivers link it statically, so stat-ing target/release/libtinycbor.a now can
    easily fingerprint a build that landed after these binaries were made. If the
    archive on disk has moved since the link, say so loudly: the numbers are
    still valid for the linked bytes, but they are not about the current tree."""
    with open(os.path.join(BUILD, "link-manifest.json")) as f:
        man = json.load(f)
    for tag, path in (("rust", os.path.join(ROOT, "target", "release", "libtinycbor.a")),
                      ("c_reference", os.path.join(HERE, "reference", "libtinycbor-upstream.a"))):
        current = sha256_file(path) if os.path.exists(path) else None
        man[tag]["matches_current_tree"] = (current == man[tag]["sha256"])
        if not man[tag]["matches_current_tree"]:
            man[tag]["current_tree_sha256"] = current
            print("  !! %s archive changed since link; rerun bench/build.sh to "
                  "measure the current tree" % tag, file=sys.stderr)
    return man


def environment(pin):
    gov = read_first("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
    boost = read_first("/sys/devices/system/cpu/intel_pstate/no_turbo")
    try:
        cc = run(["cc", "--version"]).stdout.splitlines()[0]
    except Exception:
        cc = "unknown"
    try:
        rustc = run(["rustc", "--version"]).stdout.strip()
    except Exception:
        rustc = "unknown"
    return {
        "cpu_model": cpu_model(),
        "cpu_count": os.cpu_count(),
        "scaling_governor": gov,
        "intel_pstate_no_turbo": boost,
        "kernel": platform.release(),
        "loadavg": os.getloadavg(),
        "cc_version": cc,
        "rustc_version": rustc,
        "c_reference_cflags": "-O3 -DNDEBUG (CMAKE_BUILD_TYPE=Release, upstream CMakeCache)",
        "rust_profile": "release, lto=true, codegen-units=1, panic=abort",
        "pinned_to_cpu": pin,
        "libs_as_linked": linked_libs(),
        "timestamp_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }


def corpus_files():
    with open(os.path.join(CORPUS, "manifest.json")) as f:
        man = json.load(f)
    return man["files"]


def check_equivalence(prefix):
    """Both builds pretty-print every corpus file; identical bytes or it gets
    flagged. A timing win that comes from emitting different output is not a
    timing win."""
    out = {}
    for entry in corpus_files():
        name = entry["file"]
        path = os.path.join(CORPUS, name)
        digests = {}
        for tag, exe in BINARIES.items():
            p = subprocess.run(prefix + [exe, "dump", path, "1"], capture_output=True, check=True)
            digests[tag] = (hashlib.sha256(p.stdout).hexdigest(), len(p.stdout))
        same = digests["c"][0] == digests["rust"][0]
        out[name] = {
            "identical": same,
            "c_sha256": digests["c"][0],
            "rust_sha256": digests["rust"][0],
            "c_output_bytes": digests["c"][1],
            "rust_output_bytes": digests["rust"][1],
        }
        print("  %-18s %s" % (name, "identical" if same else "*** MISMATCH ***"))
    return out


def block(prefix, exe, mode, path, reps):
    p = run(prefix + [exe, mode, path, str(reps)])
    return json.loads(p.stdout)


def calibrate(prefix, mode, path):
    """Pick a rep count from the slower of the two builds so both do the same
    amount of work per block."""
    worst = 0
    for exe in BINARIES.values():
        r = block(prefix, exe, mode, path, 3)
        worst = max(worst, sorted(r["ns"])[1])
    if worst == 0:
        return MAX_REPS
    return max(MIN_REPS, min(MAX_REPS, TARGET_BLOCK_NS // worst))


def throughput(prefix, files):
    results = {}
    for mode in MODES:
        for entry in files:
            name, nbytes = entry["file"], entry["bytes"]
            path = os.path.join(CORPUS, name)
            reps = calibrate(prefix, mode, path)
            warm = max(2, int(reps * WARMUP_FRAC))

            samples = {"c": [], "rust": []}
            rss = {"c": 0, "rust": 0}
            timer = 0
            for _ in range(ROUNDS):
                for tag, exe in BINARIES.items():  # alternates c, rust, c, rust...
                    r = block(prefix, exe, mode, path, reps)
                    samples[tag].extend(r["ns"][warm:])
                    rss[tag] = max(rss[tag], r["peak_rss_kib"])
                    timer = max(timer, r["timer_overhead_ns"])

            entry_out = {"bytes": nbytes, "reps_per_block": reps, "warmup_discarded": warm,
                         "rounds": ROUNDS, "timer_overhead_ns": timer}
            for tag in BINARIES:
                s = sorted(samples[tag])
                p50, p99 = pct(s, 50), pct(s, 99)
                entry_out[tag] = {
                    "samples": len(s),
                    "p50_ns": p50,
                    "p99_ns": p99,
                    "min_ns": s[0],
                    "max_ns": s[-1],
                    "p50_mb_per_s": round(nbytes / p50 * 1000.0, 2),
                    "peak_rss_kib": rss[tag],
                }
            c50, r50 = entry_out["c"]["p50_ns"], entry_out["rust"]["p50_ns"]
            c99, r99 = entry_out["c"]["p99_ns"], entry_out["rust"]["p99_ns"]
            entry_out["rust_vs_c_p50"] = round(r50 / c50, 4)
            entry_out["rust_vs_c_p99"] = round(r99 / c99, 4)
            entry_out["verdict"] = "rust faster" if r50 < c50 else "rust slower"
            results.setdefault(mode, {})[name] = entry_out
            print("  %-7s %-18s c p50 %9d ns   rust p50 %9d ns   %5.2fx  %s"
                  % (mode, name, c50, r50, r50 / c50, entry_out["verdict"]))
    return results


def startup(prefix, tmp):
    """Process startup: exec + dynamic loader + libc/runtime init + one parse of
    the 58-byte file. Whatever a Rust staticlib costs to start up shows here."""
    path = os.path.join(CORPUS, "small_ints.cbor")
    out = {}
    for tag, exe in BINARIES.items():
        js = os.path.join(tmp, "hf-%s.json" % tag)
        cmd = ["hyperfine", "-N", "--warmup", str(HYPERFINE_WARMUP),
               "--runs", str(HYPERFINE_RUNS), "--export-json", js,
               " ".join(prefix + [exe, "parse", path, "1"])]
        run(cmd)
        with open(js) as f:
            times = sorted(t * 1e9 for t in json.load(f)["results"][0]["times"])
        out[tag] = {
            "runs": len(times),
            "p50_ns": int(pct(times, 50)),
            "p99_ns": int(pct(times, 99)),
            "min_ns": int(times[0]),
            "binary_bytes": os.path.getsize(exe),
        }
        print("  %-5s startup p50 %8d ns   p99 %8d ns   binary %d B"
              % (tag, out[tag]["p50_ns"], out[tag]["p99_ns"], out[tag]["binary_bytes"]))
    out["rust_vs_c_p50"] = round(out["rust"]["p50_ns"] / out["c"]["p50_ns"], 4)
    out["rust_vs_c_p99"] = round(out["rust"]["p99_ns"] / out["c"]["p99_ns"], 4)
    out["verdict"] = "rust faster" if out["rust_vs_c_p50"] < 1 else "rust slower"
    return out


def main():
    for exe in BINARIES.values():
        if not os.path.exists(exe):
            sys.exit("missing %s -- run bench/build.sh first" % exe)
    if not shutil.which("hyperfine"):
        sys.exit("hyperfine not found on PATH")

    pin = os.environ.get("BENCH_CPU", "2")
    prefix = ["taskset", "-c", pin] if shutil.which("taskset") else []
    if not prefix:
        pin = None

    tmp = os.path.join(BUILD, "hyperfine")
    os.makedirs(tmp, exist_ok=True)

    print("== output equivalence ==")
    equiv = check_equivalence(prefix)
    print("== startup (hyperfine) ==")
    su = startup(prefix, tmp)
    print("== throughput ==")
    tp = throughput(prefix, corpus_files())

    losses = [("startup", "-", su["rust_vs_c_p50"])] if su["rust_vs_c_p50"] > 1 else []
    for mode, files in tp.items():
        for name, e in files.items():
            if e["rust_vs_c_p50"] > 1:
                losses.append((mode, name, e["rust_vs_c_p50"]))
    losses.sort(key=lambda t: -t[2])

    doc = {
        "environment": environment(pin),
        "output_equivalence": equiv,
        "startup": su,
        "throughput": tp,
        "regressions_rust_slower_at_p50": [
            {"mode": m, "file": f, "rust_vs_c_p50": r} for m, f, r in losses
        ],
        "notes": [
            "rust_vs_c ratios are rust/c: >1.0 means the Rust port is SLOWER.",
            "Percentiles are nearest-rank over pooled per-repetition samples; no "
            "outliers were removed. Only the leading warmup reps of each block "
            "are discarded, and that count is recorded per entry.",
            "Any corpus file with output_equivalence.identical == false is NOT a "
            "fair timing comparison: the two builds emit different bytes and "
            "therefore do different amounts of work.",
            "peak_rss_kib is /proc/self/status VmHWM, not getrusage ru_maxrss: "
            "ru_maxrss survives fork/exec on Linux and reports the harness's "
            "high-water mark, not the driver's. It includes the input file buffer.",
            "cbor_value_to_json_advance is not benchmarked: it is unimplemented "
            "in the Rust port.",
            "parse mode calls cbor_value_advance() on the top-level value, which "
            "walks the structure but does not read text/byte string payloads -- it "
            "only skips over them. Its p50_mb_per_s is therefore a structural "
            "traversal rate, NOT a bytes-consumed rate, and is meaningless for the "
            "string-heavy files (text_utf8, bytes_heavy). pretty mode touches every "
            "byte and its MB/s figure is real.",
        ],
    }
    with open(os.path.join(HERE, "results.json"), "w") as f:
        json.dump(doc, f, indent=2)
        f.write("\n")
    print("\nwrote bench/results.json")
    if losses:
        total = 1 + len(MODES) * len(corpus_files())
        print("Rust is SLOWER at p50 in %d of %d measurements:" % (len(losses), total))
        for m, f, r in losses:
            print("  %-7s %-18s %.2fx" % (m, f, r))
    else:
        print("Rust is at or ahead of C everywhere measured.")


if __name__ == "__main__":
    main()
