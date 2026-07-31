//! The CBOR diagnostic-notation printer (`cbor_value_to_pretty_*`).
//!
//! Upstream streams this out through a printf-style callback, one fragment at a
//! time. Here the whole thing is rendered into a `String` first and emitted in
//! one write. That is simpler, and it means the formatting code never has to
//! care what the sink is — but it does mean a caller's stream function sees one
//! call instead of dozens. Nothing in the API promises a call count, and the
//! output bytes are identical.
//!
//! **SAFETY (whole module):** the `CborValue` pointer must be a live, valid
//! cursor, and `FILE*` must be an open stream. Same contract as the parser.

use crate::parser::{
    self, ERR_UNEXPECTED_EOF, NO_ERROR, SMALL_VALUE_MASK, TYPE_ARRAY, TYPE_BOOLEAN,
    TYPE_BYTE_STRING, TYPE_INTEGER, TYPE_INVALID, TYPE_MAP, TYPE_SIMPLE, TYPE_TAG,
    TYPE_TEXT_STRING, VALUE_8BIT,
};
use crate::types::CborValue;
use cbor_core::fmt::{escape_utf8, format_g};
use core::ffi::{c_char, c_int, c_void};

/// `CborPrettyFlags`.
const INDICATE_INDETERMINATE_LENGTH: c_int = 0x02;
const INDICATE_OVERLONG_NUMBERS: c_int = 0x04;
const NUMERIC_ENCODING_INDICATORS: c_int = 0x01;
const SHOW_STRING_FRAGMENTS: c_int = 0x100;
pub(crate) const DEFAULT_FLAGS: c_int = INDICATE_INDETERMINATE_LENGTH;

const TYPE_NULL: u8 = 0xf6;
const TYPE_UNDEFINED: u8 = 0xf7;
const TYPE_HALF_FLOAT: u8 = 0xf9;
const TYPE_FLOAT: u8 = 0xfa;
const TYPE_DOUBLE: u8 = 0xfb;

const MAX_RECURSIONS: i32 = 1024;
const ERR_INVALID_UTF8: c_int = 516;
const ERR_UNSUPPORTED_TYPE: c_int = 1026;

extern "C" {
    fn fwrite(ptr: *const c_void, size: usize, nmemb: usize, stream: *mut c_void) -> usize;
}

/// Which `_N` suffix, if any, follows this item.
///
/// `_` marks an indefinite length; `_0`.._3` mark a number written in more
/// bytes than it needed. Both are off unless the caller asks, and only the
/// first is on by default.
fn indicator(it: &CborValue, flags: c_int) -> &'static str {
    let Some(head) = parser::read_byte(it) else {
        return "";
    };
    let ai = head & SMALL_VALUE_MASK;
    if ai < VALUE_8BIT {
        return "";
    }
    if flags & INDICATE_INDETERMINATE_LENGTH != 0 && ai == 31 {
        return "_";
    }
    if flags & INDICATE_OVERLONG_NUMBERS == 0 {
        return "";
    }

    let value = parser::extract_int64(it);
    // The shortest additional-info that could have carried this value.
    let mut expected = VALUE_8BIT - 1;
    if value >= VALUE_8BIT as u64 {
        expected += 1;
    }
    if value > 0xff {
        expected += 1;
    }
    if value > 0xffff {
        expected += 1;
    }
    if value > 0xffff_ffff {
        expected += 1;
    }
    if expected == ai {
        return "";
    }
    match ai {
        24 => "_0",
        25 => "_1",
        26 => "_2",
        27 => "_3",
        _ => "",
    }
}

/// True when `v` is a whole number that fits in a `u64`, in which case upstream
/// prints it as an integer with a trailing dot rather than in `%g` form.
fn as_whole_u64(v: f64) -> Option<u64> {
    let v = v.abs();
    // 2^64 exactly. NaN is spelled out rather than left to fall out of a
    // negated comparison, because that reads as a mistake even when it is not.
    if v.is_nan() || v >= 18446744073709551616.0 {
        return None;
    }
    let i = v as u64;
    (i as f64 == v).then_some(i)
}

