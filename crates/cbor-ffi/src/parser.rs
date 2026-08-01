//! The parser half of the C ABI.
//!
//! `CborValue` is a StAX-style cursor: it holds a pointer into the buffer, the
//! partially-decoded head of the item it points at, and how many items are left
//! in the enclosing container. `preparse` fills those fields in; everything else
//! is navigation.
//!
//! **SAFETY (whole module):** every `CborValue`/`CborParser` pointer must point
//! to an initialised struct the caller keeps alive for the call, and the buffer
//! a `CborParser` was initialised with must stay valid and unmodified for as
//! long as any `CborValue` refers to it. That is upstream's contract too; a
//! `CborValue` is a borrowed cursor with no lifetime to enforce it.

use crate::types::{CborParser, CborValue};
use core::ffi::{c_char, c_int, c_void};

// Head byte decomposition (RFC 8949 §3).
pub(crate) const MAJOR_TYPE_MASK: u8 = 0xe0;
pub(crate) const SMALL_VALUE_MASK: u8 = 0x1f;
const MAJOR_TYPE_SHIFT: u8 = 5;
pub(crate) const VALUE_8BIT: u8 = 24;
pub(crate) const VALUE_64BIT: u8 = 27;
pub(crate) const INDEFINITE_LENGTH: u8 = 31;
const BREAK_BYTE: u8 = 0xff;

// CborType, which is the major type in the top bits except for major 7, where
// the whole head byte is the type.
pub(crate) const TYPE_BYTE_STRING: u8 = 0x40;
pub(crate) const TYPE_TEXT_STRING: u8 = 0x60;
pub(crate) const TYPE_ARRAY: u8 = 0x80;
pub(crate) const TYPE_MAP: u8 = 0xa0;
pub(crate) const TYPE_TAG: u8 = 0xc0;
pub(crate) const TYPE_SIMPLE: u8 = 0xe0;
pub(crate) const TYPE_INTEGER: u8 = 0x00;
pub(crate) const TYPE_BOOLEAN: u8 = 0xf5;
pub(crate) const TYPE_INVALID: u8 = 0xff;

// Major type numbers, after shifting.
const MAJOR_NEGATIVE: u8 = 1;
const MAJOR_SIMPLE: u8 = 7;

// Simple-value slots inside major type 7.
const FALSE_VALUE: u8 = 20;
const SIMPLE_TYPE_IN_NEXT_BYTE: u8 = 24;
const SINGLE_PRECISION_FLOAT: u8 = 26;
const DOUBLE_PRECISION_FLOAT: u8 = 27;

// CborIteratorFlags.
const F_IS_64BIT: u8 = 0x01;
const F_TOO_LARGE: u8 = 0x02;
pub(crate) const F_NEGATIVE: u8 = 0x04;
/// Shares 0x04 with F_NEGATIVE: one is only meaningful on integers, the other
/// only during string-chunk iteration, so upstream reuses the bit.
const F_BEFORE_FIRST_CHUNK: u8 = 0x04;
const F_ITERATING_CHUNKS: u8 = 0x08;
pub(crate) const F_UNKNOWN_LENGTH: u8 = 0x10;
const F_CONTAINER_IS_MAP: u8 = 0x20;
const F_NEXT_IS_MAP_KEY: u8 = 0x40;

pub(crate) const NO_ERROR: c_int = 0;
const ERR_ADVANCE_PAST_EOF: c_int = 3;
pub(crate) const ERR_UNEXPECTED_EOF: c_int = 257;
const ERR_UNEXPECTED_BREAK: c_int = 258;
const ERR_UNKNOWN_TYPE: c_int = 259;
const ERR_ILLEGAL_NUMBER: c_int = 261;
const ERR_ILLEGAL_SIMPLE_TYPE: c_int = 262;
const ERR_DATA_TOO_LARGE: c_int = 1024;
pub(crate) const ERR_NESTING_TOO_DEEP: c_int = 1025;
pub(crate) const ERR_NO_MORE_STRING_CHUNKS: c_int = 263;

/// Upstream's `CBOR_PARSER_MAX_RECURSIONS`.
const MAX_RECURSIONS: i32 = 1024;

/// SAFETY: see the module contract.
unsafe fn as_mut<'a>(p: *mut CborValue) -> &'a mut CborValue {
    &mut *p
}

/// SAFETY: see the module contract.
unsafe fn as_ref<'a>(p: *const CborValue) -> &'a CborValue {
    &*p
}

/// The callback table a caller supplies to `cbor_parser_init_reader`.
///
/// With an external source there is no buffer to point into: the cursor is an
/// opaque token and every read goes through these.
#[repr(C)]
pub struct CborParserOperations {
    pub can_read_bytes: extern "C" fn(*mut c_void, usize) -> bool,
    pub read_bytes: extern "C" fn(*mut c_void, *mut c_void, usize, usize) -> *mut c_void,
    pub advance_bytes: extern "C" fn(*mut c_void, usize),
    pub transfer_string: extern "C" fn(*mut c_void, *mut *const c_void, usize, usize) -> c_int,
}

/// `CborParserFlag_ExternalSource`.
const EXTERNAL_SOURCE: u32 = 0x01;

#[inline(always)]
fn external(it: &CborValue) -> bool {
    // SAFETY: module contract keeps `parser` alive.
    unsafe { (*it.parser).flags & EXTERNAL_SOURCE != 0 }
}

/// The operations table. Only meaningful when [`external`] is true, where the
/// parser's `source` word holds the table instead of a buffer end.
///
/// SAFETY: whoever called `cbor_parser_init_reader` promised this table
/// outlives the parser.
#[inline(always)]
fn ops(it: &CborValue) -> &CborParserOperations {
    unsafe { &*((*it.parser).source.0 as *const CborParserOperations) }
}

