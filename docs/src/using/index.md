# Using the library

The build produces `target/release/libtinycbor.a`. It exports the same 44 symbols as
upstream's archive of the same name, and the headers in `crates/cbor-ffi/include/` are
upstream's, byte for byte. So if you already link tinycbor, you link this the same way and
change nothing else.

```console
$ make
$ cc -Icrates/cbor-ffi/include yourprogram.c -o yourprogram \
     target/release/libtinycbor.a -lm -lpthread -ldl
```

Those last three flags are the Rust standard library's own link requirements. They are the
only difference from linking the C, and they are not optional.

## The smallest thing that works

Encode a two-key map into a stack buffer:

```c
#include <stdio.h>
#include <cbor.h>

int main(void)
{
    uint8_t buf[64];
    CborEncoder root, map;

    cbor_encoder_init(&root, buf, sizeof buf, 0);
    cbor_encoder_create_map(&root, &map, 2);
    cbor_encode_text_stringz(&map, "id");
    cbor_encode_uint(&map, 7);
    cbor_encode_text_stringz(&map, "ok");
    cbor_encode_boolean(&map, true);
    cbor_encoder_close_container(&root, &map);

    size_t len = cbor_encoder_get_buffer_size(&root, buf);
    for (size_t i = 0; i < len; ++i)
        printf("%02x", buf[i]);
    printf("\n%zu bytes\n", len);
    return 0;
}
```

```console
$ cc -std=c99 -Icrates/cbor-ffi/include encode.c -o encode \
     target/release/libtinycbor.a -lm -lpthread -ldl
$ ./encode
a262696407626f6bf5
9 bytes
```

Nine bytes for `{"id": 7, "ok": true}`. That is the entire pitch for CBOR, and the reason
tinycbor exists.

## What you get, and what you do not

**No allocation on the parse path.** Not "few allocations", none. You give the encoder a
buffer and you give the parser a buffer, and the library never asks the heap for anything.
The one exception is `cbor_value_dup_text_string`, which is documented to `malloc` and hand
you the result. That is [decision 20](../reference/decisions.md).

**No `errno`, no global state.** Every call returns a `CborError`. Two calls on two threads
touching two different encoders cannot interfere, because there is nothing shared to
interfere with.

**No dynamic dispatch you did not ask for.** The parser reads a flat buffer by default. If
your bytes arrive some other way, `cbor_parser_init_reader` takes a table of callbacks
instead. Both go through the same code.

**Not a Rust API.** This is a C library that happens to be written in Rust. There is no
`serde` integration, no `#[derive]`, no `Value` enum. If you want an idiomatic Rust CBOR
crate, several good ones exist and this is not one of them. What this is for is replacing
the C in a codebase that already speaks C.

## Reading on

- [Encoding](encoding.md): buffers that are too small, indefinite lengths, and why the
  encoder keeps going after it runs out of room.
- [Parsing](parsing.md): containers, strings that arrive in chunks, and the cursor rules.
- [The command-line tools](tools.md): `cbordump` and `json2cbor`.

If what you actually want is to know whether to trust any of this, start at the
[scoreboard](../verification/scoreboard.md) instead.
