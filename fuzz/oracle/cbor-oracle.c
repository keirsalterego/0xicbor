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