/// One-past-the-end of the buffer. Buffer sources only.
///
/// SAFETY: the module contract keeps `parser` alive and pointing at the parser
/// that was initialised with this buffer.
#[inline(always)]
fn end(it: &CborValue) -> *const u8 {
    unsafe { (*it.parser).source.0 as *const u8 }
}

#[inline(always)]
fn ptr(it: &CborValue) -> *const u8 {
    it.source.0 as *const u8
}

/// Where the bytes come from.
///
/// Upstream tests `parser->flags & ExternalSource` inside each of these four
/// operations, so the test runs on the head of every item. GCC at `-O3` clones
/// the callers and folds the branch away — that is `-fipa-cp-clone`, and
/// switching it off costs upstream 17%. rustc has no equivalent: it will not
/// specialise a function on a runtime flag, so the branch survives, and with it
/// a call the optimiser has to assume clobbers the cursor.
///
/// Making the source a type rather than a flag does the same job at the
/// language level. Each entry point picks an instantiation once, and the buffer
/// one contains no branch and no indirect call anywhere in it. The flag still
/// lives where C expects it — the ABI is unchanged; it is just read once per
/// call instead of once per read.
trait Source {
    fn can_read(it: &CborValue, len: usize) -> bool;

    /// Reads `len` big-endian bytes at `offset` from the cursor.
    ///
    /// SAFETY: callers must have established `can_read(it, offset + len)`.
    unsafe fn read(it: &CborValue, offset: usize, len: usize) -> u64;

    fn advance(it: &mut CborValue, n: usize);

    /// Points `out` at `len` bytes of string content starting `offset` ahead,
    /// and steps the cursor past them. An external source may materialise the
    /// bytes itself, which is why it gets to answer this rather than us
    /// computing it.
    fn transfer_string(
        it: &mut CborValue,
        out: &mut *const c_void,
        offset: usize,
        len: usize,
    ) -> c_int;

    #[inline(always)]
    fn read_byte(it: &CborValue) -> Option<u8> {
        if !Self::can_read(it, 1) {
            return None;
        }
        // SAFETY: just bounds-checked.
        Some(unsafe { Self::read(it, 0, 1) } as u8)
    }

    /// The value carried in the head, reading past `extra` when it did not fit.
    #[inline(always)]
    fn int64(it: &CborValue) -> u64 {
        if it.flags & F_TOO_LARGE != 0 {
            Self::wide_int64(it)
        } else {
            it.extra as u64
        }
    }

    #[inline(always)]
    fn wide_int64(it: &CborValue) -> u64 {
        // SAFETY: preparse only sets these flags after checking the bytes are
        // there.
        unsafe {
            if it.flags & F_IS_64BIT != 0 {
                Self::read(it, 1, 8)
            } else {
                Self::read(it, 1, 4)
            }
        }
    }

    /// Decodes the number in the head byte at the cursor, reading it fresh from
    /// the source rather than from `extra`.
    ///
    /// The cached fields describe whatever was last preparsed. During
    /// string-chunk iteration the cursor sits on a chunk head that was never
    /// preparsed, so the cached value belongs to the enclosing string and is the
    /// wrong answer.
    fn number_at_cursor(it: &CborValue) -> Option<u64> {
        let head = Self::read_byte(it)?;
        let descriptor = head & SMALL_VALUE_MASK;
        if descriptor < VALUE_8BIT {
            return Some(descriptor as u64);
        }
        if descriptor > VALUE_64BIT {
            return None;
        }
        let need = bytes_needed(descriptor);
        if !Self::can_read(it, 1 + need) {
            return None;
        }
        // SAFETY: bounds checked immediately above.
        Some(unsafe { Self::read(it, 1, need) })
    }
}

/// A flat buffer: `CborValue::source` is the cursor and `CborParser::source` is
/// one past the end.
struct Buffer;

/// A caller-supplied reader: `CborValue::source` is an opaque token and
/// `CborParser::source` is the operations table.
struct Reader;

impl Source for Buffer {
    #[inline(always)]
    fn can_read(it: &CborValue, len: usize) -> bool {
        (end(it) as usize).saturating_sub(ptr(it) as usize) >= len
    }

    #[inline(always)]
    unsafe fn read(it: &CborValue, offset: usize, len: usize) -> u64 {
        be_load(ptr(it).add(offset), len)
    }

    #[inline(always)]
    fn advance(it: &mut CborValue, n: usize) {
        it.source.0 = (it.source.0 as usize).wrapping_add(n) as *mut c_void;
    }

    fn transfer_string(
        it: &mut CborValue,
        out: &mut *const c_void,
        offset: usize,
        len: usize,
    ) -> c_int {
        Self::advance(it, offset);
        if !Self::can_read(it, len) {
            return ERR_UNEXPECTED_EOF;
        }
        *out = ptr(it) as *const c_void;
        Self::advance(it, len);
        NO_ERROR
    }
}

impl Source for Reader {
    fn can_read(it: &CborValue, len: usize) -> bool {
        (ops(it).can_read_bytes)(it.source.0, len)
    }

    unsafe fn read(it: &CborValue, offset: usize, len: usize) -> u64 {
        let mut buf = [0u8; 8];
        // The callback may fill our buffer or hand back its own pointer, so use
        // whatever it returns rather than assuming it wrote into `buf`.
        let src = (ops(it).read_bytes)(it.source.0, buf.as_mut_ptr() as *mut c_void, offset, len)
            as *const u8;
        be_load(src, len)
    }

    fn advance(it: &mut CborValue, n: usize) {
        (ops(it).advance_bytes)(it.source.0, n);
    }

