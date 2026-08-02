//! The encoder half of the C ABI.
//!
//! All the CBOR knowledge is in `cbor_core::encoder`; what lives here is buffer
//! management, which is inherently pointer work and therefore inherently
//! `unsafe`. Every block below is dereferencing something the C caller handed
//! us, and the invariant is always the same one, so it is stated once here:
//!
//! **SAFETY (whole module):** every `*mut CborEncoder` parameter must point to
//! an initialised `CborEncoder` that the caller keeps alive and does not alias
//! for the duration of the call. This is the same contract upstream's C has,
//! and it cannot be checked from this side.

use crate::types::CborEncoder;
use cbor_core::encoder::{head, indefinite_head, major, Head, BREAK};
use core::ffi::{c_char, c_int, c_void};

/// Encoder flag bits, from `enum CborEncoderFlags` and `enum CborIteratorFlags`.
const WRITER_FUNCTION: i32 = 0x01;
const UNKNOWN_LENGTH: i32 = 0x10;
const CONTAINER_IS_MAP: i32 = 0x20;

/// `CborIndefiniteLength` is `SIZE_MAX`.
const INDEFINITE_LENGTH: usize = usize::MAX;

const NO_ERROR: c_int = 0;
const ERR_OUT_OF_MEMORY: c_int = c_int::MIN;
const ERR_TOO_MANY_ITEMS: c_int = 768;
const ERR_TOO_FEW_ITEMS: c_int = 769;
const ERR_ILLEGAL_SIMPLE_TYPE: c_int = 262;

/// `CborEncoderAppendType`, which the writer callback receives so it can tell
/// structural bytes from payload.
const APPEND_CBOR_DATA: u32 = 0;
const APPEND_STRING_DATA: u32 = 1;
const APPEND_RAW_DATA: u32 = 2;

/// The writer callback signature from cbor.h.
type WriteFunction = extern "C" fn(*mut c_void, *const c_void, usize, u32) -> c_int;

/// True once the buffer has overrun and the encoder is only counting.
fn counting(e: &CborEncoder) -> bool {
    e.end.is_null()
}

/// Upstream's `would_overflow`, which is deliberately written to also work once
/// `end` is null and `data` has become a byte count rather than a pointer.
fn would_overflow(e: &CborEncoder, len: usize) -> bool {
    // C picks `ptr` or `bytes_needed` depending on whether `end` is set, but
    // both are the same union word, so one read covers it.
    (e.end as isize - e.data.0 as isize - len as isize) < 0
}

/// Upstream's `advance_ptr`: moves the write cursor, or grows the shortfall
/// counter once the buffer is gone.
fn advance(e: &mut CborEncoder, n: usize) {
    e.data.0 = (e.data.0 as usize).wrapping_add(n) as *mut c_void;
}

/// The one place bytes leave this module.
///
/// On overflow upstream discards the partial write, drops `end` to null, and
/// from then on only accumulates how many bytes the caller would have needed.
/// The already-written prefix stays valid, which is why `len` is reduced by the
/// space that did fit before switching modes.
fn append(e: &mut CborEncoder, data: &[u8], append_type: u32) -> c_int {
    if e.flags & WRITER_FUNCTION != 0 {
        // SAFETY: the caller set WRITER_FUNCTION via cbor_encoder_init_writer,
        // which is the only way this bit gets set, and stored a valid function
        // pointer in `data` and its token in `end`.
        let writer: WriteFunction = unsafe { core::mem::transmute(e.data.0) };
        return writer(
            e.end as *mut c_void,
            data.as_ptr() as *const c_void,
            data.len(),
            append_type,
        );
    }

    let mut len = data.len();
    if would_overflow(e, len) {
        if !counting(e) {
            let fitted = e.end as usize - e.data.0 as usize;
            len -= fitted;
            e.end = core::ptr::null_mut();
            e.data.0 = core::ptr::null_mut();
        }
        advance(e, len);
        return ERR_OUT_OF_MEMORY;
    }

    // SAFETY: would_overflow just proved data.ptr..data.ptr+len is inside the
    // caller's buffer, and the source is a Rust slice we own.
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), e.data.0 as *mut u8, len);
    }
    advance(e, len);
    NO_ERROR
}