fn value_to_pretty(
    out: &mut String,
    it: *mut CborValue,
    flags: c_int,
    recursions_left: i32,
) -> c_int {
    // SAFETY: module contract.
    let v = unsafe { &*it };
    let type_ = v.type_;

    match type_ {
        TYPE_ARRAY | TYPE_MAP => {
            let ind = indicator(v, flags);
            let space = if ind.is_empty() { "" } else { " " };
            out.push(if type_ == TYPE_ARRAY { '[' } else { '{' });
            out.push_str(ind);
            out.push_str(space);

            let mut recursed = parser::clone(v);
            let err = crate::parser::cbor_value_enter_container(it, &mut recursed);
            if err != NO_ERROR {
                return err;
            }
            let err = container_to_pretty(out, &mut recursed, type_, flags, recursions_left - 1);
            if err != NO_ERROR {
                return err;
            }
            let err = crate::parser::cbor_value_leave_container(it, &recursed);
            if err != NO_ERROR {
                return err;
            }
            out.push(if type_ == TYPE_ARRAY { ']' } else { '}' });
            return NO_ERROR;
        }

        TYPE_INTEGER => {
            let raw = parser::extract_int64(v);
            if v.flags & parser::F_NEGATIVE != 0 {
                // Stored as -1-n, so the printed magnitude is n+1 and the
                // largest one overflows a u64 by exactly one.
                match raw.checked_add(1) {
                    Some(m) => {
                        out.push('-');
                        push_u64_into(out, m);
                    }
                    None => out.push_str("-18446744073709551616"),
                }
            } else {
                push_u64_into(out, raw);
            }
            out.push_str(indicator(v, flags));
        }

        TYPE_BYTE_STRING | TYPE_TEXT_STRING => {
            return string_to_pretty(out, it, type_, flags);
        }

        TYPE_TAG => {
            let tag = parser::extract_int64(v);
            push_u64_into(out, tag);
            out.push_str(indicator(v, flags));
            out.push('(');
            let err = crate::parser::cbor_value_advance_fixed(it);
            if err != NO_ERROR {
                return err;
            }
            if recursions_left > 0 {
                let err = value_to_pretty(out, it, flags, recursions_left - 1);
                if err != NO_ERROR {
                    return err;
                }
            } else {
                out.push_str("<nesting too deep, recursion stopped>");
            }
            out.push(')');
            return NO_ERROR;
        }

        TYPE_SIMPLE => {
            out.push_str("simple(");
            push_u64_into(out, v.extra as u64);
            out.push(')');
        }

        TYPE_NULL => out.push_str("null"),
        TYPE_UNDEFINED => out.push_str("undefined"),
        TYPE_BOOLEAN => out.push_str(if v.extra != 0 { "true" } else { "false" }),

        TYPE_HALF_FLOAT | TYPE_FLOAT | TYPE_DOUBLE => {
            let (val, mut suffix) = match type_ {
                TYPE_HALF_FLOAT => (
                    cbor_core::half::decode(v.extra) as f64,
                    if flags & NUMERIC_ENCODING_INDICATORS != 0 {
                        "_1"
                    } else {
                        "f16"
                    },
                ),
                TYPE_FLOAT => (
                    f32::from_bits(parser::decode_int64_internal(v) as u32) as f64,
                    if flags & NUMERIC_ENCODING_INDICATORS != 0 {
                        "_2"
                    } else {
                        "f"
                    },
                ),
                _ => (f64::from_bits(parser::decode_int64_internal(v)), ""),
            };
            // Without numeric indicators, nan and inf carry no suffix: there is
            // no ambiguity to disambiguate.
            if flags & NUMERIC_ENCODING_INDICATORS == 0 && (val.is_nan() || val.is_infinite()) {
                suffix = "";
            }
            match as_whole_u64(val) {
                Some(i) => {
                    if val < 0.0 {
                        out.push('-');
                    }
                    push_u64_into(out, i);
                    out.push('.');
                }
                None => out.push_str(&format_g(val, 17)),
            }
            out.push_str(suffix);
        }

        TYPE_INVALID => return ERR_UNEXPECTED_EOF,
        _ => return ERR_UNSUPPORTED_TYPE,
    }

    // SAFETY: module contract.
    crate::parser::cbor_value_advance_fixed(it)
}

fn container_to_pretty(
    out: &mut String,
    it: &mut CborValue,
    container: u8,
    flags: c_int,
    recursions_left: i32,
) -> c_int {
    if recursions_left <= 0 {
        out.push_str("<nesting too deep, recursion stopped>");
        while it.type_ != TYPE_INVALID {
            let err = crate::parser::cbor_value_advance(it);
            if err != NO_ERROR {
                return err;
            }
        }
        return NO_ERROR;
    }

    let mut comma = "";
    while it.type_ != TYPE_INVALID {
        out.push_str(comma);
        comma = ", ";

        let err = value_to_pretty(out, it, flags, recursions_left);
        if err != NO_ERROR {
            return err;
        }
        if container == TYPE_ARRAY {
            continue;
        }
        // A map: that was the key, now the value.
        out.push_str(": ");
        let err = value_to_pretty(out, it, flags, recursions_left);
        if err != NO_ERROR {
            return err;
        }
    }
    NO_ERROR
}

