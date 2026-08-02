/* Parses the same bytes twice, once through a flat buffer and once through a
 * caller-supplied reader, and prints both results so they can be compared.
 *
 *     tst_reader_diff FILE...
 *
 * The two source kinds are meant to be indistinguishable to a caller: one reads
 * a pointer into a buffer, the other calls back for every byte, and the answer
 * is supposed to be the same either way. Nothing checked that.
 *
 * Upstream's suite exercises `cbor_parser_init_reader` in one test function
 * fed one test function's worth of data, and both of this port's fuzzers use
 * buffer sources exclusively. The subtree scan added later is buffer-only by
 * construction, so the reader path is also the fallback that nothing else
 * reaches. That is three reasons the reader side is the least-walked code in
 * the library and none of them are reasons to trust it.
 *
 * Two things are checked here. Within one library, buffer and reader must
 * agree; that is printed as `same=1`. Across the two libraries, the whole
 * transcript must match, which is what `make test` diffs.
 *
 * The operations table is modelled on the one in upstream's tst_parser.cpp so
 * that a divergence is about the parser rather than about a reader written to
 * flatter it.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "cbor.h"

#define INPUT_CAP (1 << 20)

/* The token handed to every callback: the bytes, and how far in we are. */
typedef struct {
    const unsigned char *data;
    size_t size;
    size_t consumed;
} Input;

static bool reader_can_read(void *token, size_t len)
{
    Input *in = token;
    return in->size - in->consumed >= len;
}

static void *reader_read(void *token, void *dst, size_t offset, size_t len)
{
    Input *in = token;
    return memcpy(dst, in->data + in->consumed + offset, len);
}

static void reader_advance(void *token, size_t len)
{
    Input *in = token;
    in->consumed += len;
}

static CborError reader_transfer_string(void *token, const void **userptr, size_t offset,
                                        size_t len)
{
    Input *in = token;
    if (in->size - in->consumed < len + offset)
        return CborErrorUnexpectedEOF;
    in->consumed += offset;
    *userptr = in->data + in->consumed;
    in->consumed += len;
    return CborNoError;
}

static const struct CborParserOperations ops = {
    reader_can_read,
    reader_read,
    reader_advance,
    reader_transfer_string,
};

/* Renders through whichever source, into `out`. Returns the CborError. */
static CborError render_buffer(const unsigned char *data, size_t n, char *out, size_t cap)
{
    CborParser parser;
    CborValue it;
    CborError err;
    FILE *sink = fmemopen(out, cap, "w");
    if (!sink)
        return CborErrorInternalError;
    err = cbor_parser_init(data, n, 0, &parser, &it);
    if (!err)
        err = cbor_value_to_pretty_advance(sink, &it);
    fclose(sink);
    return err;
}

static CborError render_reader(const unsigned char *data, size_t n, char *out, size_t cap)
{
    CborParser parser;
    CborValue it;
    CborError err;
    Input in = { data, n, 0 };
    FILE *sink = fmemopen(out, cap, "w");
    if (!sink)
        return CborErrorInternalError;
    err = cbor_parser_init_reader(&ops, &parser, &it, &in);
    if (!err)
        err = cbor_value_to_pretty_advance(sink, &it);
    fclose(sink);
    return err;
}

static int replay(const char *path)
{
    static unsigned char input[INPUT_CAP];
    static char from_buffer[INPUT_CAP];
    static char from_reader[INPUT_CAP];
    const char *name;
    size_t n;
    FILE *in;
    CborError berr, rerr;

    in = fopen(path, "rb");
    if (!in) {
        printf("%-34s OPEN FAILED\n", path);
        return 1;
    }
    n = fread(input, 1, sizeof input, in);
    fclose(in);

    memset(from_buffer, 0, sizeof from_buffer);
    memset(from_reader, 0, sizeof from_reader);
    berr = render_buffer(input, n, from_buffer, sizeof from_buffer);
    rerr = render_reader(input, n, from_reader, sizeof from_reader);

    name = strrchr(path, '/');
    name = name ? name + 1 : path;

    /* Output is only comparable when the render finished: both libraries
     * stream, but a failed render stops at a different point depending on
     * which source noticed, and this port buffers rather than streaming. The
     * error code is comparable always. */
    printf("%-34s in=%lu buffer=%d reader=%d same=%d\n", name, (unsigned long)n, (int)berr,
           (int)rerr,
           berr == rerr && (berr != CborNoError || strcmp(from_buffer, from_reader) == 0));
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