fn append_head(e: &mut CborEncoder, (buf, n): Head) -> c_int {
    append(e, &buf[..n], APPEND_CBOR_DATA)
}

/// Upstream's `saturated_decrement`: the item counter stops at zero rather than
/// wrapping, so an over-full container reports TooManyItems instead of looping.
fn saturated_decrement(e: &mut CborEncoder) {
    if e.remaining != 0 {
        e.remaining -= 1;
    }
}

fn encode_number(e: &mut CborEncoder, value: u64, major: u8) -> c_int {
    saturated_decrement(e);
    append_head(e, head(major, value))
}

fn is_oom(err: c_int) -> bool {
    err == ERR_OUT_OF_MEMORY
}

/// SAFETY: see the module-level contract.
unsafe fn as_mut<'a>(p: *mut CborEncoder) -> &'a mut CborEncoder {
    &mut *p
}

// -- initialisation --------------------------------------------------------

#[no_mangle]
pub extern "C" fn cbor_encoder_init(
    encoder: *mut CborEncoder,
    buffer: *mut u8,
    size: usize,
    flags: c_int,
) {
    // SAFETY: module contract.
    let e = unsafe { as_mut(encoder) };
    e.data.0 = buffer as *mut c_void;
    e.end = buffer.wrapping_add(size);
    e.remaining = 2;
    e.flags = flags;
}

#[no_mangle]
pub extern "C" fn cbor_encoder_init_writer(
    encoder: *mut CborEncoder,
    writer: *mut c_void,
    token: *mut c_void,
) {
    // SAFETY: module contract.
    let e = unsafe { as_mut(encoder) };
    e.data.0 = writer;
    e.end = token as *mut u8;
    e.remaining = 2;
    e.flags = WRITER_FUNCTION;
}

// -- scalars ---------------------------------------------------------------

#[no_mangle]
pub extern "C" fn cbor_encode_uint(encoder: *mut CborEncoder, value: u64) -> c_int {
    // SAFETY: module contract.
    encode_number(unsafe { as_mut(encoder) }, value, major::UNSIGNED)
}

/// Takes the absolute value, so -1 arrives here as 1. Major type 1 carries
/// `-1 - n` (RFC 8949 §3.1), hence the decrement. Upstream wraps on 0 and so
/// does this: encoding "negative zero" is the caller's mistake, not ours to
/// diagnose.
#[no_mangle]
pub extern "C" fn cbor_encode_negative_int(
    encoder: *mut CborEncoder,
    absolute_value: u64,
) -> c_int {
    // SAFETY: module contract.
    encode_number(
        unsafe { as_mut(encoder) },
        absolute_value.wrapping_sub(1),
        major::NEGATIVE,
    )
}

/// Negative values encode as major type 1 carrying `-1 - n` (RFC 8949 §3.1), so
/// -1 is `0x20`. Taking the absolute value of `i64::MIN` would overflow, which
/// is why this goes through the unsigned complement rather than `abs()`.
#[no_mangle]
pub extern "C" fn cbor_encode_int(encoder: *mut CborEncoder, value: i64) -> c_int {
    // SAFETY: module contract.
    let e = unsafe { as_mut(encoder) };
    if value < 0 {
        encode_number(e, !(value as u64), major::NEGATIVE)
    } else {
        encode_number(e, value as u64, major::UNSIGNED)
    }
}

#[no_mangle]
pub extern "C" fn cbor_encode_tag(encoder: *mut CborEncoder, tag: u64) -> c_int {
    // A tag is a prefix, not an item of its own, so the container's item count
    // is left alone — hence encode_number_no_update upstream.
    // SAFETY: module contract.
    let e = unsafe { as_mut(encoder) };
    append_head(e, head(major::TAG, tag))
}