fn string_to_pretty(out: &mut String, it: *mut CborValue, type_: u8, flags: c_int) -> c_int {
    // SAFETY: module contract.
    let v = unsafe { &*it };
    let is_text = type_ == TYPE_TEXT_STRING;
    let open = if is_text { "\"" } else { "h'" };
    let close = if is_text { '"' } else { '\'' };
    let chunked = v.flags & parser::F_UNKNOWN_LENGTH != 0;
    let showing_fragments = flags & SHOW_STRING_FRAGMENTS != 0 && chunked;

    if showing_fragments {
        out.push_str("(_ ");
    } else {
        out.push_str(open);
    }

    let mut trailing = "";
    crate::parser::_cbor_value_begin_string_iteration(it);
    let mut separator = "";
    loop {
        // The indicator is read before the chunk is consumed.
        if showing_fragments || trailing.is_empty() {
            trailing = indicator(unsafe { &*it }, flags);
        }

        let mut ptr: *const c_void = core::ptr::null();
        let mut len: usize = 0;
        let err = crate::parser::_cbor_value_get_string_chunk(it, &mut ptr, &mut len, it);
        if err == parser::ERR_NO_MORE_STRING_CHUNKS {
            let err = crate::parser::_cbor_value_finish_string_iteration(it);
            if err != NO_ERROR {
                return err;
            }
            break;
        }
        if err != NO_ERROR {
            return err;
        }

        if showing_fragments {
            out.push_str(separator);
            out.push_str(open);
        }
        // SAFETY: the chunk API just handed back a pointer and length inside
        // the parse buffer.
        let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
        if is_text {
            if escape_utf8(out, bytes).is_err() {
                return ERR_INVALID_UTF8;
            }
        } else {
            for b in bytes {
                push_hex(out, *b);
            }
        }
        if showing_fragments {
            out.push(close);
            out.push_str(trailing);
            separator = ", ";
        }
    }

    if showing_fragments {
        out.push(')');
    } else {
        out.push(close);
        out.push_str(trailing);
    }
    NO_ERROR
}

pub(crate) fn push_u64_into(out: &mut String, mut v: u64) {
    if v == 0 {
        out.push('0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    while v != 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    out.push_str(core::str::from_utf8(&buf[i..]).expect("ascii digits"));
}

fn push_hex(out: &mut String, b: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push(HEX[(b >> 4) as usize] as char);
    out.push(HEX[(b & 0xf) as usize] as char);
}

// -- entry points ----------------------------------------------------------

/// Renders exactly one value and advances past it. tojson uses this for the
/// types whose diagnostic notation is already valid JSON, and for stringified
/// map keys.
pub(crate) fn render_one(out: &mut String, value: *mut CborValue, flags: c_int) -> c_int {
    value_to_pretty(out, value, flags, MAX_RECURSIONS)
}

fn render(value: *mut CborValue, flags: c_int) -> Result<String, c_int> {
    let mut out = String::new();
    let err = value_to_pretty(&mut out, value, flags, MAX_RECURSIONS);
    if err != NO_ERROR {
        return Err(err);
    }
    Ok(out)
}

#[no_mangle]
pub extern "C" fn cbor_value_to_pretty_advance_flags(
    out: *mut c_void,
    value: *mut CborValue,
    flags: c_int,
) -> c_int {
    match render(value, flags) {
        Err(e) => e,
        Ok(s) => {
            // SAFETY: `out` is an open FILE* per the module contract.
            unsafe { fwrite(s.as_ptr() as *const c_void, 1, s.len(), out) };
            NO_ERROR
        }
    }
}

#[no_mangle]
pub extern "C" fn cbor_value_to_pretty_advance(out: *mut c_void, value: *mut CborValue) -> c_int {
    cbor_value_to_pretty_advance_flags(out, value, DEFAULT_FLAGS)
}

/// The caller's sink is a printf-style variadic function. Handing it a literal
/// `"%s"` plus the rendered text is the only way to drive it without knowing
/// what format specifiers it supports.
type StreamFunction = unsafe extern "C" fn(*mut c_void, *const c_char, ...) -> c_int;

#[no_mangle]
pub extern "C" fn cbor_value_to_pretty_stream(
    stream: *mut c_void,
    token: *mut c_void,
    value: *mut CborValue,
    flags: c_int,
) -> c_int {
    match render(value, flags) {
        Err(e) => e,
        Ok(s) => {
            let mut owned = s.into_bytes();
            owned.push(0);
            // SAFETY: the caller passed a CborStreamFunction, whose signature is
            // exactly this. The strings handed to it are NUL-terminated.
            unsafe {
                let f: StreamFunction = core::mem::transmute(stream);
                f(token, c"%s".as_ptr(), owned.as_ptr() as *const c_char)
            }
        }
    }
}
