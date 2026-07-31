//! `cbor_value_validate`: the strict and canonical-mode checks.
//!
//! `cbor_value_validate_basic` only asks whether the stream is well-formed —
//! that is a walk, and it lives in the parser. This module answers the harder
//! question: is the stream well-formed *and* does it obey the extra rules the
//! caller asked for. Shortest-form numbers, sorted map keys, UTF-8, tag/type
//! agreement, no indefinite lengths, and so on.
//!
//! **SAFETY (whole module):** same contract as the parser — the `CborValue`
//! must be a live cursor over a buffer that outlives the call.

use crate::parser::{
    self, F_UNKNOWN_LENGTH, NO_ERROR, TYPE_ARRAY, TYPE_BOOLEAN, TYPE_BYTE_STRING, TYPE_INTEGER,
    TYPE_INVALID, TYPE_MAP, TYPE_SIMPLE, TYPE_TAG, TYPE_TEXT_STRING, VALUE_8BIT,
};
use crate::types::CborValue;
use core::ffi::{c_int, c_void};

// CborValidationFlags. The composite ones really are supersets in the C enum,
// and several checks test for the *whole* composite rather than one bit, so the
// values are reproduced exactly rather than simplified.
const SHORTEST_INTEGRALS: u32 = 0x0001;
const SHORTEST_FLOATING_POINT: u32 = 0x0002;
const NO_INDETERMINATE_LENGTH: u32 = 0x0100;
const MAP_IS_SORTED: u32 = 0x0200 | NO_INDETERMINATE_LENGTH;
const MAP_KEYS_ARE_UNIQUE: u32 = 0x1000 | MAP_IS_SORTED;
const TAG_USE: u32 = 0x2000;
const UTF8: u32 = 0x4000;
const MAP_KEYS_ARE_STRING: u32 = 0x0010_0000;
const NO_UNDEFINED: u32 = 0x0020_0000;
const NO_TAGS: u32 = 0x0040_0000;
const FINITE_FLOATING_POINT: u32 = 0x0080_0000;
const NO_UNKNOWN_SIMPLE_TYPES_SA: u32 = 0x0400_0000;
const NO_UNKNOWN_SIMPLE_TYPES: u32 = 0x0800_0000 | NO_UNKNOWN_SIMPLE_TYPES_SA;
const NO_UNKNOWN_TAGS_SA: u32 = 0x1000_0000;
const NO_UNKNOWN_TAGS_SR: u32 = 0x2000_0000 | NO_UNKNOWN_TAGS_SA;
const NO_UNKNOWN_TAGS: u32 = 0x4000_0000 | NO_UNKNOWN_TAGS_SR;
const COMPLETE_DATA: u32 = 0x8000_0000;

const TYPE_NULL: u8 = 0xf6;
const TYPE_UNDEFINED: u8 = 0xf7;
const TYPE_HALF_FLOAT: u8 = 0xf9;
const TYPE_FLOAT: u8 = 0xfa;
const TYPE_DOUBLE: u8 = 0xfb;

const ERR_UNKNOWN_LENGTH: c_int = 2;
const ERR_GARBAGE_AT_END: c_int = 256;
const ERR_UNKNOWN_TYPE: c_int = 259;
const ERR_UNKNOWN_SIMPLE_TYPE: c_int = 512;
const ERR_UNKNOWN_TAG: c_int = 513;
const ERR_INAPPROPRIATE_TAG_FOR_TYPE: c_int = 514;
const ERR_INVALID_UTF8: c_int = 516;
const ERR_EXCLUDED_TYPE: c_int = 517;
const ERR_EXCLUDED_VALUE: c_int = 518;
const ERR_IMPROPER_VALUE: c_int = 519;
const ERR_OVERLONG_ENCODING: c_int = 520;
const ERR_MAP_KEY_NOT_STRING: c_int = 521;
const ERR_MAP_NOT_SORTED: c_int = 522;
const ERR_MAP_KEYS_NOT_UNIQUE: c_int = 523;
const ERR_NESTING_TOO_DEEP: c_int = 1025;

const MAX_RECURSIONS: i32 = 1024;

/// Tags whose content type is constrained, and what they allow.
///
/// `types` packs up to three permitted `CborType` bytes, low byte first, which
/// is upstream's encoding. Zero means "any type". The table is sorted by tag,
/// and the lookup relies on that.
struct KnownTag {
    tag: u64,
    types: u32,
}

const STRING_OR_ARRAY_OR_MAP: u32 =
    TYPE_BYTE_STRING as u32 | ((TYPE_ARRAY as u32) << 8) | ((TYPE_MAP as u32) << 16);

