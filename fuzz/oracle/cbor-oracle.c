/*
 * cbor-oracle -- the reference side of the differential fuzzer.
 *
 * This is upstream intel/tinycbor (commit 9441b2ca), unmodified, wrapped in a
 * main(). It reads CBOR bytes on stdin, runs cbor_parser_init() followed by
 * one of the two renderers, writes the result to stdout, and exits with the
 * CborError.
 *
 *     cbor-oracle                  diagnostic notation, the default
 *     cbor-oracle json FLAGS       JSON, with that CborToJsonFlags bitmask
 *     cbor-oracle validate FLAGS   cbor_value_validate, that CborValidationFlags
 *                                  bitmask, no output beyond the error code
 *     cbor-oracle encode           runs stdin as an encoder program; see below
 *
 * No arguments keeps the original behaviour, so the pretty harness spawns it
 * exactly as it always did.
 *
 * IT IS DELIBERATELY A SEPARATE PROCESS, AND THIS FILE LIVES OUTSIDE crates/.
 *
 * The port ships as libtinycbor.a with zero C in it: no cc crate, no bindgen,
 * no build.rs compiling C, and nothing under crates/ links against upstream.
 * Running the oracle out-of-process is what makes that claim checkable instead
 * of merely stated -- there is no build configuration under which this
 * translation unit can end up inside the shipped Rust artifact. The harness
 * talks to it over a pipe and compares bytes.
 *
 * Error reporting: CborError is an int running up to INT_MAX
 * (CborErrorInternalError), while a process exit status carries only 8 bits.
 * The untruncated value is therefore also written to stderr as a bare decimal,
 * and that is what the harness compares. The exit status is still set from the
 * error so the program stays useful by hand.
 *
 * Build: fuzz/oracle/build.sh
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <cbor.h>
#include <cborjson.h>

/* Matches MAX_INPUT in the fuzz target. Inputs are small; this is a backstop. */
#define MAX_INPUT (1 << 20)

/*
 * The encoder program.
 *
 * Everything else here reads CBOR. Half of tinycbor writes it, and no
 * differential test reached any of it, because there is no input to feed an
 * encoder -- it takes calls, not bytes. So the fuzzer's bytes are read as the
 * calls: a two-byte output buffer size, then a stream of opcodes.
 *
 *     bytes 0..1   output buffer size, little endian, modulo ENC_BUF_MAX;
 *                  bit 15 selects the writer-callback encoder instead
 *     bytes 2..    opcodes, each one byte followed by its operands
 *
 * Bit 15 picks between the two ways an encoder can exist. Clear, and it writes
 * into a flat buffer, which is what cbor_encoder_init does. Set, and it calls
 * back for every fragment, which is cbor_encoder_init_writer -- a separate path
 * through the same append(), with its own idea of what running out of room
 * means. The callback records the CborEncoderAppendType it was handed alongside
 * the bytes, because that argument is ABI surface with nothing else checking it,
 * and refuses everything past the same size limit so the error coming *back*
 * through the encoder is exercised too.
 *
 * The opcode is taken modulo the table size so that random bytes are all
 * meaningful, and an operand running off the end of the input ends the program.
 * Small buffers are the point: most of the encoder's interesting behaviour is
 * what it does once it has run out of room, where it stops writing, keeps
 * counting, and reports how much more it needed.
 *
 * The transcript is every call's CborError as a four-byte little-endian int,
 * then the whole output buffer, then cbor_encoder_get_extra_bytes_needed. The
 * buffer is dumped whole rather than up to the write cursor because that cursor
 * is past the end once the encoder has overrun, and the comparison wants
 * something well defined on exactly the inputs that matter.
 *
 * This interpreter and the one in fuzz_targets/encode_diff.rs are written from
 * this comment. They have to agree about the program before they can disagree
 * about the encoder, so a divergence in the first few hundred executions is far
 * more likely to be the two readers than the two encoders.
 */
#define ENC_BUF_MAX 1024
#define ENC_MAX_DEPTH 16
#define ENC_OPS 19

/* Reads `n` bytes at `*pos`, or returns 0 having left `*pos` alone. */
static int enc_take(const unsigned char *p, size_t len, size_t *pos, size_t n,
                    const unsigned char **out)
{
    if (len - *pos < n)
        return 0;
    *out = p + *pos;
    *pos += n;
    return 1;
}

