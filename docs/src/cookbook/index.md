# Cookbook: porting a C library to Rust

This section is the part you can steal. It walks through how 0xicbor was actually
built, in order, with the reasoning left in, including the bits that went wrong.

You do not need to know CBOR. You need to be able to read C well enough to follow a
`switch`, and to write Rust well enough to be annoyed by the borrow checker.

## The one idea

Most "rewrite it in Rust" projects fail the same way: you rewrite the library, you
rewrite the tests to match, and now you have no idea whether it behaves like the
original. Every disagreement becomes a test edit, and a test suite you edited proves
nothing.

The whole method here is one move that makes that impossible:

> Keep the original test suite. Byte for byte. Make your Rust satisfy it.

Everything else (the ABI shim, the layout assertions, the symbol diff) exists to
make that one move physically possible.

## The order things happened

1. **[Get to a red test loop](red-loop.md)**: before writing a single line of real
   logic, make the original tests compile, link, run, and fail against an empty
   library. That failure count is your progress bar for the rest of the project.
2. **[Match the ABI](matching-the-abi.md)**: the tests read struct fields directly,
   so your structs have to be laid out exactly like C's. This is measurable, not a
   matter of care.
3. **[Write the first module](first-module.md)**: half-precision floats, by hand, in
   about sixty lines. Small enough to get right, and it has a test that proves it.
4. **[Match printf exactly](matching-printf.md)**: where most of the remaining bugs
   live, and where I was wrong three times in a row.
5. **[When the C is faster](when-c-is-faster.md)**: green tests and 1.49x slower.
   Finding out which GCC optimisation pass you are competing with, and what the Rust
   equivalent of it is. Two wrong hypotheses before the right one.

## What this costs you

Being honest about the trade: this method makes your Rust look less like Rust at the
edges. You will write out-parameters and integer error codes because callers expect
them. The trick is confining that to one thin layer and keeping the real
implementation idiomatic underneath, which is what [Why an ABI
shim](../architecture/abi-shim.md) is about.

In exchange, every claim you make is a command someone else can run.