    fn transfer_string(
        it: &mut CborValue,
        out: &mut *const c_void,
        offset: usize,
        len: usize,
    ) -> c_int {
        (ops(it).transfer_string)(it.source.0, out, offset, len)
    }
}

/// Runs a source-generic function against the source this parser was
/// initialised with.
///
/// `dispatch!(it, f(a, b))` becomes `f::<Buffer>(a, b)` or `f::<Reader>(a, b)`;
/// the `S::` form does the same for a trait method. Every entry point below
/// goes through this exactly once, and nothing underneath reads the flag again.
macro_rules! dispatch {
    ($it:expr, S::$m:ident($($arg:expr),* $(,)?)) => {
        if external($it) {
            <Reader as Source>::$m($($arg),*)
        } else {
            <Buffer as Source>::$m($($arg),*)
        }
    };
    ($it:expr, $f:ident($($arg:expr),* $(,)?)) => {
        if external($it) {
            $f::<Reader>($($arg),*)
        } else {
            $f::<Buffer>($($arg),*)
        }
    };
}

/// Big-endian load of 1, 2, 4 or 8 bytes.
///
/// CBOR head lengths are only ever those four widths, so this is a sized
/// unaligned load plus a byte swap — one instruction each on x86-64. The
/// byte-at-a-time loop it replaces was a measurable share of parse time,
/// because it runs for the head of every single item.
///
/// SAFETY: `src` must be readable for `len` bytes.
#[inline(always)]
unsafe fn be_load(src: *const u8, len: usize) -> u64 {
    match len {
        1 => *src as u64,
        2 => u16::from_be_bytes(*(src as *const [u8; 2])) as u64,
        4 => u32::from_be_bytes(*(src as *const [u8; 4])) as u64,
        8 => u64::from_be_bytes(*(src as *const [u8; 8])),
        // Not reachable: bytes_needed only ever yields 0, 1, 2, 4 or 8, and a
        // zero-length read is never requested.
        _ => {
            let mut v = 0u64;
            for i in 0..len {
                v = (v << 8) | *src.add(i) as u64;
            }
            v
        }
    }
}

// The four reads other modules need. They are off the parse hot path -- pretty
// printing and JSON conversion call them once per item, around far more
// expensive formatting -- so they take the dispatch rather than making every
// caller generic too.

pub(crate) fn read_byte(it: &CborValue) -> Option<u8> {
    dispatch!(it, S::read_byte(it))
}

pub(crate) fn number_at_cursor(it: &CborValue) -> Option<u64> {
    dispatch!(it, S::number_at_cursor(it))
}

pub(crate) fn extract_int64(it: &CborValue) -> u64 {
    dispatch!(it, S::int64(it))
}

pub(crate) fn decode_int64_internal(it: &CborValue) -> u64 {
    dispatch!(it, S::wide_int64(it))
}

/// How many extra length bytes follow a head with this additional-info value.
pub(crate) fn bytes_needed(descriptor: u8) -> usize {
    if descriptor < VALUE_8BIT {
        0
    } else {
        1 << (descriptor - VALUE_8BIT)
    }
}

fn is_fixed_type(t: u8) -> bool {
    !matches!(
        t,
        TYPE_TEXT_STRING | TYPE_BYTE_STRING | TYPE_ARRAY | TYPE_MAP
    )
}

pub(crate) fn is_container(t: u8) -> bool {
    t == TYPE_ARRAY || t == TYPE_MAP
}

/// Reads the current item's head value and steps over the whole head.
fn extract_number_and_advance<S: Source>(it: &mut CborValue) -> u64 {
    let v = S::int64(it);
    // SAFETY: preparse established that the head byte is readable.
    // Must go through the source, not the buffer pointer: with an external
    // source `it.source` is an opaque token and dereferencing it reads garbage.
    let descriptor = unsafe { S::read(it, 0, 1) } as u8 & SMALL_VALUE_MASK;
    S::advance(it, bytes_needed(descriptor) + 1);
    v
}

/// Decodes the head at the cursor into `type_`, `extra` and `flags`.
///
/// This is the heart of the parser: everything else navigates, this is the only
/// place that interprets a byte.
fn preparse_value<S: Source>(it: &mut CborValue) -> c_int {
    const FLAGS_TO_KEEP: u8 = F_CONTAINER_IS_MAP | F_NEXT_IS_MAP_KEY;

    it.type_ = TYPE_INVALID;
    it.flags &= FLAGS_TO_KEEP;

    let Some(descriptor) = S::read_byte(it) else {
        return ERR_UNEXPECTED_EOF;
    };

    let type_ = descriptor & MAJOR_TYPE_MASK;
    it.type_ = type_;
    let descriptor = descriptor & SMALL_VALUE_MASK;
    it.extra = descriptor as u16;

    if descriptor > VALUE_64BIT {
        if descriptor != INDEFINITE_LENGTH {
            return if type_ == TYPE_SIMPLE {
                ERR_UNKNOWN_TYPE
            } else {
                ERR_ILLEGAL_NUMBER
            };
        }
        // Only strings and containers may be indefinite-length.
        if !is_fixed_type(type_) {
            it.flags |= F_UNKNOWN_LENGTH;
            return NO_ERROR;
        }
        return if type_ == TYPE_SIMPLE {
            ERR_UNEXPECTED_BREAK
        } else {
            ERR_ILLEGAL_NUMBER
        };
    }

    let need = bytes_needed(descriptor);
    if need != 0 {
        if !S::can_read(it, need + 1) {
            return ERR_UNEXPECTED_EOF;
        }
        it.extra = 0;
        // Up to 16 bits fit in `extra`; wider values stay in the buffer and get
        // decoded on demand, which is how the parser avoids storing a u64.
        match need {
            1 => it.extra = unsafe { S::read(it, 1, 1) } as u16,
            2 => it.extra = unsafe { S::read(it, 1, 2) } as u16,
            _ => it.flags |= descriptor & 3,
        }
    }

    let majortype = type_ >> MAJOR_TYPE_SHIFT;
    if majortype == MAJOR_NEGATIVE {
        it.flags |= F_NEGATIVE;
        it.type_ = TYPE_INTEGER;
    } else if majortype == MAJOR_SIMPLE {
        match descriptor {
            FALSE_VALUE => {
                it.extra = 0;
                it.type_ = TYPE_BOOLEAN;
            }
            SINGLE_PRECISION_FLOAT | DOUBLE_PRECISION_FLOAT => {
                it.flags |= F_TOO_LARGE;
                // SAFETY: the head byte was read above.
                it.type_ = unsafe { S::read(it, 0, 1) } as u8;
            }
            21..=23 | 25 => {
                // true, null, undefined, half-float: the head byte is the type.
                // SAFETY: the head byte was read above.
                it.type_ = unsafe { S::read(it, 0, 1) } as u8;
            }
            // A simple value below 32 must use the one-byte form; spelling it
            // in two bytes is an overlong encoding.
            SIMPLE_TYPE_IN_NEXT_BYTE if it.extra < 32 => {
                it.type_ = TYPE_INVALID;
                return ERR_ILLEGAL_SIMPLE_TYPE;
            }
            _ => {}
        }
    }

    NO_ERROR
}