static uint64_t enc_u64(const unsigned char *b)
{
    uint64_t v = 0;
    int i;
    for (i = 7; i >= 0; --i)
        v = (v << 8) | b[i];
    return v;
}

static void enc_emit(unsigned char *out, size_t *n, CborError err)
{
    unsigned u = (unsigned)err;
    out[(*n)++] = u & 0xff;
    out[(*n)++] = (u >> 8) & 0xff;
    out[(*n)++] = (u >> 16) & 0xff;
    out[(*n)++] = (u >> 24) & 0xff;
}

/* What the writer callback records: one (type, length, bytes) entry per call. */
static unsigned char enc_log[ENC_BUF_MAX * 4];
static size_t enc_log_len;
static size_t enc_log_accepted;
static size_t enc_log_cap;

static CborError enc_writer(void *token, const void *data, size_t len,
                            CborEncoderAppendType append_type)
{
    (void)token;
    /* Refuse past the cap, so the encoder has to carry an error back out of the
     * callback rather than only ever seeing success. */
    if (enc_log_accepted + len > enc_log_cap)
        return CborErrorOutOfMemory;
    /* Room in the log itself is a harness limit, not an encoder one. */
    if (enc_log_len + 3 + len > sizeof(enc_log))
        return CborErrorOutOfMemory;
    enc_log[enc_log_len++] = (unsigned char)append_type;
    enc_log[enc_log_len++] = len & 0xff;
    enc_log[enc_log_len++] = (len >> 8) & 0xff;
    memcpy(enc_log + enc_log_len, data, len);
    enc_log_len += len;
    enc_log_accepted += len;
    return CborNoError;
}

