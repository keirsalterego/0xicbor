#!/usr/bin/env python3
"""Generate the benchmark corpus in bench/corpus/.

No third-party dependencies: the ~70 lines of CBOR encoder below are all that is
needed, and they are easier to audit than a package pin.  Run from anywhere:

    python3 bench/gen_corpus.py

Output is deterministic -- a fixed PRNG seed, no timestamps -- so two people on
two machines get byte-identical files.  bench/corpus/manifest.json records the
shape of every file, and bench/corpus/README.md is the human-readable version.
"""

import json
import os
import random
import struct

SEED = 0x1CB0


# --- minimal CBOR encoder (RFC 8949 core types) -----------------------------


def _head(major, arg):
    """Encode the initial byte plus the argument for one CBOR head."""
    mt = major << 5
    if arg < 24:
        return bytes([mt | arg])
    if arg < 0x100:
        return bytes([mt | 24, arg])
    if arg < 0x10000:
        return bytes([mt | 25]) + struct.pack(">H", arg)
    if arg < 0x100000000:
        return bytes([mt | 26]) + struct.pack(">I", arg)
    return bytes([mt | 27]) + struct.pack(">Q", arg)


def uint(n):
    return _head(0, n)


def nint(n):
    """Negative integer; n must be < 0."""
    return _head(1, -1 - n)


def integer(n):
    return uint(n) if n >= 0 else nint(n)


def bstr(b):
    return _head(2, len(b)) + b


def tstr(s):
    b = s.encode("utf-8")
    return _head(3, len(b)) + b


def array(items):
    return _head(4, len(items)) + b"".join(items)


def cbormap(pairs):
    return _head(5, len(pairs)) + b"".join(k + v for k, v in pairs)


def tag(n, item):
    return _head(6, n) + item


def dbl(x):
    return b"\xfb" + struct.pack(">d", x)


TRUE, FALSE, NULL = b"\xf5", b"\xf4", b"\xf6"
BREAK = b"\xff"


def indef(major, items):
    """Indefinite-length array (4), map (5), byte string (2) or text (3)."""
    return bytes([(major << 5) | 31]) + b"".join(items) + BREAK


# --- corpus ------------------------------------------------------------------


def small_ints():
    """Every integer head width in one tiny document: 0..23 inline, then the
    1/2/4/8-byte forms, positive and negative, plus the simple values."""
    vals = [0, 1, 10, 23, 24, 255, 256, 65535, 65536, 4294967295, 4294967296]
    vals += [-1, -24, -25, -256, -257, -65536, -4294967297]
    return array([integer(v) for v in vals] + [TRUE, FALSE, NULL])


def flat_array(rng):
    """One flat array of 60k integers spread across every head width.  No
    nesting at all: this is the pure item-decode loop."""
    items = []
    for _ in range(60000):
        w = rng.randrange(4)
        n = rng.randrange([24, 256, 65536, 2**32][w])
        items.append(integer(n if rng.random() < 0.75 else -n - 1))
    return array(items)


def deep_nest(rng):
    """4000 independently nested chains, each 40 arrays deep with a small map at
    the bottom.  Depth stresses the parser's container enter/leave path; the
    repeat count is what pushes it to a realistic size."""
    outer = []
    for _ in range(4000):
        node = cbormap([(tstr("v"), integer(rng.randrange(1 << 20)))])
        for _ in range(40):
            node = array([node])
        outer.append(node)
    return array(outer)


def map_heavy(rng):
    """3000 records, each an 8-key map of short text keys, one of which nests a
    further 3-key map.  String-keyed map lookup shape, the common config /
    telemetry document."""
    keys = ["id", "name", "kind", "ts", "ok", "score", "tags", "meta"]
    recs = []
    for i in range(3000):
        meta = cbormap(
            [
                (tstr("host"), tstr("node-%03d" % rng.randrange(256))),
                (tstr("pid"), uint(rng.randrange(1, 32768))),
                (tstr("lvl"), uint(rng.randrange(8))),
            ]
        )
        recs.append(
            cbormap(
                [
                    (tstr(keys[0]), uint(i)),
                    (tstr(keys[1]), tstr("record-%06d" % i)),
                    (tstr(keys[2]), tstr(rng.choice(["alpha", "beta", "gamma"]))),
                    (tstr(keys[3]), uint(1700000000 + i * 7)),
                    (tstr(keys[4]), TRUE if i % 3 else FALSE),
                    (tstr(keys[5]), dbl(rng.random() * 100.0)),
                    (
                        tstr(keys[6]),
                        array([tstr("t%d" % rng.randrange(50)) for _ in range(3)]),
                    ),
                    (tstr(keys[7]), meta),
                ]
            )
        )
    return array(recs)


