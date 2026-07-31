/*
 * Benchmark driver.  Compiled twice by bench/build.sh from this one file: once
 * against the upstream C libtinycbor.a, once against our Rust staticlib.  The
 * header is byte-identical for both (build.sh asserts it), so the two binaries
 * differ only in which archive the linker pulled in -- which is the entire
 * point of an ABI-compatible port.
 *
 *   usage: driver <mode> <file.cbor> <reps>
 *     mode = pretty   parse and cbor_value_to_pretty_advance() into /dev/null
 *     mode = parse    parse only: cbor_value_advance() over the whole document
 *     mode = dump     one pretty pass to stdout, no timing -- this is how the
 *                     harness proves the two builds emit identical bytes before
 *                     it believes any timing number
 *
 * Each rep is timed on its own with CLOCK_MONOTONIC so the harness can take
 * percentiles rather than an average.  Peak RSS comes from /proc/self/status
 * VmHWM -- see peak_rss_kib() for why not getrusage.  Result goes to stdout as
 * JSON; the pretty output itself goes to /dev/null and never touches stdout.
 */

#define _POSIX_C_SOURCE 200809L

#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include <cbor.h>

static uint8_t *slurp(const char *path, size_t *len)
{
    FILE *f = fopen(path, "rb");
    uint8_t *buf;
    long n;

    if (!f) {
        fprintf(stderr, "driver: %s: %s\n", path, strerror(errno));
        exit(2);
    }
    if (fseek(f, 0, SEEK_END) != 0 || (n = ftell(f)) < 0) {
        fprintf(stderr, "driver: %s: not seekable\n", path);
        exit(2);
    }
    rewind(f);
    buf = malloc((size_t)n ? (size_t)n : 1);
    if (!buf || fread(buf, 1, (size_t)n, f) != (size_t)n) {
        fprintf(stderr, "driver: %s: short read\n", path);
        exit(2);
    }
    fclose(f);
    *len = (size_t)n;
    return buf;
}

static uint64_t now_ns(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}

/*
 * Peak RSS, in KiB.
 *
 * NOT getrusage(RUSAGE_SELF).ru_maxrss.  On Linux that counter is inherited
 * across fork() and is not reset by execve(), so a driver spawned from a fat
 * parent reports the parent's high-water mark instead of its own: the identical
 * run measured here reports 2504 KiB under a small shell and 43128 KiB under a
 * Python harness that had touched 32 MB.  VmHWM lives in the mm, which execve
 * replaces, so it is the honest per-process number and is what `/usr/bin/time
 * -v` would have wanted to report.  Returns -1 if /proc is not mounted.
 */
static long peak_rss_kib(void)
{
    FILE *f = fopen("/proc/self/status", "r");
    char line[256];
    long kib = -1;

    if (!f)
        return -1;
    while (fgets(line, sizeof(line), f)) {
        if (strncmp(line, "VmHWM:", 6) == 0) {
            kib = strtol(line + 6, NULL, 10);
            break;
        }
    }
    fclose(f);
    return kib;
}

/* Cost of one back-to-back now_ns() pair.  Reported so a reader can tell how
 * much of a sub-microsecond result is the clock rather than the library.  It is
 * the same tax on both builds, so it never changes which side wins -- it only
 * pulls the ratio toward 1.0 on the tiny corpus files. */
static uint64_t timer_overhead_ns(void)
{
    uint64_t best = (uint64_t)-1;
    int i;

    for (i = 0; i < 1000; i++) {
        uint64_t a = now_ns();
        uint64_t d = now_ns() - a;
        if (d < best)
            best = d;
    }
    return best;
}

int main(int argc, char **argv)
{
    const char *mode, *path;
    uint8_t *buf;
    size_t len;
    long reps, i;
    uint64_t *ns;
    FILE *devnull;
    int pretty;

    if (argc != 4) {
        fprintf(stderr, "usage: %s <pretty|parse|dump> <file.cbor> <reps>\n", argv[0]);
        return 2;
    }
    mode = argv[1];
    path = argv[2];
    reps = strtol(argv[3], NULL, 10);
    if (reps < 1) {
        fprintf(stderr, "driver: reps must be >= 1\n");
        return 2;
    }
    if (strcmp(mode, "dump") == 0) {
        CborParser parser;
        CborValue it;
        CborError err;

        buf = slurp(path, &len);
        err = cbor_parser_init(buf, len, 0, &parser, &it);
        if (err == CborNoError)
            err = cbor_value_to_pretty_advance(stdout, &it);
        if (err != CborNoError) {
            fprintf(stderr, "driver: %s: %s\n", path, cbor_error_string(err));
            return 1;
        }
        return fflush(stdout) == 0 ? 0 : 1;
    }
    if (strcmp(mode, "pretty") == 0) {
        pretty = 1;
    } else if (strcmp(mode, "parse") == 0) {
        pretty = 0;
    } else {
        fprintf(stderr, "driver: unknown mode '%s'\n", mode);
        return 2;
    }

    buf = slurp(path, &len);
    ns = malloc(sizeof(*ns) * (size_t)reps);
    if (!ns) {
        fprintf(stderr, "driver: out of memory\n");
        return 2;
    }

    devnull = fopen("/dev/null", "w");
    if (!devnull) {
        fprintf(stderr, "driver: /dev/null: %s\n", strerror(errno));
        return 2;
    }

    for (i = 0; i < reps; i++) {
        CborParser parser;
        CborValue it;
        CborError err;
        uint64_t t0 = now_ns();

        err = cbor_parser_init(buf, len, 0, &parser, &it);
        if (err == CborNoError)
            err = pretty ? cbor_value_to_pretty_advance(devnull, &it)
                         : cbor_value_advance(&it);

        ns[i] = now_ns() - t0;

        if (err != CborNoError) {
            fprintf(stderr, "driver: %s: %s\n", path, cbor_error_string(err));
            return 1;
        }
    }
    fflush(devnull);

    printf("{\"file\":\"%s\",\"mode\":\"%s\",\"bytes\":%zu,\"reps\":%ld,"
           "\"peak_rss_kib\":%ld,\"timer_overhead_ns\":%llu,\"ns\":[",
           path, mode, len, reps, peak_rss_kib(),
           (unsigned long long)timer_overhead_ns());
    for (i = 0; i < reps; i++)
        printf(i ? ",%llu" : "%llu", (unsigned long long)ns[i]);
    printf("]}\n");

    free(ns);
    free(buf);
    fclose(devnull);
    return 0;
}