const KNOWN_TAGS: &[KnownTag] = &[
    KnownTag {
        tag: 0,
        types: TYPE_TEXT_STRING as u32,
    },
    // Tag 1 is epoch time, which may be an integer. CborIntegerType is 0, and
    // zero already means "any", so upstream stores it as 1 and compensates in
    // the comparison below.
    KnownTag {
        tag: 1,
        types: TYPE_INTEGER as u32 + 1,
    },
    KnownTag {
        tag: 2,
        types: TYPE_BYTE_STRING as u32,
    },
    KnownTag {
        tag: 3,
        types: TYPE_BYTE_STRING as u32,
    },
    KnownTag {
        tag: 4,
        types: TYPE_ARRAY as u32,
    },
    KnownTag {
        tag: 5,
        types: TYPE_ARRAY as u32,
    },
    KnownTag {
        tag: 16,
        types: TYPE_ARRAY as u32,
    },
    KnownTag {
        tag: 17,
        types: TYPE_ARRAY as u32,
    },
    KnownTag {
        tag: 18,
        types: TYPE_ARRAY as u32,
    },
    KnownTag {
        tag: 21,
        types: STRING_OR_ARRAY_OR_MAP,
    },
    KnownTag {
        tag: 22,
        types: STRING_OR_ARRAY_OR_MAP,
    },
    KnownTag {
        tag: 23,
        types: STRING_OR_ARRAY_OR_MAP,
    },
    KnownTag {
        tag: 24,
        types: TYPE_BYTE_STRING as u32,
    },
    KnownTag {
        tag: 32,
        types: TYPE_TEXT_STRING as u32,
    },
    KnownTag {
        tag: 33,
        types: TYPE_TEXT_STRING as u32,
    },
    KnownTag {
        tag: 34,
        types: TYPE_TEXT_STRING as u32,
    },
    KnownTag {
        tag: 35,
        types: TYPE_TEXT_STRING as u32,
    },
    KnownTag {
        tag: 36,
        types: TYPE_TEXT_STRING as u32,
    },
    KnownTag {
        tag: 96,
        types: TYPE_ARRAY as u32,
    },
    KnownTag {
        tag: 97,
        types: TYPE_ARRAY as u32,
    },
    KnownTag {
        tag: 98,
        types: TYPE_ARRAY as u32,
    },
    KnownTag {
        tag: 55799,
        types: 0,
    },
];

fn length_known(it: &CborValue) -> bool {
    it.flags & F_UNKNOWN_LENGTH == 0
}

/// Rejects a number written in more bytes than its value needed.
///
/// Floats are exempt here and checked by `validate_floating_point`, which has a
/// different notion of "shortest".
fn validate_number(it: &CborValue, type_: u8, flags: u32) -> c_int {
    if flags & SHORTEST_INTEGRALS == 0 {
        return NO_ERROR;
    }
    if (TYPE_HALF_FLOAT..=TYPE_DOUBLE).contains(&type_) {
        return NO_ERROR;
    }

    let Some(head) = parser::read_byte(it) else {
        return parser::ERR_UNEXPECTED_EOF;
    };
    let used = parser::bytes_needed(head & 0x1f);
    // Fresh from the cursor, not from `extra`: when validating the chunks of an
    // indefinite-length string the cursor is on a chunk head that was never
    // preparsed, so the cached value belongs to the enclosing string.
    let Some(value) = parser::number_at_cursor(it) else {
        return parser::ERR_UNEXPECTED_EOF;
    };

    let mut needed = 0usize;
    if value >= VALUE_8BIT as u64 {
        needed += 1;
    }
    if value > 0xff {
        needed += 1;
    }
    if value > 0xffff {
        needed += 2;
    }
    if value > 0xffff_ffff {
        needed += 4;
    }
    if needed < used {
        return ERR_OVERLONG_ENCODING;
    }
    NO_ERROR
}

/// Every simple type the parser recognises is turned into its own CBOR type, so
/// anything still typed "simple" by the time it gets here is one we don't know.
fn validate_simple_type(simple: u8, flags: u32) -> c_int {
    if simple < 32 {
        return if flags & NO_UNKNOWN_SIMPLE_TYPES_SA != 0 {
            ERR_UNKNOWN_SIMPLE_TYPE
        } else {
            NO_ERROR
        };
    }
    if flags & NO_UNKNOWN_SIMPLE_TYPES == NO_UNKNOWN_SIMPLE_TYPES {
        ERR_UNKNOWN_SIMPLE_TYPE
    } else {
        NO_ERROR
    }
}

