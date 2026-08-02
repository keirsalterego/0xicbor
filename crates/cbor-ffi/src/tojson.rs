//! `cbor_value_to_json_advance`: CBOR to JSON.
//!
//! CBOR is a strict superset of JSON's data model, so this conversion is lossy
//! by construction — byte strings, tags, undefined, simple values, NaN and
//! infinity have no JSON spelling. Upstream's answer is to pick a reasonable
//! representation for each and, if `CborConvertAddMetadata` is set, emit a
//! sidecar object describing what was lost so the original can be rebuilt.
//!
//! Output accumulates in a `Vec<u8>` rather than a `String` because a CBOR text
//! string is not guaranteed to be valid UTF-8 and upstream passes the bytes
//! through unvalidated.
//!
//! **SAFETY (whole module):** same contract as the parser.

use crate::parser::{
    self, NO_ERROR, TYPE_ARRAY, TYPE_BOOLEAN, TYPE_BYTE_STRING, TYPE_INTEGER, TYPE_INVALID,
    TYPE_MAP, TYPE_SIMPLE, TYPE_TAG, TYPE_TEXT_STRING,
};
use crate::pretty;
use crate::types::CborValue;
use cbor_core::fmt::{base16, base64, base64url, escape_json, format_g};
use core::ffi::{c_int, c_void};

/// `CborToJsonFlags`.
const ADD_METADATA: c_int = 1;
const TAGS_TO_OBJECTS: c_int = 2;
const BYTE_STRINGS_TO_BASE64URL: c_int = 4;
const STRINGIFY_MAP_KEYS: c_int = 8;

/// `ConversionStatusFlags`. The low byte holds a `CborType`, so the flags start
/// above it.
const TYPE_WAS_NOT_NATIVE: u32 = 0x100;
const TYPE_WAS_TAGGED: u32 = 0x200;
const NUMBER_WAS_NAN: u32 = 0x800;
const NUMBER_WAS_INFINITE: u32 = 0x1000;
const NUMBER_WAS_NEGATIVE: u32 = 0x2000;
const FINAL_TYPE_MASK: u32 = 0xff;

const TYPE_NULL: u8 = 0xf6;
const TYPE_UNDEFINED: u8 = 0xf7;
const TYPE_HALF_FLOAT: u8 = 0xf9;
const TYPE_FLOAT: u8 = 0xfa;
const TYPE_DOUBLE: u8 = 0xfb;

const TAG_NEGATIVE_BIGNUM: u64 = 3;
const TAG_EXPECTED_BASE64: u64 = 22;
const TAG_EXPECTED_BASE16: u64 = 23;

const ERR_IO: c_int = 4;
const ERR_UNKNOWN_TYPE: c_int = 259;
const ERR_NESTING_TOO_DEEP: c_int = 1025;
const ERR_JSON_KEY_NOT_STRING: c_int = 1281;

const MAX_RECURSIONS: i32 = 1024;

extern "C" {
    fn fwrite(ptr: *const c_void, size: usize, nmemb: usize, stream: *mut c_void) -> usize;
}

/// What the last converted value lost on the way to JSON.
#[derive(Default)]
struct Status {
    flags: u32,
    last_tag: u64,
    original_number: u64,
}

/// Collects a whole string, chunked or not, as raw bytes.
fn read_string(it: *mut CborValue) -> Result<Vec<u8>, c_int> {
    let mut bytes = Vec::new();
    crate::parser::_cbor_value_begin_string_iteration(it);
    loop {
        let mut ptr: *const c_void = core::ptr::null();
        let mut len: usize = 0;
        let err = crate::parser::_cbor_value_get_string_chunk(it, &mut ptr, &mut len, it);
        if err == parser::ERR_NO_MORE_STRING_CHUNKS {
            let err = crate::parser::_cbor_value_finish_string_iteration(it);
            if err != NO_ERROR {
                return Err(err);
            }
            return Ok(bytes);
        }
        if err != NO_ERROR {
            return Err(err);
        }
        // SAFETY: the chunk API just returned this range inside the buffer.
        bytes.extend_from_slice(unsafe { core::slice::from_raw_parts(ptr as *const u8, len) });
    }
}

fn push(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
}