def text_utf8(rng):
    """400 text strings of mixed scripts: 1-byte ASCII, 2-byte Cyrillic/Greek,
    3-byte CJK and 4-byte astral (emoji, so surrogate-pair territory for any
    UTF-16 consumer).  This is the UTF-8 validation and escaping path."""
    alphabets = [
        "the quick brown fox jumps over the lazy dog 0123456789 ",
        "привет αβγδε ",
        "日本語のテキスト 中文文本 ",
        "\U0001f600\U0001f680\U0001f9ea\U0001f4be\U0001f30d ",
        'quote " backslash \\ newline \n tab \t del \x7f ',
    ]
    out = []
    for _ in range(400):
        parts = [rng.choice(alphabets) for _ in range(rng.randrange(20, 60))]
        out.append(tstr("".join(parts)))
    return array(out)


def bytes_heavy(rng):
    """1200 opaque byte strings, 16 B to 2 KiB, plus a few empties.  Pretty
    printing renders these as hex, so this is the slowest per-input-byte
    workload in the corpus by a wide margin."""
    out = [bstr(b"")]
    for _ in range(1200):
        n = rng.choice([16, 32, 64, 128, 256, 512, 1024, 2048])
        out.append(bstr(bytes(rng.randrange(256) for _ in range(n))))
    return array(out)


def indefinite(rng):
    """Indefinite-length everything: outer array, per-record maps, and chunked
    byte and text strings.  Exercises the break-handling and chunk-iteration
    code that the definite-length files never touch."""
    recs = []
    for i in range(2000):
        chunks_t = [tstr("chunk-%d/" % j) for j in range(rng.randrange(2, 6))]
        chunks_b = [bstr(bytes([rng.randrange(256)] * 8)) for _ in range(3)]
        recs.append(
            indef(
                5,
                [
                    tstr("i"),
                    uint(i),
                    tstr("t"),
                    indef(3, chunks_t),
                    tstr("b"),
                    indef(2, chunks_b),
                    tstr("a"),
                    indef(4, [integer(rng.randrange(-1000, 1000)) for _ in range(5)]),
                ],
            )
        )
    return indef(4, recs)


def tagged(rng):
    """Tagged values -- epoch times, bignums, decimal fractions, base64 hints --
    wrapped around the types above.  Tags are a separate decode path and none of
    the other files carry one."""
    out = []
    for i in range(1500):
        out.append(tag(1, uint(1700000000 + i)))
        out.append(tag(2, bstr(bytes(rng.randrange(256) for _ in range(12)))))
        out.append(tag(4, array([nint(-2), uint(rng.randrange(1 << 30))])))
        out.append(tag(23, bstr(b"\xde\xad\xbe\xef")))
        out.append(tag(32, tstr("https://example.invalid/%d" % i)))
    return array(out)


BUILDERS = [
    ("small_ints.cbor", lambda rng: small_ints(), small_ints.__doc__),
    ("flat_array.cbor", flat_array, flat_array.__doc__),
    ("deep_nest.cbor", deep_nest, deep_nest.__doc__),
    ("map_heavy.cbor", map_heavy, map_heavy.__doc__),
    ("text_utf8.cbor", text_utf8, text_utf8.__doc__),
    ("bytes_heavy.cbor", bytes_heavy, bytes_heavy.__doc__),
    ("indefinite.cbor", indefinite, indefinite.__doc__),
    ("tagged.cbor", tagged, tagged.__doc__),
]


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    out = os.path.join(here, "corpus")
    os.makedirs(out, exist_ok=True)

    manifest = []
    for name, build, doc in BUILDERS:
        rng = random.Random(SEED)  # per-file seed: adding a file cannot move another
        data = build(rng)
        with open(os.path.join(out, name), "wb") as f:
            f.write(data)
        shape = " ".join(doc.split())
        manifest.append({"file": name, "bytes": len(data), "shape": shape})
        print("%-18s %9d B" % (name, len(data)))

    with open(os.path.join(out, "manifest.json"), "w") as f:
        json.dump({"seed": SEED, "files": manifest}, f, indent=2)
        f.write("\n")

    with open(os.path.join(out, "README.md"), "w") as f:
        f.write("# Benchmark corpus\n\n")
        f.write(
            "Generated by `bench/gen_corpus.py` (seed `0x%X`). Deterministic:\n"
            "regenerating gives byte-identical files. Do not hand-edit.\n\n" % SEED
        )
        for e in manifest:
            f.write("## `%s` -- %s bytes\n\n%s\n\n" % (e["file"], f"{e['bytes']:,}", e["shape"]))


if __name__ == "__main__":
    main()
