# Summary

[Introduction](index.md)

# Using it

- [Using the library](using/index.md)
  - [Encoding](using/encoding.md)
  - [Parsing](using/parsing.md)
  - [The command-line tools](using/tools.md)

# Verification

- [Scoreboard](verification/scoreboard.md)
- [Running the original suite](verification/original-suite.md)
- [Differential fuzzing](verification/differential-fuzzing.md)

# The port

- [Why an ABI shim](architecture/abi-shim.md)
  - [Layout parity](architecture/layout-parity.md)
  - [Symbol parity](architecture/symbol-parity.md)
- [Where the C ends](architecture/the-c-question.md)
- [The unsafe budget](architecture/unsafe-budget.md)

# Cookbook

- [Porting a C library to Rust](cookbook/index.md)
  - [1. Get to a red test loop](cookbook/red-loop.md)
  - [2. Match the ABI](cookbook/matching-the-abi.md)
  - [3. Write the first module](cookbook/first-module.md)
  - [4. Match printf exactly](cookbook/matching-printf.md)
  - [5. When the C is faster](cookbook/when-c-is-faster.md)

# Reference

- [Building](reference/building.md)
- [Troubleshooting](reference/troubleshooting.md)
- [Contributing](reference/contributing.md)
- [Decision log](reference/decisions.md)
