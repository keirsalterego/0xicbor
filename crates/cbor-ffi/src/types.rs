//! Rust mirrors of the three public C structs.
//!
//! The Qt suite reads these fields directly through the 59 `static inline`
//! accessors in cbor.h, so these are not an interface we get to design. Every
//! size and offset here is asserted against `abi-layout.txt`, which was dumped
//! from a C program at kickoff.
//!
//! Upstream declares a union in each of `CborEncoder::data`, `CborParser::source`
//! and `CborValue::source`. Every member of all three is exactly pointer-sized
//! and pointer-aligned, so a single opaque word is layout-identical. Using a
//! word instead of a `union` keeps field access out of the unsafe budget: a
//! `union` read is `unsafe` in Rust regardless of whether it can actually
//! misbehave, and that would add noise to the count without adding safety.

use core::ffi::c_void;

/// One pointer-sized slot standing in for a C union of pointer-sized members.
///
/// Which member is live is decided by the owning struct's `flags`, exactly as
/// it is in C. The accessors that interpret it live next to the code that sets
/// the flag.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Word(pub *mut c_void);

/// Mirror of C `struct CborEncoder`. 32 bytes, 8-aligned.
#[repr(C)]
pub struct CborEncoder {
    /// `ptr` while writing to a buffer, `bytes_needed` once the buffer has
    /// overrun, or `writer` when the encoder was built with a write callback.
    pub data: Word,
    pub end: *mut u8,
    pub remaining: usize,
    pub flags: i32,
}

/// Mirror of C `struct CborParser`. 16 bytes, 8-aligned.
#[repr(C)]
pub struct CborParser {
    /// `end` for buffer parsing, `ops` for callback-driven parsing.
    pub source: Word,
    pub flags: u32,
}

/// Mirror of C `struct CborValue`. 24 bytes, 8-aligned.
///
/// This is the self-referential one: `parser` points back at a `CborParser` the
/// caller owns, and nothing in the C API keeps the two lifetimes tied together.
#[repr(C)]
pub struct CborValue {
    pub parser: *const CborParser,
    /// `ptr` for buffer parsing, `token` for callback-driven parsing.
    pub source: Word,
    pub remaining: u32,
    pub extra: u16,
    pub type_: u8,
    pub flags: u8,
}

#[cfg(test)]
mod layout {
    use super::*;
    use core::mem::{align_of, offset_of, size_of};

    // These numbers come from crates/cbor-ffi/abi-layout.txt. If one of these
    // fails, the Qt suite would read garbage through the inline accessors, so
    // this test is the tripwire for the whole ABI-shim approach.
    #[test]
    fn matches_c() {
        assert_eq!(size_of::<CborEncoder>(), 32);
        assert_eq!(align_of::<CborEncoder>(), 8);
        assert_eq!(offset_of!(CborEncoder, data), 0);
        assert_eq!(offset_of!(CborEncoder, end), 8);
        assert_eq!(offset_of!(CborEncoder, remaining), 16);
        assert_eq!(offset_of!(CborEncoder, flags), 24);

        assert_eq!(size_of::<CborParser>(), 16);
        assert_eq!(align_of::<CborParser>(), 8);
        assert_eq!(offset_of!(CborParser, source), 0);
        assert_eq!(offset_of!(CborParser, flags), 8);

        assert_eq!(size_of::<CborValue>(), 24);
        assert_eq!(align_of::<CborValue>(), 8);
        assert_eq!(offset_of!(CborValue, parser), 0);
        assert_eq!(offset_of!(CborValue, source), 8);
        assert_eq!(offset_of!(CborValue, remaining), 16);
        assert_eq!(offset_of!(CborValue, extra), 20);
        assert_eq!(offset_of!(CborValue, type_), 22);
        assert_eq!(offset_of!(CborValue, flags), 23);
    }
}
