/* Replays a saved CBOR input through cbor_value_to_pretty_advance and prints
 * what happened, so the same source linked against either archive can be
 * diffed.
 *
 * This is where inputs the differential fuzzer found go to stay found. The
 * fuzzer explores; a corpus file under tests/port/corpus/ is a specific thing
 * that once broke, checked on every `make test` without waiting for libFuzzer
 * to rediscover it.
 *
 *     tst_pretty_diff FILE...
 *
 * Note what is *not* printed on the error paths: anything about the output.
 * Upstream streams as it goes, so a render that fails half way has already
 * emitted the first half; this port buffers and emits once, so it emits
 * nothing. Neither the bytes nor even their length are comparable there. That
 * difference is deliberate and documented in crates/cbor-ffi/src/pretty.rs, and
 * the differential fuzzer tolerates it for the same reason. On success the
 * bytes must match exactly, and they are printed in full.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "cbor.h"

/* Not MAX_INPUT: <linux/limits.h> already has that, via stdio on glibc. */
#define INPUT_CAP (1 << 20)

static int replay(const char *path)
{
    static unsigned char input[INPUT_CAP];
    static char rendered[INPUT_CAP * 8];
    CborParser parser;
    CborValue it;
    CborError err;
    size_t n;
    FILE *in, *sink;

    in = fopen(path, "rb");
    if (!in) {
        printf("%-34s OPEN FAILED\n", path);
        return 1;
    }
    n = fread(input, 1, sizeof input, in);
    fclose(in);

    memset(rendered, 0, sizeof rendered);
    sink = fmemopen(rendered, sizeof rendered, "w");
    if (!sink) {
        printf("%-34s fmemopen failed\n", path);
        return 1;
    }

    err = cbor_parser_init(input, n, 0, &parser, &it);
    if (!err)
        err = cbor_value_to_pretty_advance(sink, &it);
    fclose(sink);

    /* basename, so the transcript does not depend on where the tree lives */
    const char *name = strrchr(path, '/');
    name = name ? name + 1 : path;

    if (err == CborNoError)
        printf("%-34s in=%lu err=0 out=%s\n", name, (unsigned long)n, rendered);
    else
        printf("%-34s in=%lu err=%d\n", name, (unsigned long)n, (int)err);
    return 0;
}

int main(int argc, char **argv)
{
    int i, bad = 0;
    if (argc < 2) {
        fprintf(stderr, "usage: %s FILE...\n", argv[0]);
        return 2;
    }
    for (i = 1; i < argc; ++i)
        bad |= replay(argv[i]);
    return bad;
}