/// Like `preparse_value`, but recognises the break byte that ends an
/// indefinite-length container.
fn preparse_next_value_nodecrement<S: Source>(it: &mut CborValue) -> c_int {
    if it.remaining == u32::MAX && S::read_byte(it) == Some(BREAK_BYTE) {
        // A map that has just read a key, or a dangling tag, cannot end here.
        let mid_pair = it.flags & F_CONTAINER_IS_MAP != 0 && it.flags & F_NEXT_IS_MAP_KEY != 0;
        if mid_pair || it.type_ == TYPE_TAG {
            return ERR_UNEXPECTED_BREAK;
        }
        it.type_ = TYPE_INVALID;
        it.remaining = 0;
        it.flags |= F_UNKNOWN_LENGTH; // leave_container consumes the break
        return NO_ERROR;
    }
    preparse_value::<S>(it)
}

fn preparse_next_value<S: Source>(it: &mut CborValue) -> c_int {
    // A tag is a prefix on the next item, so it does not consume a slot in the
    // enclosing container and does not flip the map key/value phase.
    let item_counts = it.type_ != TYPE_TAG;

    if it.remaining != u32::MAX && item_counts {
        it.remaining -= 1;
        if it.remaining == 0 {
            it.type_ = TYPE_INVALID;
            it.flags &= !F_UNKNOWN_LENGTH; // no break to consume
            return NO_ERROR;
        }
    }
    if item_counts {
        it.flags ^= F_NEXT_IS_MAP_KEY;
    }
    preparse_next_value_nodecrement::<S>(it)
}

fn advance_internal<S: Source>(it: &mut CborValue) -> c_int {
    let length = extract_number_and_advance::<S>(it);
    if it.type_ == TYPE_BYTE_STRING || it.type_ == TYPE_TEXT_STRING {
        S::advance(it, length as usize);
    }
    preparse_next_value::<S>(it)
}

// -- initialisation --------------------------------------------------------

#[no_mangle]
pub extern "C" fn cbor_parser_init(
    buffer: *const u8,
    size: usize,
    flags: u32,
    parser: *mut CborParser,
    it: *mut CborValue,
) -> c_int {
    // SAFETY: module contract.
    unsafe {
        let p = &mut *parser;
        p.source.0 = buffer.wrapping_add(size) as *mut c_void;
        p.flags = flags;

        let v = as_mut(it);
        v.parser = parser;
        v.source.0 = buffer as *mut c_void;
        v.remaining = 1; // exactly one top-level item
        v.extra = 0;
        v.type_ = TYPE_INVALID;
        v.flags = 0;
        // The caller's `flags` are free to name a source we did not set up here;
        // upstream would parse whatever that implies, so dispatch on what was
        // stored rather than assuming a buffer.
        dispatch!(v, preparse_value(v))
    }
}

/// Parsing from a caller-supplied source rather than a flat buffer. The
/// `CborValue::source` word holds the caller's token instead of a pointer, and
/// `CborParser::source` holds the operations table instead of a buffer end.
#[no_mangle]
pub extern "C" fn cbor_parser_init_reader(
    operations: *const c_void,
    parser: *mut CborParser,
    it: *mut CborValue,
    token: *mut c_void,
) -> c_int {
    // SAFETY: module contract.
    unsafe {
        let p = &mut *parser;
        p.source.0 = operations as *mut c_void;
        p.flags = EXTERNAL_SOURCE;

        let v = as_mut(it);
        v.parser = parser;
        v.source.0 = token;
        v.remaining = 1;
        v.extra = 0;
        v.type_ = TYPE_INVALID;
        v.flags = 0;
        // EXTERNAL_SOURCE was just set two lines up, so there is nothing to
        // decide.
        preparse_value::<Reader>(v)
    }
}

// -- navigation ------------------------------------------------------------

#[no_mangle]
pub extern "C" fn cbor_value_advance_fixed(it: *mut CborValue) -> c_int {
    // SAFETY: module contract.
    let v = unsafe { as_mut(it) };
    if v.remaining == 0 {
        return ERR_ADVANCE_PAST_EOF;
    }
    dispatch!(v, advance_internal(v))
}

