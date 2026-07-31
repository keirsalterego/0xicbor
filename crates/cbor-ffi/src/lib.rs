//! The C ABI shim: exports the 44 symbols that make up `libtinycbor.a`.
//!
//! The encoder lives in `encoder`, the parser in `parser`. What is left here is
//! the handful of entry points that are still scaffolding, each returning
//! `CborErrorInternalError` so the Qt suite runs to completion and reports a
//! real per-row failure count instead of dying on the first call.
//!
//! Deliberately not `no_std`, unlike cbor-core. This crate only ever links into
//! a hosted C program, and leaning on std here supplies the allocator and the
//! panic handler without hand-rolling either. The portability claim belongs to
//! cbor-core, which is where it actually buys something.

mod encoder;
mod parser;
mod pretty;
mod types;

pub use types::{CborEncoder, CborParser, CborValue};

use core::ffi::{c_char, c_int, c_void};

/// The C `CborError` value returned while a function is still a stub.
///
/// `CborErrorInternalError` is `INT_MAX` upstream. Nothing in the suite expects
/// it, so every row that touches an unfinished path fails loudly rather than
/// coincidentally passing.
const STUB: c_int = c_int::MAX;

/// Declares an `extern "C"` stub that returns [`STUB`].
///
/// The argument lists are spelled out even though the bodies ignore them: the
/// symbol name is what the linker matches, but the parameter types are what the
/// signature keeps once the body is real, so each function gets filled in rather
/// than rewritten.
macro_rules! stub {
    ($( $name:ident ( $($arg:ident : $ty:ty),* $(,)? ); )*) => {$(
        #[no_mangle]
        pub extern "C" fn $name($($arg: $ty),*) -> c_int {
            $( let _ = $arg; )*
            STUB
        }
    )*};
}

// -- not yet ported --------------------------------------------------------

stub! {
    // Allocates and hands ownership to the caller, who frees it with free().
    // That cross-language allocation contract is the one place the unsafe
    // budget is expected to grow for a reason other than pointer validation.
    _cbor_value_dup_string(value: *const CborValue, buffer: *mut *mut c_void, buflen: *mut usize, next: *mut CborValue);

    // Strict and canonical-mode validation, beyond the well-formedness check
    // that parser::cbor_value_validate_basic already performs.
    cbor_value_validate(it: *const CborValue, flags: u32);

    cbor_value_to_json_advance(out: *mut c_void, value: *mut CborValue, flags: c_int);
}

/// Returns null while stubbed. The real version hands back a `'static` C string.
#[no_mangle]
pub extern "C" fn cbor_error_string(_error: c_int) -> *const c_char {
    core::ptr::null()
}