fn push_u64(out: &mut Vec<u8>, v: u64) {
    let mut s = String::new();
    pretty::push_u64_into(&mut s, v);
    push(out, &s);
}

/// Walks past any run of tags, reporting the innermost type and the last tag.
fn find_tagged_type(it: *mut CborValue, nesting: i32) -> Result<(u64, u8), c_int> {
    let mut tag = 0u64;
    let mut nesting = nesting;
    // SAFETY: module contract.
    let mut type_ = unsafe { (*it).type_ };
    while type_ == TYPE_TAG {
        if nesting == 0 {
            return Err(ERR_NESTING_TOO_DEEP);
        }
        nesting -= 1;
        // SAFETY: module contract.
        tag = parser::extract_int64(unsafe { &*it });
        let err = crate::parser::cbor_value_advance_fixed(it);
        if err != NO_ERROR {
            return Err(err);
        }
        // SAFETY: module contract.
        type_ = unsafe { (*it).type_ };
    }
    Ok((tag, type_))
}

/// The `$cbor` sidecar describing what a value lost in conversion.
fn add_value_metadata(out: &mut Vec<u8>, mut type_: u8, status: &Status) {
    let mut flags = status.flags;
    if flags & TYPE_WAS_TAGGED != 0 {
        // The tagged type may itself be JSON-native, so unwrap it first.
        type_ = (flags & FINAL_TYPE_MASK) as u8;
        flags &= !(FINAL_TYPE_MASK | TYPE_WAS_TAGGED);

        push(out, "\"tag\":\"");
        push_u64(out, status.last_tag);
        push(out, "\"");
        if flags != 0 {
            push(out, ",");
        }
    }
    if flags == 0 {
        return;
    }

    push(out, "\"t\":");
    push_u64(out, type_ as u64);

    if flags & NUMBER_WAS_NAN != 0 {
        push(out, ",\"v\":\"nan\"");
    }
    if flags & NUMBER_WAS_INFINITE != 0 {
        push(out, ",\"v\":\"");
        if flags & NUMBER_WAS_NEGATIVE != 0 {
            push(out, "-");
        }
        push(out, "inf\"");
    }
    if type_ == TYPE_SIMPLE {
        push(out, ",\"v\":");
        push_u64(out, status.original_number);
    }
}