fn advance_recursive<S: Source>(it: &mut CborValue, nesting: i32) -> c_int {
    if is_fixed_type(it.type_) {
        return advance_internal::<S>(it);
    }
    if !is_container(it.type_) {
        // A string: skip its bytes, chunked or not. C passes `it` as both the
        // value to read and the `next` to write, which Rust will not allow as
        // one borrow. Reading from a snapshot makes the aliasing go away
        // instead of papering over it with a raw pointer.
        let out: *mut CborValue = it;
        let mut len = usize::MAX;
        let mut all = false;
        return iterate_string_chunks::<S>(
            it,
            core::ptr::null_mut(),
            &mut len,
            &mut all,
            Some(out),
            Iterate::Noop,
        );
    }
    if nesting == 0 {
        return ERR_NESTING_TOO_DEEP;
    }

    let mut recursed = CborValue {
        parser: it.parser,
        source: it.source,
        remaining: 0,
        extra: 0,
        type_: 0,
        flags: 0,
    };
    let err = enter_container::<S>(it, &mut recursed);
    if err != NO_ERROR {
        return err;
    }
    while recursed.type_ != TYPE_INVALID {
        let err = advance_recursive::<S>(&mut recursed, nesting - 1);
        if err != NO_ERROR {
            return err;
        }
    }
    leave_container::<S>(it, &recursed)
}

#[no_mangle]
pub extern "C" fn cbor_value_advance(it: *mut CborValue) -> c_int {
    // SAFETY: module contract.
    let v = unsafe { as_mut(it) };
    dispatch!(v, advance(v))
}

fn enter_container<S: Source>(it: &CborValue, recursed: &mut CborValue) -> c_int {
    *recursed = CborValue {
        parser: it.parser,
        source: it.source,
        remaining: it.remaining,
        extra: it.extra,
        type_: it.type_,
        flags: it.flags,
    };

    if it.flags & F_UNKNOWN_LENGTH != 0 {
        recursed.remaining = u32::MAX;
        S::advance(recursed, 1);
    } else {
        let len = extract_number_and_advance::<S>(recursed);
        recursed.remaining = len as u32;
        if recursed.remaining as u64 != len || len == u32::MAX as u64 {
            recursed.source = it.source;
            return ERR_DATA_TOO_LARGE;
        }
        if recursed.type_ == TYPE_MAP {
            // Keys and values are separate items.
            if recursed.remaining > u32::MAX / 2 {
                recursed.source = it.source;
                return ERR_DATA_TOO_LARGE;
            }
            recursed.remaining *= 2;
        }
        if len == 0 {
            recursed.type_ = TYPE_INVALID;
            return NO_ERROR;
        }
    }
    recursed.flags = recursed.type_ & F_CONTAINER_IS_MAP;
    preparse_next_value_nodecrement::<S>(recursed)
}

#[no_mangle]
pub extern "C" fn cbor_value_enter_container(
    it: *const CborValue,
    recursed: *mut CborValue,
) -> c_int {
    // SAFETY: module contract, for two distinct values.
    unsafe {
        let v = as_ref(it);
        dispatch!(v, enter_container(v, as_mut(recursed)))
    }
}

fn leave_container<S: Source>(it: &mut CborValue, recursed: &CborValue) -> c_int {
    it.source = recursed.source;
    if recursed.flags & F_UNKNOWN_LENGTH != 0 {
        S::advance(it, 1); // consume the break
    }
    preparse_next_value::<S>(it)
}

#[no_mangle]
pub extern "C" fn cbor_value_leave_container(
    it: *mut CborValue,
    recursed: *const CborValue,
) -> c_int {
    // SAFETY: module contract, for two distinct values.
    unsafe {
        let v = as_mut(it);
        dispatch!(v, leave_container(v, as_ref(recursed)))
    }
}

#[no_mangle]
pub extern "C" fn cbor_value_reparse(it: *mut CborValue) -> c_int {
    // SAFETY: module contract.
    let v = unsafe { as_mut(it) };
    dispatch!(v, preparse_value(v))
}

#[no_mangle]
pub extern "C" fn cbor_value_validate_basic(it: *const CborValue) -> c_int {
    // SAFETY: module contract; advancing a copy leaves the caller's value alone.
    let mut copy = unsafe { clone(as_ref(it)) };
    cbor_value_advance(&mut copy)
}

pub(crate) fn clone(it: &CborValue) -> CborValue {
    CborValue {
        parser: it.parser,
        source: it.source,
        remaining: it.remaining,
        extra: it.extra,
        type_: it.type_,
        flags: it.flags,
    }
}

#[no_mangle]
pub extern "C" fn cbor_value_skip_tag(it: *mut CborValue) -> c_int {
    // SAFETY: module contract.
    let v = unsafe { as_mut(it) };
    dispatch!(v, skip_tag(v))
}

fn skip_tag<S: Source>(it: &mut CborValue) -> c_int {
    while it.type_ == TYPE_TAG {
        if it.remaining == 0 {
            return ERR_ADVANCE_PAST_EOF;
        }
        let err = advance_internal::<S>(it);
        if err != NO_ERROR {
            return err;
        }
    }
    NO_ERROR
}

// -- scalar extraction -----------------------------------------------------

#[no_mangle]
pub extern "C" fn _cbor_value_decode_int64_internal(value: *const CborValue) -> u64 {
    // SAFETY: module contract.
    decode_int64_internal(unsafe { as_ref(value) })
}

#[no_mangle]
pub extern "C" fn cbor_value_get_int64_checked(value: *const CborValue, result: *mut i64) -> c_int {
    // SAFETY: module contract, plus a writable `result`.
    unsafe {
        let v = as_ref(value);
        let raw = extract_int64(v);
        // A negative integer n encodes -1-raw, so the representable range is
        // one larger on the negative side. Rejecting raw > i64::MAX for
        // positives and raw >= 2^63 for negatives catches exactly the overflow.
        if v.flags & F_NEGATIVE != 0 {
            if raw > i64::MAX as u64 {
                return ERR_DATA_TOO_LARGE;
            }
            *result = -1 - raw as i64;
        } else {
            if raw > i64::MAX as u64 {
                return ERR_DATA_TOO_LARGE;
            }
            *result = raw as i64;
        }
        NO_ERROR
    }
}