static CborError run_encoder_program(const unsigned char *prog, size_t len)
{
    static unsigned char outbuf[ENC_BUF_MAX];
    /* Four bytes of transcript per op, and an op is at least one byte. */
    static unsigned char transcript[4 * MAX_INPUT + ENC_BUF_MAX + 8];
    CborEncoder stack[ENC_MAX_DEPTH + 1];
    size_t pos = 0, tn = 0, bufsize, needed, header;
    int depth = 0, use_writer;
    const unsigned char *b;
    CborError err = CborNoError;

    if (len < 2)
        return CborNoError;
    header = (size_t)prog[0] | ((size_t)prog[1] << 8);
    use_writer = (header & 0x8000) != 0;
    bufsize = (header & 0x7fff) % (ENC_BUF_MAX + 1);
    pos = 2;

    memset(outbuf, 0, sizeof(outbuf));
    enc_log_len = enc_log_accepted = 0;
    enc_log_cap = bufsize;
    if (use_writer)
        cbor_encoder_init_writer(&stack[0], enc_writer, NULL);
    else
        cbor_encoder_init(&stack[0], outbuf, bufsize, 0);

    while (pos < len) {
        unsigned op = prog[pos++] % ENC_OPS;
        CborEncoder *cur = &stack[depth];

        switch (op) {
        case 0:
            if (!enc_take(prog, len, &pos, 8, &b)) goto done;
            err = cbor_encode_uint(cur, enc_u64(b));
            break;
        case 1:
            if (!enc_take(prog, len, &pos, 8, &b)) goto done;
            err = cbor_encode_int(cur, (int64_t)enc_u64(b));
            break;
        case 2:
            if (!enc_take(prog, len, &pos, 8, &b)) goto done;
            err = cbor_encode_negative_int(cur, enc_u64(b));
            break;
        case 3:
            if (!enc_take(prog, len, &pos, 1, &b)) goto done;
            err = cbor_encode_simple_value(cur, b[0]);
            break;
        case 4:
            if (!enc_take(prog, len, &pos, 8, &b)) goto done;
            err = cbor_encode_tag(cur, enc_u64(b));
            break;
        case 5: {
            const unsigned char *s;
            if (!enc_take(prog, len, &pos, 1, &b)) goto done;
            if (!enc_take(prog, len, &pos, b[0], &s)) goto done;
            err = cbor_encode_text_string(cur, (const char *)s, b[0]);
            break;
        }
        case 6: {
            const unsigned char *s;
            if (!enc_take(prog, len, &pos, 1, &b)) goto done;
            if (!enc_take(prog, len, &pos, b[0], &s)) goto done;
            err = cbor_encode_byte_string(cur, s, b[0]);
            break;
        }
        case 7: {
            float f;
            if (!enc_take(prog, len, &pos, 4, &b)) goto done;
            memcpy(&f, b, 4);
            err = cbor_encode_float(cur, f);
            break;
        }
        case 8: {
            double d;
            if (!enc_take(prog, len, &pos, 8, &b)) goto done;
            memcpy(&d, b, 8);
            err = cbor_encode_double(cur, d);
            break;
        }
        case 9: {
            uint16_t h;
            if (!enc_take(prog, len, &pos, 2, &b)) goto done;
            memcpy(&h, b, 2);
            err = cbor_encode_half_float(cur, &h);
            break;
        }
        case 10: {
            float f;
            if (!enc_take(prog, len, &pos, 4, &b)) goto done;
            memcpy(&f, b, 4);
            err = cbor_encode_float_as_half_float(cur, f);
            break;
        }
        case 11: {
            const unsigned char *s;
            if (!enc_take(prog, len, &pos, 1, &b)) goto done;
            if (!enc_take(prog, len, &pos, b[0], &s)) goto done;
            err = cbor_encode_raw(cur, s, b[0]);
            break;
        }
        case 12:
        case 13: {
            size_t n;
            if (depth == ENC_MAX_DEPTH) { pos += 1; continue; }
            if (!enc_take(prog, len, &pos, 1, &b)) goto done;
            n = b[0] == 0xff ? CborIndefiniteLength : (size_t)b[0];
            err = op == 12 ? cbor_encoder_create_array(cur, &stack[depth + 1], n)
                           : cbor_encoder_create_map(cur, &stack[depth + 1], n);
            /* Upstream writes the head and then hands back the child either
             * way, so the child is live even when the head did not fit. */
            depth++;
            break;
        }
        case 14:
        case 15:
            if (depth == 0)
                continue;
            err = op == 14
                      ? cbor_encoder_close_container(&stack[depth - 1], cur)
                      : cbor_encoder_close_container_checked(&stack[depth - 1], cur);
            depth--;
            break;
        case 16:
            if (!enc_take(prog, len, &pos, 1, &b)) goto done;
            err = cbor_encode_boolean(cur, b[0] & 1);
            break;
        case 17:
            err = cbor_encode_null(cur);
            break;
        default:
            err = cbor_encode_undefined(cur);
            break;
        }
        enc_emit(transcript, &tn, err);
    }

done:
    if (use_writer) {
        /* What the callback saw, in order, with the append type it was told. */
        memcpy(transcript + tn, enc_log, enc_log_len);
        tn += enc_log_len;
        enc_emit(transcript, &tn, (CborError)enc_log_accepted);
    } else {
        /* Extra bytes needed is only meaningful on the root encoder, and only
         * after everything opened has been closed; an unclosed container has
         * not accounted for its own closing byte. Report it either way and let
         * the comparison decide, since both sides answer the same question. */
        needed = cbor_encoder_get_extra_bytes_needed(&stack[0]);
        memcpy(transcript + tn, outbuf, bufsize);
        tn += bufsize;
        enc_emit(transcript, &tn, (CborError)(needed > 0xffffffffu ? 0xffffffffu : needed));
    }

    fwrite(transcript, 1, tn, stdout);
    return err;
}

int main(int argc, char **argv)
{
    static unsigned char buf[MAX_INPUT];
    CborParser parser;
    CborValue value;
    CborError err;
    size_t len;
    const char *mode = argc > 1 ? argv[1] : "";
    unsigned long flags = argc > 2 ? strtoul(argv[2], NULL, 10) : 0;

    len = fread(buf, 1, sizeof(buf), stdin);

    if (strcmp(mode, "encode") == 0) {
        err = run_encoder_program(buf, len);
        fflush(stdout);
        fprintf(stderr, "%d\n", (int)err);
        return (int)err;
    }

    err = cbor_parser_init(buf, len, 0, &parser, &value);
    if (err == CborNoError) {
        if (strcmp(mode, "json") == 0)
            err = cbor_value_to_json_advance(stdout, &value, (int)flags);
        else if (strcmp(mode, "validate") == 0)
            err = cbor_value_validate(&value, (uint32_t)flags);
        else
            err = cbor_value_to_pretty_advance(stdout, &value);
    }

    fflush(stdout);
    fprintf(stderr, "%d\n", (int)err);
    return (int)err;
}
