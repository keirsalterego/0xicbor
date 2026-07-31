//! A CBOR (RFC 8949) encoder and parser, ported from intel/tinycbor.
//!
//! This crate is the whole port. It has no dependencies, not even for
//! half-floats, and never touches C. The C-compatible surface lives in
//! `cbor-ffi`, which is the only crate allowed to write `unsafe`.
//!
//! `no_std` because upstream targets microcontrollers and dropping that would
//! be a regression in capability, not just in spirit. `alloc` is pulled in
//! separately: only the `dup_string` family and indefinite-length string
//! reassembly need to own memory, and both are optional upstream too.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod error;

pub use error::{CborError, CborResult};