#[no_mangle]
pub extern "C" fn cbor_value_get_int_checked(value: *const CborValue, result: *mut c_int) -> c_int {
    let mut wide: i64 = 0;
    let err = cbor_value_get_int64_checked(value, &mut wide);
    if err != NO_ERROR {
        return err;
    }
    if wide < c_int::MIN as i64 || wide > c_int::MAX as i64 {
        return ERR_DATA_TOO_LARGE;
    }
    // SAFETY: caller-provided out-parameter, per the module contract.
    unsafe { *result = wide as c_int };
    NO_ERROR
}

#[no_mangle]
pub extern "C" fn cbor_value_get_half_float_as_float(
    value: *const CborValue,
    result: *mut f32,
) -> c_int {
    // SAFETY: module contract, plus a writable `result`.
    unsafe {
        let v = as_ref(value);
        let bits = dispatch!(v, S::read(v, 1, 2)) as u16;
        *result = cbor_core::half::decode(bits);
        NO_ERROR
    }
}

// -- string access --------------------------------------------------------
//
// Upstream funnels copying, measuring and comparing through one primitive,
// `iterate_string_chunks`, differing only in what it does with each chunk.
// Keeping that shape matters: the NUL-termination rule and the "did it all
// fit" flag are subtle, and three separate implementations would drift.

/// What to do with each chunk as it is walked.
#[derive(Clone, Copy, PartialEq)]
enum Iterate {
    /// Measure only; `buffer` is ignored.
    Noop,
    /// Copy into `buffer` at the running offset.
    Copy,
    /// Compare against `buffer` at the running offset.
    Compare,
}

/// Walks a string, definite or chunked, applying `op` to every chunk.
///
/// `buflen` is in/out: capacity in, true length out — never counting the NUL.
/// `result` reports whether every chunk was processed, which for `Copy` means
/// it all fit and for `Compare` means it all matched.
///
/// A NUL is written only when there is room *beyond* the content (`buflen >
/// total`, strictly). For `Compare` that NUL check is what stops a prefix from
/// comparing equal to a longer expected string.
fn iterate_string_chunks<S: Source>(
    value: &CborValue,
    buffer: *mut u8,
    buflen: &mut usize,
    result: &mut bool,
    next: Option<*mut CborValue>,
    op: Iterate,
) -> c_int {
    let mut cursor = clone(value);
    *result = true;
    let mut total: usize = 0;

    let err = begin_string_iteration::<S>(&mut cursor);
    if err != NO_ERROR {
        return err;
    }

    loop {
        let mut ptr: *const c_void = core::ptr::null();
        let mut chunk_len: usize = 0;
        let err = get_chunk::<S>(&mut cursor, &mut ptr, &mut chunk_len);
        if err == ERR_NO_MORE_STRING_CHUNKS {
            break;
        }
        if err != NO_ERROR {
            return err;
        }

        let Some(new_total) = total.checked_add(chunk_len) else {
            return ERR_DATA_TOO_LARGE;
        };

        if *result && *buflen >= new_total {
            // SAFETY: the chunk API returned this range, and the bounds check
            // above proved the destination has room for it.
            *result = unsafe { apply(op, buffer, total, ptr as *const u8, chunk_len) };
        } else {
            *result = false;
        }
        total = new_total;
    }

    if *result && *buflen > total {
        // SAFETY: `buflen > total` leaves at least one byte spare.
        *result = unsafe { apply(op, buffer, total, [0u8].as_ptr(), 1) };
    }
    *buflen = total;

    let err = finish_string_iteration::<S>(&mut cursor);
    if let Some(n) = next {
        // SAFETY: module contract. `next` frequently aliases `value`, which is
        // why the walk ran on a copy.
        unsafe { *n = cursor };
    }
    err
}

/// SAFETY: `dst + offset .. + len` must be writable for `Copy`/readable for
/// `Compare`, and `src .. src + len` readable. Both are checked by the caller.
unsafe fn apply(op: Iterate, dst: *mut u8, offset: usize, src: *const u8, len: usize) -> bool {
    match op {
        Iterate::Noop => true,
        Iterate::Copy => {
            core::ptr::copy_nonoverlapping(src, dst.add(offset), len);
            true
        }
        Iterate::Compare => {
            core::slice::from_raw_parts(dst.add(offset) as *const u8, len)
                == core::slice::from_raw_parts(src, len)
        }
    }
}

/// One chunk at the cursor, advancing past it. Split out so
/// `iterate_string_chunks` reads the same as upstream's loop.
fn get_chunk<S: Source>(it: &mut CborValue, out: &mut *const c_void, len: &mut usize) -> c_int {
    let (offset, n) = match chunk_size::<S>(it) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let err = S::transfer_string(it, out, offset, n);
    if err != NO_ERROR {
        return err;
    }
    *len = n;
    it.flags &= !F_BEFORE_FIRST_CHUNK;
    NO_ERROR
}

const ERR_ILLEGAL_TYPE: c_int = 260;
const ERR_OUT_OF_MEMORY: c_int = c_int::MIN;

#[no_mangle]
pub extern "C" fn cbor_value_calculate_string_length(
    value: *const CborValue,
    length: *mut usize,
) -> c_int {
    // SAFETY: module contract, plus a writable `length`.
    unsafe {
        *length = usize::MAX;
        _cbor_value_copy_string(value, core::ptr::null_mut(), length, core::ptr::null_mut())
    }
}

