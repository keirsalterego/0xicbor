/* cbor_value_dup_{text,byte}_string, which upstream's Qt suite never calls.
 *
 * Nothing under tests/original/ reaches _cbor_value_dup_string, so a port could
 * leave it stubbed and still show 4929/4929. This covers it, and it is written
 * in C rather than Rust because what needs testing is the ABI contract: the
 * caller gets a pointer it releases with free(), which means the block has to
 * come from libc's allocator and not Rust's.
 *
 * The transcript on stdout is deterministic, so `make test-port` builds this
 * once against the Rust archive and once against upstream's and diffs the two.
 * Any divergence is a real one; there is no expected-output file to drift.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "cbor.h"

static void hex(const unsigned char *p, size_t n)
{
    size_t i;
    for (i = 0; i < n; ++i)
        printf("%02x", p[i]);
}

/* Dumps everything the API promised: the bytes, the length, the NUL that is
 * supposed to sit past them, and where the cursor ended up.
 *
 * Only on success. The documentation is explicit that after any error other
 * than out-of-memory, *buffer, *buflen and next "are undefined and mustn't be
 * used" -- and neither implementation writes next on those paths, so printing
 * it reads whatever the stack held. An earlier version of this test did exactly
 * that and reported a divergence that was entirely its own. */
static void report(const char *what, CborError err, void *buffer, size_t buflen,
                   const CborValue *next)
{
    printf("%-22s err=%d", what, (int)err);
    if (err == CborNoError) {
        printf(" len=%lu buf=", (unsigned long)buflen);
        hex((const unsigned char *)buffer, buflen);
        /* The documented behaviour is a terminating NUL beyond the content. */
        printf(" nul=%d", ((const unsigned char *)buffer)[buflen] == 0);
        if (next != NULL)
            printf(" next_type=%d", (int)cbor_value_get_type(next));
    }
    printf("\n");
}

static void dup_text(const char *what, const uint8_t *data, size_t len, int alias)
{
    CborParser parser;
    CborValue it, next;
    char *buffer = NULL;
    size_t buflen = 0;
    CborError err;

    err = cbor_parser_init(data, len, 0, &parser, &it);
    if (err) {
        printf("%-22s init err=%d\n", what, (int)err);
        return;
    }
    /* alias: pass the same cursor as both source and destination, which is what
     * upstream's own comment says callers usually do. */
    err = cbor_value_dup_text_string(&it, &buffer, &buflen, alias ? &it : &next);
    report(what, err, buffer, buflen, alias ? &it : &next);
    free(buffer);
}

static void dup_bytes(const char *what, const uint8_t *data, size_t len)
{
    CborParser parser;
    CborValue it, next;
    uint8_t *buffer = NULL;
    size_t buflen = 0;
    CborError err;

    err = cbor_parser_init(data, len, 0, &parser, &it);
    if (err) {
        printf("%-22s init err=%d\n", what, (int)err);
        return;
    }
    err = cbor_value_dup_byte_string(&it, &buffer, &buflen, &next);
    report(what, err, buffer, buflen, &next);
    free(buffer);
}

int main(void)
{
    /* "hello" */
    static const uint8_t text[] = {0x65, 'h', 'e', 'l', 'l', 'o'};
    /* "" -- a zero-length string still has to return a freeable pointer. */
    static const uint8_t empty[] = {0x60};
    /* (_ "str", "eaming") -- indefinite length, reassembled across chunks. */
    static const uint8_t chunked[] = {0x7f, 0x63, 's',  't', 'r', 0x66, 'e',
                                      'a',  'm',  'i',  'n', 'g', 0xff};
    /* h'01020304' */
    static const uint8_t bytes[] = {0x44, 0x01, 0x02, 0x03, 0x04};
    /* h'' */
    static const uint8_t nobytes[] = {0x40};
    /* (_ h'0102', h'03') */
    static const uint8_t chunked_bytes[] = {0x5f, 0x42, 0x01, 0x02, 0x41, 0x03, 0xff};
    /* ["hi", 1] -- next must land on the 1. */
    static const uint8_t in_array[] = {0x82, 0x62, 'h', 'i', 0x01};
    /* A text string claiming five bytes with four present. */
    static const uint8_t truncated[] = {0x65, 'h', 'e', 'l', 'l'};
    /* An indefinite text string whose chunk is a byte string. */
    static const uint8_t mistyped_chunk[] = {0x7f, 0x42, 0x01, 0x02, 0xff};

    dup_text("text", text, sizeof(text), 0);
    dup_text("text_aliased_next", text, sizeof(text), 1);
    dup_text("text_empty", empty, sizeof(empty), 0);
    dup_text("text_chunked", chunked, sizeof(chunked), 0);
    dup_text("text_truncated", truncated, sizeof(truncated), 0);
    dup_text("text_mistyped_chunk", mistyped_chunk, sizeof(mistyped_chunk), 0);

    dup_bytes("bytes", bytes, sizeof(bytes));
    dup_bytes("bytes_empty", nobytes, sizeof(nobytes));
    dup_bytes("bytes_chunked", chunked_bytes, sizeof(chunked_bytes));

    /* Inside a container, so `next` has somewhere real to point. */
    {
        CborParser parser;
        CborValue it, arr;
        char *buffer = NULL;
        size_t buflen = 0;
        CborError err = cbor_parser_init(in_array, sizeof(in_array), 0, &parser, &it);
        if (!err)
            err = cbor_value_enter_container(&it, &arr);
        if (!err) {
            err = cbor_value_dup_text_string(&arr, &buffer, &buflen, &arr);
            report("text_in_array", err, buffer, buflen, &arr);
            free(buffer);
        } else {
            printf("%-22s setup err=%d\n", "text_in_array", (int)err);
        }
    }

    return 0;
}