/// Simple values 25..=31 are the encodings CBOR uses for floats and the break
/// byte, so they cannot be requested directly.
///
/// The range is upstream's `value >= HalfPrecisionFloat && value <= Break`, and
/// it starts at 25, not 24. 24 is the escape byte that introduces a two-byte
/// simple value, and asking for it here produces `f8 18` -- which upstream's own
/// parser then rejects as a value under 32 written in two bytes. The encoder is
/// happy to write what the parser will not read. That asymmetry is upstream's,
/// and this port keeps it for the same reason it keeps the UTF-8 one: the claim
/// is equivalence with a commit, not with the specification.
#[no_mangle]
pub extern "C" fn cbor_encode_simple_value(encoder: *mut CborEncoder, value: u8) -> c_int {
    if (25..=31).contains(&value) {
        return ERR_ILLEGAL_SIMPLE_TYPE;
    }
    // SAFETY: module contract.
    encode_number(unsafe { as_mut(encoder) }, value as u64, major::SIMPLE)
}

// -- floats ----------------------------------------------------------------

/// `fp_type` is a `CborType`: 0xf9 half, 0xfa float, 0xfb double. The width is
/// `2 << (fp_type - HalfFloat)`, which is upstream's trick for turning those
/// three adjacent tags into 2, 4 and 8.
#[no_mangle]
pub extern "C" fn cbor_encode_floating_point(
    encoder: *mut CborEncoder,
    fp_type: c_int,
    value: *const c_void,
) -> c_int {
    let size = 2usize << (fp_type - 0xf9);
    let mut buf = [0u8; 9];
    buf[0] = fp_type as u8;

    // SAFETY: the caller promises `value` points to `size` readable bytes, which
    // is what the fp_type it passed means. Read unaligned because C only
    // guarantees the pointee's own alignment, not u64's.
    unsafe {
        match size {
            8 => buf[1..9].copy_from_slice(&(*(value as *const u64)).to_be_bytes()),
            4 => buf[1..5].copy_from_slice(&(*(value as *const u32)).to_be_bytes()),
            _ => buf[1..3].copy_from_slice(&(*(value as *const u16)).to_be_bytes()),
        }
    }

    // SAFETY: module contract.
    let e = unsafe { as_mut(encoder) };
    saturated_decrement(e);
    append(e, &buf[..size + 1], APPEND_CBOR_DATA)
}

#[no_mangle]
pub extern "C" fn cbor_encode_float_as_half_float(encoder: *mut CborEncoder, value: f32) -> c_int {
    let half = cbor_core::half::encode(value);
    let mut buf = [0u8; 3];
    buf[0] = 0xf9;
    buf[1..3].copy_from_slice(&half.to_be_bytes());
    // SAFETY: module contract.
    let e = unsafe { as_mut(encoder) };
    saturated_decrement(e);
    append(e, &buf, APPEND_CBOR_DATA)
}

// -- strings ---------------------------------------------------------------

/// The length head and the payload are two appends, and an out-of-memory from
/// the first is not fatal: the encoder is in counting mode and still needs to
/// count the payload, so the caller learns the true total.
fn encode_string(e: &mut CborEncoder, data: &[u8], major: u8) -> c_int {
    let err = encode_number(e, data.len() as u64, major);
    if err != NO_ERROR && !is_oom(err) {
        return err;
    }
    append(e, data, APPEND_STRING_DATA)
}

/// SAFETY: `p` and `len` must describe a readable region, or `p` may be null
/// when `len` is 0 — which C callers do pass for empty strings.
unsafe fn slice<'a>(p: *const u8, len: usize) -> &'a [u8] {
    if len == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(p, len)
    }
}

#[no_mangle]
pub extern "C" fn cbor_encode_text_string(
    encoder: *mut CborEncoder,
    string: *const c_char,
    length: usize,
) -> c_int {
    // SAFETY: module contract, plus the caller's string/length pair.
    unsafe {
        encode_string(
            as_mut(encoder),
            slice(string as *const u8, length),
            major::TEXT_STRING,
        )
    }
}