fn value_to_json(
    out: &mut Vec<u8>,
    it: *mut CborValue,
    flags: c_int,
    type_: u8,
    nesting: i32,
    status: &mut Status,
) -> c_int {
    status.flags = 0;
    if nesting == 0 {
        return ERR_NESTING_TOO_DEEP;
    }

    match type_ {
        TYPE_ARRAY | TYPE_MAP => {
            // SAFETY: module contract.
            let mut recursed = parser::clone(unsafe { &*it });
            let err = crate::parser::cbor_value_enter_container(it, &mut recursed);
            if err != NO_ERROR {
                return err;
            }
            push(out, if type_ == TYPE_ARRAY { "[" } else { "{" });
            let err = if type_ == TYPE_ARRAY {
                array_to_json(out, &mut recursed, flags, nesting - 1, status)
            } else {
                map_to_json(out, &mut recursed, flags, nesting - 1, status)
            };
            if err != NO_ERROR {
                return err;
            }
            push(out, if type_ == TYPE_ARRAY { "]" } else { "}" });
            let err = crate::parser::cbor_value_leave_container(it, &recursed);
            if err != NO_ERROR {
                return err;
            }
            // A container never loses anything itself.
            status.flags = 0;
            return NO_ERROR;
        }

        // These three have exact JSON spellings, and diagnostic notation already
        // produces them, so upstream reuses the pretty printer verbatim.
        TYPE_INTEGER | TYPE_NULL | TYPE_BOOLEAN => {
            let mut s = String::new();
            let err = pretty::render_one(&mut s, it, pretty::DEFAULT_FLAGS);
            if err != NO_ERROR {
                return err;
            }
            push(out, &s);
            return NO_ERROR;
        }

        TYPE_BYTE_STRING => {
            let bytes = match read_string(it) {
                Ok(b) => b,
                Err(e) => return e,
            };
            push(out, "\"");
            base64url(out, &bytes);
            push(out, "\"");
            status.flags = TYPE_WAS_NOT_NATIVE;
            return NO_ERROR;
        }

        TYPE_TEXT_STRING => {
            let bytes = match read_string(it) {
                Ok(b) => b,
                Err(e) => return e,
            };
            push(out, "\"");
            escape_json(out, &bytes);
            push(out, "\"");
            return NO_ERROR;
        }

        TYPE_TAG => return tagged_value_to_json(out, it, flags, nesting - 1, status),

        TYPE_SIMPLE => {
            // SAFETY: module contract.
            let simple = unsafe { (*it).extra } as u64;
            status.flags = TYPE_WAS_NOT_NATIVE;
            status.original_number = simple;
            push(out, "\"simple(");
            push_u64(out, simple);
            push(out, ")\"");
        }

        TYPE_UNDEFINED => {
            status.flags = TYPE_WAS_NOT_NATIVE;
            push(out, "\"undefined\"");
        }

        TYPE_HALF_FLOAT | TYPE_FLOAT | TYPE_DOUBLE => {
            // SAFETY: module contract.
            let v = unsafe { &*it };
            let val: f64 = match type_ {
                TYPE_DOUBLE => f64::from_bits(parser::decode_int64_internal(v)),
                TYPE_FLOAT => {
                    status.flags = TYPE_WAS_NOT_NATIVE;
                    f32::from_bits(parser::decode_int64_internal(v) as u32) as f64
                }
                _ => {
                    status.flags = TYPE_WAS_NOT_NATIVE;
                    cbor_core::half::decode(v.extra) as f64
                }
            };

            if val.is_nan() {
                push(out, "null");
                status.flags |= NUMBER_WAS_NAN;
            } else if val.is_infinite() {
                push(out, "null");
                status.flags |= NUMBER_WAS_INFINITE;
                if val < 0.0 {
                    status.flags |= NUMBER_WAS_NEGATIVE;
                }
            } else {
                // A double that happens to be integral prints as an integer so
                // no precision is lost in the JSON text; the metadata records
                // that it was really a double.
                let a = val.abs();
                let ival = a as u64;
                if a < 18446744073709551616.0 && ival as f64 == a {
                    if val < 0.0 {
                        push(out, "-");
                    }
                    push_u64(out, ival);
                    status.flags |= TYPE_WAS_NOT_NATIVE;
                } else {
                    push(out, &format_g(val, 17));
                }
            }
        }

        TYPE_INVALID => return ERR_UNKNOWN_TYPE,
        _ => return ERR_UNKNOWN_TYPE,
    }

    crate::parser::cbor_value_advance_fixed(it)
}

fn tagged_value_to_json(
    out: &mut Vec<u8>,
    it: *mut CborValue,
    flags: c_int,
    nesting: i32,
    status: &mut Status,
) -> c_int {
    if flags & TAGS_TO_OBJECTS != 0 {
        // SAFETY: module contract.
        let tag = parser::extract_int64(unsafe { &*it });
        let err = crate::parser::cbor_value_advance_fixed(it);
        if err != NO_ERROR {
            return err;
        }
        push(out, "{\"tag");
        push_u64(out, tag);
        push(out, "\":");

        // SAFETY: module contract.
        let type_ = unsafe { (*it).type_ };
        let err = value_to_json(out, it, flags, type_, nesting, status);
        if err != NO_ERROR {
            return err;
        }
        if flags & ADD_METADATA != 0 && status.flags != 0 {
            push(out, ",\"tag");
            push_u64(out, tag);
            push(out, "$cbor\":{");
            add_value_metadata(out, type_, status);
            push(out, "}");
        }
        push(out, "}");
        status.flags = TYPE_WAS_NOT_NATIVE | TYPE_TAG as u32;
        return NO_ERROR;
    }

    let (tag, type_) = match find_tagged_type(it, nesting) {
        Ok(v) => v,
        Err(e) => return e,
    };
    status.last_tag = tag;

    // A byte string carrying one of the base-encoding tags is rendered in the
    // encoding the tag asks for instead of the base64url default.
    if type_ == TYPE_BYTE_STRING
        && flags & BYTE_STRINGS_TO_BASE64URL == 0
        && matches!(
            tag,
            TAG_NEGATIVE_BIGNUM | TAG_EXPECTED_BASE16 | TAG_EXPECTED_BASE64
        )
    {
        let bytes = match read_string(it) {
            Ok(b) => b,
            Err(e) => return e,
        };
        push(out, "\"");
        if tag == TAG_NEGATIVE_BIGNUM {
            // A negative bignum is -1 - n, and JSON has no way to say that, so
            // upstream marks it with a leading tilde.
            push(out, "~");
            base64url(out, &bytes);
        } else if tag == TAG_EXPECTED_BASE64 {
            base64(out, &bytes);
        } else {
            base16(out, &bytes);
        }
        push(out, "\"");
        status.flags = TYPE_WAS_NOT_NATIVE | TYPE_WAS_TAGGED | TYPE_BYTE_STRING as u32;
        return NO_ERROR;
    }

    let err = value_to_json(out, it, flags, type_, nesting, status);
    status.flags |= TYPE_WAS_TAGGED | type_ as u32;
    err
}