#[no_mangle]
pub extern "C" fn _cbor_value_copy_string(
    value: *const CborValue,
    buffer: *mut c_void,
    buflen: *mut usize,
    next: *mut CborValue,
) -> c_int {
    // SAFETY: module contract, plus a writable `buflen`.
    unsafe {
        let mut copied_all = false;
        let op = if buffer.is_null() {
            Iterate::Noop
        } else {
            Iterate::Copy
        };
        let v = as_ref(value);
        let err = dispatch!(
            v,
            iterate_string_chunks(
                v,
                buffer as *mut u8,
                &mut *buflen,
                &mut copied_all,
                if next.is_null() { None } else { Some(next) },
                op,
            )
        );
        if err != NO_ERROR {
            return err;
        }
        if copied_all {
            NO_ERROR
        } else {
            ERR_OUT_OF_MEMORY
        }
    }
}

// The one place this library allocates on the caller's behalf.
//
// The documented contract is that `*buffer` comes back owned by the caller and
// is released with `free()`. That names the allocator, so it has to be libc's:
// handing back a pointer from Rust's allocator would be a heap mismatch the
// moment anyone honoured the contract, and Rust makes no promise that its
// global allocator is malloc even where it happens to be today.
extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
}

#[no_mangle]
pub extern "C" fn _cbor_value_dup_string(
    value: *const CborValue,
    buffer: *mut *mut c_void,
    buflen: *mut usize,
    next: *mut CborValue,
) -> c_int {
    // SAFETY: module contract, plus writable `buffer` and `buflen`, which
    // upstream asserts are non-NULL rather than checking.
    unsafe {
        // Callers routinely pass the same cursor as `value` and `next`, so read
        // it before the first pass writes through `next`.
        let it = clone(as_ref(value));

        // First pass measures. SIZE_MAX capacity means nothing is ever copied
        // and nothing can fail to fit, so this only walks the chunks.
        *buflen = usize::MAX;
        let err = _cbor_value_copy_string(&it, core::ptr::null_mut(), buflen, next);
        if err != NO_ERROR {
            return err;
        }

        // One byte beyond the content, which is what makes the copy pass write
        // the NUL. Never zero, so `malloc(0)` is not a case to think about.
        *buflen += 1;
        let tmp = malloc(*buflen);
        if tmp.is_null() {
            // *buflen keeps the size that would have been needed, which is what
            // the documentation promises for this one error.
            return ERR_OUT_OF_MEMORY;
        }

        let err = _cbor_value_copy_string(&it, tmp, buflen, next);
        if err != NO_ERROR {
            // Cannot happen: the walk above already succeeded over the same
            // bytes. Freeing rather than leaking on the impossible path anyway.
            free(tmp);
            return err;
        }
        *buffer = tmp;
        NO_ERROR
    }
}

/// Compares without copying: the expected string is walked in place as the
/// chunks arrive, so a mismatch costs nothing extra.
#[no_mangle]
pub extern "C" fn cbor_value_text_string_equals(
    value: *const CborValue,
    string: *const c_char,
    result: *mut bool,
) -> c_int {
    // SAFETY: module contract; `string` is a NUL-terminated C string.
    unsafe {
        let v = as_ref(value);
        dispatch!(v, text_string_equals(v, string, &mut *result))
    }
}

/// SAFETY: `string` is a NUL-terminated C string.
unsafe fn text_string_equals<S: Source>(
    value: &CborValue,
    string: *const c_char,
    result: &mut bool,
) -> c_int {
    // A tagged string is still that string as far as comparison goes.
    let mut copy = clone(value);
    let err = skip_tag::<S>(&mut copy);
    if err != NO_ERROR {
        return err;
    }
    if copy.type_ != TYPE_TEXT_STRING {
        *result = false;
        return NO_ERROR;
    }

    let mut buflen = 0usize;
    while *string.add(buflen) != 0 {
        buflen += 1;
    }
    // Capacity is strlen, not strlen+1. The NUL comparison then runs only when
    // the CBOR string is *shorter* than expected, which is exactly the case
    // that has to fail; for equal lengths there is no spare byte and no
    // comparison, and for equal empty strings nothing is compared at all.
    iterate_string_chunks::<S>(
        &copy,
        string as *mut u8,
        &mut buflen,
        result,
        None,
        Iterate::Compare,
    )
}

#[no_mangle]
pub extern "C" fn cbor_value_map_find_value(
    map: *const CborValue,
    string: *const c_char,
    element: *mut CborValue,
) -> c_int {
    // SAFETY: module contract.
    unsafe {
        let m = as_ref(map);
        dispatch!(m, map_find_value(m, string, as_mut(element)))
    }
}

/// SAFETY: `string` is a NUL-terminated C string; `element` is the caller's
/// out-parameter and is distinct from `map`.
unsafe fn map_find_value<S: Source>(
    map: &CborValue,
    string: *const c_char,
    e: &mut CborValue,
) -> c_int {
    let err = enter_container::<S>(map, e);
    if err != NO_ERROR {
        e.type_ = TYPE_INVALID;
        return err;
    }
    let mut len = 0usize;
    while *string.add(len) != 0 {
        len += 1;
    }

    while e.type_ != TYPE_INVALID {
        // Keys may be tagged; the tag is not part of the comparison.
        let err = skip_tag::<S>(e);
        if err != NO_ERROR {
            e.type_ = TYPE_INVALID;
            return err;
        }

        if e.type_ == TYPE_TEXT_STRING {
            let mut equals = false;
            let mut buflen = len;
            let out: *mut CborValue = e;
            let err = iterate_string_chunks::<S>(
                e,
                string as *mut u8,
                &mut buflen,
                &mut equals,
                Some(out),
                Iterate::Compare,
            );
            if err != NO_ERROR {
                e.type_ = TYPE_INVALID;
                return err;
            }
            if equals {
                // The cursor already sits on the value; re-decode its head so
                // the caller gets a usable iterator.
                return preparse_value::<S>(e);
            }
        } else {
            let err = advance::<S>(e);
            if err != NO_ERROR {
                e.type_ = TYPE_INVALID;
                return err;
            }
        }

        // Skip the value this key belonged to.
        let err = skip_tag::<S>(e);
        if err != NO_ERROR {
            e.type_ = TYPE_INVALID;
            return err;
        }
        let err = advance::<S>(e);
        if err != NO_ERROR {
            e.type_ = TYPE_INVALID;
            return err;
        }
    }

    e.type_ = TYPE_INVALID;
    NO_ERROR
}