/// "Shortest" for a float means no narrower encoding would have been exact.
fn validate_floating_point(it: &CborValue, type_: u8, flags: u32) -> c_int {
    let mut half_bits: u16 = 0x7c01; // upstream's dummy: an infinity
    let val: f64 = match type_ {
        TYPE_DOUBLE => f64::from_bits(parser::decode_int64_internal(it)),
        TYPE_FLOAT => f32::from_bits(parser::decode_int64_internal(it) as u32) as f64,
        _ => {
            half_bits = it.extra;
            cbor_core::half::decode(half_bits) as f64
        }
    };

    if val.is_nan() || val.is_infinite() {
        if flags & FINITE_FLOATING_POINT != 0 {
            return ERR_EXCLUDED_VALUE;
        }
        if flags & SHORTEST_FLOATING_POINT != 0 {
            // A non-finite value always fits in a half, so anything wider is
            // overlong, and the half itself must use the canonical bit pattern.
            if type_ == TYPE_DOUBLE || type_ == TYPE_FLOAT {
                return ERR_OVERLONG_ENCODING;
            }
            if val.is_nan() && half_bits != 0x7e00 {
                return ERR_IMPROPER_VALUE;
            }
            if val.is_infinite() && half_bits != 0x7c00 && half_bits != 0xfc00 {
                return ERR_IMPROPER_VALUE;
            }
        }
    }

    if flags & SHORTEST_FLOATING_POINT != 0 && type_ > TYPE_HALF_FLOAT {
        if type_ == TYPE_DOUBLE && (val as f32) as f64 == val {
            return ERR_OVERLONG_ENCODING;
        }
        if type_ == TYPE_FLOAT {
            let f = val as f32;
            if f == cbor_core::half::decode(cbor_core::half::encode(f)) {
                return ERR_OVERLONG_ENCODING;
            }
        }
    }
    NO_ERROR
}

fn validate_tag(it: &mut CborValue, tag: u64, flags: u32, recursion_left: i32) -> c_int {
    if recursion_left == 0 {
        return ERR_NESTING_TOO_DEEP;
    }
    if flags & NO_TAGS != 0 {
        return ERR_EXCLUDED_TYPE;
    }
    let known = KNOWN_TAGS.iter().find(|k| k.tag == tag);

    if flags & NO_UNKNOWN_TAGS != 0 && known.is_none() {
        // Three tiers: tags below 24 are the most reserved, then below 256,
        // then everything. Each tier is a superset of the previous one.
        if flags & NO_UNKNOWN_TAGS_SA != 0 && tag < 24 {
            return ERR_UNKNOWN_TAG;
        }
        if flags & NO_UNKNOWN_TAGS_SR == NO_UNKNOWN_TAGS_SR && tag < 256 {
            return ERR_UNKNOWN_TAG;
        }
        if flags & NO_UNKNOWN_TAGS == NO_UNKNOWN_TAGS {
            return ERR_UNKNOWN_TAG;
        }
    }

    if flags & TAG_USE != 0 {
        if let Some(k) = known {
            if k.types != 0 {
                // Integer is type 0, which collides with the "any" sentinel, so
                // it is stored and compared as 1.
                let mut ty = it.type_;
                if ty == TYPE_INTEGER {
                    ty += 1;
                }
                let mut allowed = k.types;
                while allowed != 0 {
                    if (allowed & 0xff) as u8 == ty {
                        break;
                    }
                    allowed >>= 8;
                }
                if allowed == 0 {
                    return ERR_INAPPROPRIATE_TAG_FOR_TYPE;
                }
            }
        }
    }

    validate_value(it, flags, recursion_left)
}

fn validate_container(it: &mut CborValue, container: u8, flags: u32, recursion_left: i32) -> c_int {
    if recursion_left == 0 {
        return ERR_NESTING_TOO_DEEP;
    }
    // Map key ordering is compared on the raw encoded bytes, so each key's
    // extent in the buffer is remembered rather than its decoded value.
    let mut previous: Option<(*const u8, *const u8)> = None;

    while it.type_ != TYPE_INVALID {
        let current = it.source.0 as *const u8;

        if container == TYPE_MAP && flags & MAP_KEYS_ARE_STRING != 0 {
            let mut ty = it.type_;
            if ty == TYPE_TAG {
                let mut copy = parser::clone(it);
                let err = crate::parser::cbor_value_skip_tag(&mut copy);
                if err != NO_ERROR {
                    return err;
                }
                ty = copy.type_;
            }
            if ty != TYPE_TEXT_STRING {
                return ERR_MAP_KEY_NOT_STRING;
            }
        }

        let err = validate_value(it, flags, recursion_left);
        if err != NO_ERROR {
            return err;
        }
        if container != TYPE_MAP {
            continue;
        }

        if flags & MAP_IS_SORTED != 0 {
            let key_end = it.source.0 as *const u8;
            if let Some((prev, prev_end)) = previous {
                // SAFETY: both ranges are keys already walked inside this
                // buffer, so both are in bounds and non-overlapping with us.
                let (a, b) = unsafe {
                    (
                        core::slice::from_raw_parts(prev, prev_end as usize - prev as usize),
                        core::slice::from_raw_parts(current, key_end as usize - current as usize),
                    )
                };
                // Canonical ordering compares the shared prefix first, then
                // falls back to length — not plain lexicographic byte order.
                let shared = a.len().min(b.len());
                let mut ord = a[..shared].cmp(&b[..shared]);
                if ord == core::cmp::Ordering::Equal {
                    ord = a.len().cmp(&b.len());
                }
                if ord == core::cmp::Ordering::Greater {
                    return ERR_MAP_NOT_SORTED;
                }
                if ord == core::cmp::Ordering::Equal
                    && flags & MAP_KEYS_ARE_UNIQUE == MAP_KEYS_ARE_UNIQUE
                {
                    return ERR_MAP_KEYS_NOT_UNIQUE;
                }
            }
            previous = Some((current, key_end));
        }

        // That was the key; now the value.
        let err = validate_value(it, flags, recursion_left);
        if err != NO_ERROR {
            return err;
        }
    }
    NO_ERROR
}

