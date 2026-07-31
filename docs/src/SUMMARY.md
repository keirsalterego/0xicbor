# Summary

[Introduction](index.md)

# Cookbook

- [Porting a C library to Rust](cookbook/index.md)
  - [1. Get to a red test loop](cookbook/red-loop.md)
  - [2. Match the ABI](cookbook/matching-the-abi.md)
  - [3. Write the first module](cookbook/first-module.md)
  - [4. Match printf exactly](cookbook/matching-printf.md)

# The port

- [Why an ABI shim](architecture/abi-shim.md)
  - [Layout parity](architecture/layout-parity.md)
  - [Symbol parity](architecture/symbol-parity.md)
- [Where the C ends](architecture/the-c-question.md)
- [The unsafe budget](architecture/unsafe-budget.md)

# Verification

- [Running the original suite](verification/original-suite.md)
- [Scoreboard](verification/scoreboard.md)

# Reference

- [Building](reference/building.md)
- [Decision log](reference/decisions.md)
