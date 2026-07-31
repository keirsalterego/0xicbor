//! The C ABI shim: exports the 44 symbols that make up `libtinycbor.a`.
//!
//! Everything here is scaffolding for now. Each entry point returns
//! `CborErrorInternalError` (or does nothing, for the two `void` functions) so
//! the Qt suite links, runs to completion, and reports a real per-row failure
//! count. Returning an error rather than `unimplemented!()` is deliberate: a
//! panic in a `panic = "abort"` staticlib kills the test process on the first
//! call and the baseline number would be "it crashed" instead of 4929.
//!
//! As each module of cbor-core lands, the matching stubs here get replaced and
//! the failure count drops. That number is the progress bar for the port.

// Deliberately not `no_std`, unlike cbor-core. This crate only ever links into
// a hosted C program, and leaning on std here supplies the allocator and the
// panic handler without hand-rolling either. The portability claim belongs to
// cbor-core, which is where it actually buys something.

mod encoder;
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
/// The argument lists still have to be spelled out: the symbol name is what the
/// linker matches, but the parameter types are what the signature will keep
/// once the body is real, and writing them now means each function gets filled
/// in rather than rewritten.
macro_rules! stub {
    ($( $name:ident ( $($arg:ident : $ty:ty),* $(,)? ); )*) => {$(
        #[no_mangle]
        pub extern "C" fn $name($($arg: $ty),*) -> c_int {
            $( let _ = $arg; )*
            STUB
        }
    )*};
}

// -- parser ----------------------------------------------------------------

stub! {
    cbor_parser_init(buffer: *const u8, size: usize, flags: u32, parser: *mut CborParser, it: *mut CborValue);
    cbor_parser_init_reader(ops: *const c_void, parser: *mut CborParser, it: *mut CborValue, token: *mut c_void);
    cbor_value_advance(it: *mut CborValue);
    cbor_value_advance_fixed(it: *mut CborValue);
    cbor_value_enter_container(it: *const CborValue, recursed: *mut CborValue);
    cbor_value_leave_container(it: *mut CborValue, recursed: *const CborValue);
    cbor_value_reparse(it: *mut CborValue);
    cbor_value_skip_tag(it: *mut CborValue);
    cbor_value_get_int_checked(value: *const CborValue, result: *mut c_int);
    cbor_value_get_int64_checked(value: *const CborValue, result: *mut i64);
    cbor_value_get_half_float_as_float(value: *const CborValue, result: *mut f32);
    cbor_value_calculate_string_length(value: *const CborValue, length: *mut usize);
    cbor_value_text_string_equals(value: *const CborValue, string: *const c_char, result: *mut bool);
    cbor_value_map_find_value(map: *const CborValue, string: *const c_char, element: *mut CborValue);
}

// Private but exported: cbor.h calls these from its inline accessors, so they
// are part of the ABI whether or not they are part of the documented API.
stub! {
    _cbor_value_decode_int64_internal(value: *const CborValue);
    _cbor_value_copy_string(value: *const CborValue, buffer: *mut c_void, buflen: *mut usize, next: *mut CborValue);
    _cbor_value_dup_string(value: *const CborValue, buffer: *mut *mut c_void, buflen: *mut usize, next: *mut CborValue);
    _cbor_value_get_string_chunk(value: *const CborValue, bufferptr: *mut *const c_void, len: *mut usize, next: *mut CborValue);
    _cbor_value_get_string_chunk_size(value: *const CborValue, len: *mut usize);
    _cbor_value_begin_string_iteration(value: *mut CborValue);
    _cbor_value_finish_string_iteration(value: *mut CborValue);
}

// -- validation ------------------------------------------------------------

stub! {
    cbor_value_validate(it: *const CborValue, flags: u32);
    cbor_value_validate_basic(it: *const CborValue);
}

// -- pretty printing and JSON ----------------------------------------------

stub! {
    cbor_value_to_pretty_advance(out: *mut c_void, value: *mut CborValue);
    cbor_value_to_pretty_advance_flags(out: *mut c_void, value: *mut CborValue, flags: c_int);
    cbor_value_to_pretty_stream(stream: *mut c_void, token: *mut c_void, value: *mut CborValue, flags: c_int);
    cbor_value_to_json_advance(out: *mut c_void, value: *mut CborValue, flags: c_int);
}

// -- error strings ---------------------------------------------------------

/// Returns null while stubbed. The real version hands back a `'static` C string.
#[no_mangle]
pub extern "C" fn cbor_error_string(_error: c_int) -> *const c_char {
    core::ptr::null()
}