fn validate_value(it: &mut CborValue, flags: u32, recursion_left: i32) -> c_int {
    let type_ = it.type_;

    if length_known(it) {
        let err = validate_number(it, type_, flags);
        if err != NO_ERROR {
            return err;
        }
    } else if flags & NO_INDETERMINATE_LENGTH != 0 {
        return ERR_UNKNOWN_LENGTH;
    }

    match type_ {
        TYPE_ARRAY | TYPE_MAP => {
            let mut recursed = parser::clone(it);
            let err = crate::parser::cbor_value_enter_container(it, &mut recursed);
            if err != NO_ERROR {
                return err;
            }
            let err = validate_container(&mut recursed, type_, flags, recursion_left - 1);
            if err != NO_ERROR {
                it.source = recursed.source;
                return err;
            }
            return crate::parser::cbor_value_leave_container(it, &recursed);
        }

        TYPE_BYTE_STRING | TYPE_TEXT_STRING => {
            crate::parser::_cbor_value_begin_string_iteration(it);
            loop {
                let mut ptr: *const c_void = core::ptr::null();
                let mut n: usize = 0;
                let mut next = parser::clone(it);
                let err =
                    crate::parser::_cbor_value_get_string_chunk(it, &mut ptr, &mut n, &mut next);
                if err == NO_ERROR {
                    let e = validate_number(it, type_, flags);
                    if e != NO_ERROR {
                        return e;
                    }
                }
                *it = next;
                if err == parser::ERR_NO_MORE_STRING_CHUNKS {
                    return crate::parser::_cbor_value_finish_string_iteration(it);
                }
                if err != NO_ERROR {
                    return err;
                }
                if type_ == TYPE_TEXT_STRING && flags & UTF8 != 0 {
                    // SAFETY: the chunk API just returned this range.
                    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, n) };
                    if core::str::from_utf8(bytes).is_err() {
                        return ERR_INVALID_UTF8;
                    }
                }
            }
        }

        TYPE_TAG => {
            let tag = parser::extract_int64(it);
            let err = crate::parser::cbor_value_advance_fixed(it);
            if err != NO_ERROR {
                return err;
            }
            return validate_tag(it, tag, flags, recursion_left - 1);
        }

        TYPE_SIMPLE => {
            let err = validate_simple_type(it.extra as u8, flags);
            if err != NO_ERROR {
                return err;
            }
        }

        TYPE_INTEGER | TYPE_NULL | TYPE_BOOLEAN => {}

        TYPE_UNDEFINED => {
            if flags & NO_UNDEFINED != 0 {
                return ERR_EXCLUDED_TYPE;
            }
        }

        TYPE_HALF_FLOAT | TYPE_FLOAT | TYPE_DOUBLE => {
            let err = validate_floating_point(it, type_, flags);
            if err != NO_ERROR {
                return err;
            }
        }

        TYPE_INVALID => return ERR_UNKNOWN_TYPE,
        _ => return ERR_UNKNOWN_TYPE,
    }

    crate::parser::cbor_value_advance_fixed(it)
}

#[no_mangle]
pub extern "C" fn cbor_value_validate(it: *const CborValue, flags: u32) -> c_int {
    // SAFETY: module contract. Walking a copy leaves the caller's cursor alone.
    let mut value = unsafe { parser::clone(&*it) };
    let err = validate_value(&mut value, flags, MAX_RECURSIONS);
    if err != NO_ERROR {
        return err;
    }
    if flags & COMPLETE_DATA != 0 && parser::read_byte(&value).is_some() {
        return ERR_GARBAGE_AT_END;
    }
    NO_ERROR
}
