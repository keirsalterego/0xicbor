# Symbol parity

If the exported symbol list matches upstream's exactly, any program that links against
tinycbor links against this instead. That is the strongest statement a drop-in replacement
can make, and it is one command:

```console
$ make symbols
symbols: 44/44, zero diff
```

Which runs:

```console
$ nm -g --defined-only target/release/libtinycbor.a \
    | awk '$2 ~ /^[A-TV-Z]$/ {print $3}' | grep -E '^_?cbor_' | sort -u \
    | diff bench/reference/symbols-upstream.txt -
```

`bench/reference/symbols-upstream.txt` was generated the same way from upstream's own
`libtinycbor.a`, built at kickoff and committed as the comparison target. It is never
linked into anything.

## The 44

Thirty-seven are documented API. Seven are the private-but-exported functions that
`cbor.h`'s inline accessors call, and they are part of the ABI whether or not they are part
of the API:

```
_cbor_value_begin_string_iteration
_cbor_value_copy_string
_cbor_value_decode_int64_internal
_cbor_value_dup_string
_cbor_value_finish_string_iteration
_cbor_value_get_string_chunk
_cbor_value_get_string_chunk_size
```

Missing any one of those produces a link error in the test binary, not a runtime failure,
which is a pleasant category of bug to have.

## Getting it right the first time

The scaffold exported all 44 as stubs before a single line of CBOR logic existed, and the
diff was empty on the first build. That ordering is deliberate: symbol parity is a property
of the *interface*, so it costs almost nothing to establish up front and becomes a
regression test for every commit afterwards. Discovering a missing symbol at hour 60 is a
much worse afternoon.