#[no_mangle]
pub extern "C" fn cbor_encode_byte_string(
    encoder: *mut CborEncoder,
    string: *const u8,
    length: usize,
) -> c_int {
    // SAFETY: module contract, plus the caller's string/length pair.
    unsafe { encode_string(as_mut(encoder), slice(string, length), major::BYTE_STRING) }
}

/// Writes bytes through untouched and without counting an item: the caller is
/// splicing in CBOR they encoded elsewhere.
#[no_mangle]
pub extern "C" fn cbor_encode_raw(
    encoder: *mut CborEncoder,
    raw: *const u8,
    length: usize,
) -> c_int {
    // SAFETY: module contract, plus the caller's raw/length pair.
    unsafe { append(as_mut(encoder), slice(raw, length), APPEND_RAW_DATA) }
}

// -- containers ------------------------------------------------------------

/// The child encoder shares the parent's buffer cursor; `close_container` copies
/// it back. `remaining` is the item count plus one for the container itself, and
/// a map counts twice per entry because keys and values are separate items.
fn create_container(
    parent: &mut CborEncoder,
    container: &mut CborEncoder,
    length: usize,
    major: u8,
) -> c_int {
    container.data = parent.data;
    container.end = parent.end;
    saturated_decrement(parent);
    container.remaining = length.wrapping_add(1);

    container.flags = (major as i32) & CONTAINER_IS_MAP;
    container.flags |= parent.flags & WRITER_FUNCTION;

    if length == INDEFINITE_LENGTH {
        container.flags |= UNKNOWN_LENGTH;
        append_head(container, indefinite_head(major))
    } else {
        if container.flags & CONTAINER_IS_MAP != 0 {
            container.remaining = container.remaining.wrapping_add(length);
        }
        append_head(container, head(major, length as u64))
    }
}

#[no_mangle]
pub extern "C" fn cbor_encoder_create_array(
    parent: *mut CborEncoder,
    array: *mut CborEncoder,
    length: usize,
) -> c_int {
    // SAFETY: module contract, for two distinct encoders.
    unsafe { create_container(as_mut(parent), as_mut(array), length, major::ARRAY) }
}

#[no_mangle]
pub extern "C" fn cbor_encoder_create_map(
    parent: *mut CborEncoder,
    map: *mut CborEncoder,
    length: usize,
) -> c_int {
    // SAFETY: module contract, for two distinct encoders.
    unsafe { create_container(as_mut(parent), as_mut(map), length, major::MAP) }
}

#[no_mangle]
pub extern "C" fn cbor_encoder_close_container(
    parent: *mut CborEncoder,
    container: *const CborEncoder,
) -> c_int {
    // SAFETY: module contract; `container` is only read.
    let (p, c) = unsafe { (as_mut(parent), &*container) };
    p.end = c.end;
    p.data = c.data;

    if c.flags & UNKNOWN_LENGTH != 0 {
        return append(p, &[BREAK], APPEND_CBOR_DATA);
    }
    if c.remaining != 1 {
        return if c.remaining == 0 {
            ERR_TOO_MANY_ITEMS
        } else {
            ERR_TOO_FEW_ITEMS
        };
    }
    if counting(p) {
        return ERR_OUT_OF_MEMORY; // preserve the shortfall count
    }
    NO_ERROR
}

/// A separate symbol upstream, but with `CBOR_NO_VALIDATION` off it is a plain
/// alias for the unchecked close. It stays a distinct exported symbol because
/// the ABI has one; it does not stay a distinct implementation, because
/// upstream's is one line delegating here.
#[no_mangle]
pub extern "C" fn cbor_encoder_close_container_checked(
    parent: *mut CborEncoder,
    container: *const CborEncoder,
) -> c_int {
    cbor_encoder_close_container(parent, container)
}
