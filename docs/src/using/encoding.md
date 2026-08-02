# Encoding

An encoder writes into memory you own. `cbor_encoder_init` takes the buffer and its size,
and from then on the encoder never asks anyone for more room.

```c
uint8_t buf[64];
CborEncoder root;
cbor_encoder_init(&root, buf, sizeof buf, 0);
```

Containers get their own encoder, which is how the library tracks how many items you still
owe without allocating a stack:

```c
CborEncoder array;
cbor_encoder_create_array(&root, &array, 3);
cbor_encode_uint(&array, 1);
cbor_encode_uint(&array, 2);
cbor_encode_uint(&array, 3);
cbor_encoder_close_container(&root, &array);
```

Write to the child while it is open, write to the parent after you close it. Mixing those
up is the most common way to produce a document that decodes into something you did not
expect, and nothing will stop you.

## Running out of room is not an error you have to handle immediately

This is the part of the API most people miss, and it is genuinely good design.

When the buffer fills, the encoder does not stop. It quietly switches from writing bytes to
counting them, returns `CborErrorOutOfMemory` from that call and every call after it, and
keeps tracking exactly how much space the whole document would have taken.

```c
uint8_t small[4];
CborEncoder e;
cbor_encoder_init(&e, small, sizeof small, 0);

CborError err = cbor_encode_text_stringz(&e, "far too long for four bytes");
printf("err = %d (%s)\n", err, cbor_error_string(err));
printf("extra bytes needed = %zu\n", cbor_encoder_get_extra_bytes_needed(&e));
```

```console
err = -2147483648 (out of memory/need more memory)
extra bytes needed = 25
```

So you can encode into a deliberately tiny buffer, ignore every return value, ask at the
end how much you needed, allocate exactly that, and encode again. Two passes, no guessing,
no `realloc` loop. On a microcontroller that is often the difference between fitting and
not.

The bytes already written stay valid. The encoder truncates at the boundary rather than
half-writing an item.

## Indefinite lengths

Pass `CborIndefiniteLength` when you do not know the count up front. The encoder writes the
`_` form and the break byte on close.

```c
CborEncoder array;
cbor_encoder_create_array(&root, &array, CborIndefiniteLength);
cbor_encode_uint(&array, 1);
cbor_encode_uint(&array, 2);
cbor_encoder_close_container(&root, &array);
```

That produces `9f 01 02 ff`, four bytes, against `83 01 02 03`-style three for the definite
form of the same length. You pay one byte for not knowing.

## Closing containers, checked and unchecked

`cbor_encoder_close_container` trusts you. `cbor_encoder_close_container_checked` verifies
you wrote exactly as many items as you promised and returns `CborErrorTooFewItems` if you
did not.

Use the checked one while you are developing. The unchecked one exists because on a device
where you have already proven the shape, the comparison is dead weight.

## Not writing to a buffer at all

`cbor_encoder_init_writer` takes a callback instead of a buffer. Every fragment the encoder
produces is handed to your function, tagged with a `CborEncoderAppendType` saying whether
it is structural CBOR, string payload, or raw passthrough. Return `CborNoError` to accept
it, anything else to stop the encode.

That is how you stream straight to a socket without buffering the document first. It is
also the path that had no test coverage anywhere until the
[encoder fuzz target](../verification/differential-fuzzing.md) started driving it.

## Floats, and the one that bites

`cbor_encode_float` and `cbor_encode_double` write what you give them. `cbor_encode_half_float`
takes a pointer to a raw 16-bit pattern you already have.

`cbor_encode_float_as_half_float` is the interesting one: it converts, and conversion means
rounding. It rounds to nearest, ties to even, matching what the C gets from a hardware
`_Float16` cast. If you are round-tripping floats through half precision and comparing for
equality, that rounding is where your surprise is going to come from, and it is worth
reading [decision 19](../reference/decisions.md) before you file a bug.

## Next steps

- [Parsing](parsing.md), which is the same ideas pointed the other way.
- [The scoreboard](../verification/scoreboard.md) if you want to know how much of the above
  is checked against upstream rather than asserted.