fn array_to_json(
    out: &mut Vec<u8>,
    it: &mut CborValue,
    flags: c_int,
    nesting: i32,
    status: &mut Status,
) -> c_int {
    let mut comma = "";
    while it.type_ != TYPE_INVALID {
        push(out, comma);
        comma = ",";
        let type_ = it.type_;
        let err = value_to_json(out, it, flags, type_, nesting, status);
        if err != NO_ERROR {
            return err;
        }
    }
    NO_ERROR
}

fn map_to_json(
    out: &mut Vec<u8>,
    it: &mut CborValue,
    flags: c_int,
    nesting: i32,
    status: &mut Status,
) -> c_int {
    let mut comma = "";
    while it.type_ != TYPE_INVALID {
        push(out, comma);
        comma = ",";

        let key_type = it.type_;
        // JSON object keys must be strings. A non-string key is either an error
        // or, with StringifyMapKeys, its diagnostic-notation form escaped into
        // a string.
        let key: Vec<u8> = if key_type == TYPE_TEXT_STRING {
            match read_string(it) {
                Ok(b) => {
                    let mut e = Vec::new();
                    escape_json(&mut e, &b);
                    e
                }
                Err(e) => return e,
            }
        } else if flags & STRINGIFY_MAP_KEYS != 0 {
            let mut s = String::new();
            let err = pretty::render_one(&mut s, it, pretty::DEFAULT_FLAGS);
            if err != NO_ERROR {
                return err;
            }
            let mut e = Vec::new();
            escape_json(&mut e, s.as_bytes());
            e
        } else {
            return ERR_JSON_KEY_NOT_STRING;
        };

        push(out, "\"");
        out.extend_from_slice(&key);
        push(out, "\":");

        let value_type = it.type_;
        let err = value_to_json(out, it, flags, value_type, nesting, status);
        if err != NO_ERROR {
            return err;
        }

        if flags & ADD_METADATA != 0 {
            if key_type != TYPE_TEXT_STRING {
                push(out, ",\"");
                out.extend_from_slice(&key);
                push(out, "$keycbordump\":true");
            }
            if status.flags != 0 {
                push(out, ",\"");
                out.extend_from_slice(&key);
                push(out, "$cbor\":{");
                add_value_metadata(out, value_type, status);
                push(out, "}");
            }
        }
    }
    NO_ERROR
}

#[no_mangle]
pub extern "C" fn cbor_value_to_json_advance(
    out: *mut c_void,
    value: *mut CborValue,
    flags: c_int,
) -> c_int {
    let mut buf = Vec::new();
    let mut status = Status::default();
    // SAFETY: module contract.
    let type_ = unsafe { (*value).type_ };
    let err = value_to_json(&mut buf, value, flags, type_, MAX_RECURSIONS, &mut status);
    if err != NO_ERROR {
        return err;
    }
    // Upstream checks every fprintf into the caller's FILE* and answers
    // CborErrorIO when one fails. Buffering the whole document and writing it
    // once leaves a single return to check, and a full sink is still a failed
    // conversion rather than a truncated one.
    //
    // SAFETY: `out` is an open FILE* per the module contract.
    let written = unsafe { fwrite(buf.as_ptr() as *const c_void, 1, buf.len(), out) };
    if written == buf.len() {
        NO_ERROR
    } else {
        ERR_IO
    }
}
