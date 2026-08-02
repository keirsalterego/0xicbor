# The command-line tools

Two binaries, both rewritten in safe Rust rather than being C over the library. Same flags,
same output, same exit codes as upstream's.

```console
$ cargo build --release -p cbordump -p json2cbor
```

## cbordump

Reads CBOR, prints it. Diagnostic notation by default, JSON with `-j`.

```console
$ cbordump doc.cbor
{"id": 7, "ok": true}

$ cbordump -j doc.cbor
{"id":7,"ok":true}
```

Diagnostic notation ([RFC 8949 §8](https://www.rfc-editor.org/rfc/rfc8949#section-8)) is
the one to reach for when you are debugging, because it shows you things JSON cannot say:
tags, byte strings, indefinite lengths, `undefined`, the difference between a float and an
integer that happens to be whole.

| flag | effect |
|---|---|
| `-c` | diagnostic notation (default) |
| `-j` | JSON |
| `-f` | show text and byte string fragments separately |
| `-n` | show overlong encodings of numbers and lengths |
| `-M` | with `-j`, add metadata so the conversion back is exact |
| `-O` | with `-j`, turn CBOR tags into JSON objects |
| `-S` | with `-j`, stringify map keys that are not text |
| `-U` | with `-j`, base64url every byte string regardless of tags |

Malformed input goes to stderr and the exit code is the `CborError`. Whatever was decoded
before the failure has already been printed, which is deliberate and matches upstream:

```console
$ cbordump truncated.cbor
{"id
truncated.cbor: unexpected end of data
$ echo $?
1
```

## json2cbor

The other direction.

```console
$ cbordump -j doc.cbor | json2cbor - > out.cbor
```

`-` reads stdin. `-M` interprets the metadata `cbordump -jM` adds, which is what makes the
round trip lossless. Without it, JSON has no way to express a tag or a byte string, so the
trip through JSON quietly flattens them.

## They are a second implementation, and that mattered

These tools do not call `libtinycbor.a`. They cannot: the FFI speaks in raw pointers, and a
binary using it would need `unsafe` outside the one crate allowed to have any. So they are a
separate parser and a separate printer built on `cbor-core`.

Which means nothing was checking them. Upstream's Qt suite tests the library. The fuzzers
call the C ABI directly. A tool could have disagreed with upstream on every input and the
board would still have read green.

Running them against upstream's own binaries over 4,509 documents and every flag
combination found four bugs, one of which accepted a document upstream rejects. The harness
is `tests/port/tools_diff.sh` and it runs in `make test`. The write-up is
[decision 18](../reference/decisions.md).

## Next steps

- [Encoding](encoding.md) and [Parsing](parsing.md) for the library behind the tools.
- [Troubleshooting](../reference/troubleshooting.md) if `make test-tools` is skipping.