fn advance<S: Source>(it: &mut CborValue) -> c_int {
    if it.remaining == 0 {
        return ERR_ADVANCE_PAST_EOF;
    }
    advance_recursive::<S>(it, MAX_RECURSIONS)
}

// -- string chunk iteration ------------------------------------------------
//
// A definite-length string is one chunk; an indefinite-length one is a run of
// definite chunks ending in a break. The caller drives both the same way:
// begin, then get_chunk until NoMoreStringChunks, then finish. The state lives
// in two flag bits rather than anywhere else, which is what keeps CborValue at
// 24 bytes.

fn length_known(it: &CborValue) -> bool {
    it.flags & F_UNKNOWN_LENGTH == 0
}

#[no_mangle]
pub extern "C" fn _cbor_value_begin_string_iteration(value: *mut CborValue) -> c_int {
    // SAFETY: module contract.
    let it = unsafe { as_mut(value) };
    dispatch!(it, begin_string_iteration(it))
}

fn begin_string_iteration<S: Source>(it: &mut CborValue) -> c_int {
    it.flags |= F_ITERATING_CHUNKS | F_BEFORE_FIRST_CHUNK;
    if !length_known(it) {
        S::advance(it, 1); // step over the indefinite head onto chunk one
    }
    NO_ERROR
}

#[no_mangle]
pub extern "C" fn _cbor_value_finish_string_iteration(value: *mut CborValue) -> c_int {
    // SAFETY: module contract.
    let it = unsafe { as_mut(value) };
    dispatch!(it, finish_string_iteration(it))
}

fn finish_string_iteration<S: Source>(it: &mut CborValue) -> c_int {
    if !length_known(it) {
        S::advance(it, 1); // consume the break
    }
    preparse_next_value::<S>(it)
}

/// Size of the chunk at the cursor, plus how far past the cursor its bytes
/// start. Returns `NoMoreStringChunks` at the end, which is the loop condition
/// rather than an error.
fn chunk_size<S: Source>(it: &CborValue) -> Result<(usize, usize), c_int> {
    if length_known(it) && it.flags & F_BEFORE_FIRST_CHUNK == 0 {
        return Err(ERR_NO_MORE_STRING_CHUNKS);
    }
    let Some(descriptor) = S::read_byte(it) else {
        return Err(ERR_UNEXPECTED_EOF);
    };
    if descriptor == BREAK_BYTE {
        return Err(ERR_NO_MORE_STRING_CHUNKS);
    }
    if descriptor & MAJOR_TYPE_MASK != it.type_ {
        return Err(ERR_ILLEGAL_TYPE);
    }

    let descriptor = descriptor & SMALL_VALUE_MASK;
    if descriptor < VALUE_8BIT {
        return Ok((1, descriptor as usize));
    }
    if descriptor > VALUE_64BIT {
        return Err(ERR_ILLEGAL_NUMBER);
    }
    let need = bytes_needed(descriptor);
    if !S::can_read(it, 1 + need) {
        return Err(ERR_UNEXPECTED_EOF);
    }
    // SAFETY: bounds checked immediately above.
    let val = unsafe { S::read(it, 1, need) };
    let len = val as usize;
    if len as u64 != val {
        return Err(ERR_DATA_TOO_LARGE);
    }
    Ok((1 + need, len))
}

#[no_mangle]
pub extern "C" fn _cbor_value_get_string_chunk_size(
    value: *const CborValue,
    len: *mut usize,
) -> c_int {
    // SAFETY: module contract, plus a writable `len`.
    unsafe {
        let v = as_ref(value);
        match dispatch!(v, chunk_size(v)) {
            Ok((_, n)) => {
                *len = n;
                NO_ERROR
            }
            Err(e) => e,
        }
    }
}

#[no_mangle]
pub extern "C" fn _cbor_value_get_string_chunk(
    value: *const CborValue,
    bufferptr: *mut *const c_void,
    len: *mut usize,
    next: *mut CborValue,
) -> c_int {
    // SAFETY: module contract. `next` routinely aliases `value` — callers pass
    // the same cursor for both — so the read happens before anything is written.
    unsafe {
        let it = as_ref(value);
        dispatch!(
            it,
            get_string_chunk(it, &mut *bufferptr, &mut *len, as_mut(next))
        )
    }
}

fn get_string_chunk<S: Source>(
    it: &CborValue,
    bufferptr: &mut *const c_void,
    len: &mut usize,
    next: &mut CborValue,
) -> c_int {
    let (offset, n) = match chunk_size::<S>(it) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let mut cursor = clone(it);
    let err = S::transfer_string(&mut cursor, bufferptr, offset, n);
    if err != NO_ERROR {
        return err;
    }
    *len = n;

    *next = cursor;
    next.flags &= !F_BEFORE_FIRST_CHUNK;
    NO_ERROR
}
