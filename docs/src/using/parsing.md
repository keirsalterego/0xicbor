# Parsing

A parser is a cursor over bytes you own. Nothing is copied, nothing is allocated, and the
`CborValue` you get back points into your buffer. Keep the buffer alive for as long as you
keep the cursor.

```c
CborParser parser;
CborValue it;
CborError err = cbor_parser_init(doc, sizeof doc, 0, &parser, &it);
if (err) {
    fprintf(stderr, "%s\n", cbor_error_string(err));
    return 1;
}
```

Keep the `CborParser` alive too. `CborValue` holds a pointer back to it, and neither the
compiler nor this library will notice if you let it go out of scope first. That is a real
sharp edge inherited from the C API, and [decision 21](../reference/decisions.md) explains
why it could not be fixed without breaking the ABI.

## Walking a map

```c
CborValue map;
cbor_value_enter_container(&it, &map);

while (!cbor_value_at_end(&map)) {
    char key[16];
    size_t n = sizeof key;
    cbor_value_copy_text_string(&map, key, &n, &map);

    if (cbor_value_is_unsigned_integer(&map)) {
        uint64_t v;
        cbor_value_get_uint64(&map, &v);
        printf("%s = %llu\n", key, (unsigned long long)v);
    } else if (cbor_value_is_boolean(&map)) {
        bool v;
        cbor_value_get_boolean(&map, &v);
        printf("%s = %s\n", key, v ? "true" : "false");
    }
    cbor_value_advance(&map);
}

cbor_value_leave_container(&it, &map);
```

On `{"id": 7, "ok": true}` that prints:

```console
id = 7
ok = true
```

Three rules hold this together.

**A map is a flat sequence.** Key, value, key, value. There is no pair abstraction. If you
advance an odd number of times you are now reading values as keys and the library will not
warn you.

**`enter` and `leave` come in pairs.** `cbor_value_leave_container` is what moves the outer
cursor past the whole container. Skip it and the outer cursor is still sitting on the
opening byte.

**Every call can fail.** The examples here drop the return values to stay short. Do not do
that in code that runs.

## Strings, and the length parameter that goes both ways

`cbor_value_copy_text_string` takes a buffer and a pointer to its size. On success the size
becomes the length written. If the buffer is too small you get `CborErrorOutOfMemory` and
the size becomes the length you needed, so the two-pass pattern works here as well:

```c
size_t len = 0;
cbor_value_calculate_string_length(&value, &len);
char *s = malloc(len + 1);
cbor_value_copy_text_string(&value, s, &len, NULL);
s[len] = '\0';
```

`cbor_value_dup_text_string` does that for you and hands back a `malloc`ed pointer. It is
the only function in the library that allocates, and the memory is yours to `free`.

**Strings can arrive in pieces.** An indefinite-length string is a series of chunks with no
single contiguous form on the wire. The copy functions hide that: they walk the chunks and
join them into your buffer. `cbor_value_get_text_string_chunk` exposes the chunks directly
if you would rather not have a buffer big enough for the whole thing.

## Skipping things you do not care about

`cbor_value_advance` moves past the current item, whatever it is. On a container that means
the entire subtree, however deep.

This turns out to be the hottest function in the library, and how it works is the reason
this port parses faster than the C. Upstream descends recursively and fully decodes every
item on the way past, then throws all of it away. This one scans for the end instead,
because the only thing an advance produces is a position. It hands back to the recursive
path on anything unusual, so the errors you get are still the original errors. That is
[decision 16](../reference/decisions.md), and the measurements are in the
[benchmark methodology](https://github.com/keirsalterego/0xicbor/blob/main/bench/methodology.md).

## Bytes from somewhere other than a buffer

`cbor_parser_init_reader` takes four callbacks instead of a pointer: can you read this many
bytes, read them, advance, and hand me a string. Use it when the document is arriving over a
transport and you do not want to assemble it first.

The rest of the API is identical afterwards. Internally the two are separate specialised
copies of the parser rather than one copy testing a flag, which is
[decision 13](../reference/decisions.md).

One honest caveat: the subtree scan above is buffer-only by construction, so a reader source
takes the recursive path and is slower. It is correct, and it is checked against upstream by
`tests/port/tst_reader_diff.c` on every `make test`, but it is not the fast path.

## Validation is separate

Parsing accepts anything structurally well formed. If you need more, `cbor_value_validate`
takes a flag matrix: shortest-form integers, sorted map keys, unique keys, valid UTF-8,
no undefined, no tags, finite floats, and a dozen more. `CborValidateStrictest` turns on
everything.

Run it before you trust a document from outside. It is a separate call because most callers
do not need it and it is not free.

## Next steps

- [The command-line tools](tools.md), which are the fastest way to see what a document
  actually contains.
- [Differential fuzzing](../verification/differential-fuzzing.md) for how the above is
  checked against the C rather than merely believed.
