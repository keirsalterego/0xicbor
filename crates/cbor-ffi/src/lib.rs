//! The C ABI shim: exports the 44 symbols that make up `libtinycbor.a`.
//!
//! The encoder lives in `encoder`, the parser in `parser`, diagnostic notation
//! in `pretty`, JSON in `tojson` and the strictness checks in `validation`.
//! What is left here is the one entry point that belongs to none of them.
//!
//! Deliberately not `no_std`, unlike cbor-core. This crate only ever links into
//! a hosted C program, and leaning on std here supplies the allocator and the
//! panic handler without hand-rolling either. The portability claim belongs to
//! cbor-core, which is where it actually buys something.

mod encoder;
mod parser;
mod pretty;
mod tojson;
mod types;
mod validation;

pub use types::{CborEncoder, CborParser, CborValue};

use core::ffi::{c_char, c_int};

/// Static storage, so the caller can hold the pointer indefinitely — which is
/// what upstream promises and what callers assume.
#[no_mangle]
pub extern "C" fn cbor_error_string(error: c_int) -> *const c_char {
    cbor_core::errstr::error_string(error).as_ptr()
}
