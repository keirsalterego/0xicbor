/* Replays a saved CBOR input through cbor_value_advance and prints everything
 * observable afterwards, so the same source linked against either archive can
 * be diffed.
 *
 *     tst_advance_diff FILE...
 *
 * `cbor_value_advance` skips the item under the cursor and everything nested
 * inside it. Only two things survive that walk: where the cursor ends up and
 * which error, if any, came back. Both are printed, along with the decoded
 * head of whatever the cursor landed on, which is the rest of what a caller
 * can see.
 *
 * This exists because the Rust port skips the subtree with a flat scan instead
 * of a recursive descent when it can, and the original suite reaches
 * cbor_value_advance mostly through pretty-printing rather than directly.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "cbor.h"

#define INPUT_CAP (1 << 20)

static int replay(const char *path)
{
    static unsigned char input[INPUT_CAP];
    CborParser parser;
    CborValue it;
    CborError err;
    size_t n;
    FILE *in;
    const char *name;

    in = fopen(path, "rb");
    if (!in) {
        printf("%-44s OPEN FAILED\n", path);
        return 1;
    }
    n = fread(input, 1, sizeof input, in);
    fclose(in);

    name = strrchr(path, '/');
    name = name ? name + 1 : path;

    err = cbor_parser_init(input, n, 0, &parser, &it);
    if (err) {
        printf("%-44s in=%lu init=%d\n", name, (unsigned long)n, (int)err);
        return 0;
    }

    err = cbor_value_advance(&it);

    /* Offset rather than the raw pointer, so the transcript does not depend on
     * where the buffer happened to land. */
    printf("%-44s in=%lu err=%d off=%ld", name, (unsigned long)n, (int)err,
           (long)(cbor_value_get_next_byte(&it) - input));
    if (!err)
        printf(" type=%d at_end=%d", (int)cbor_value_get_type(&it),
               (int)cbor_value_at_end(&it));
    printf("\n");
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
