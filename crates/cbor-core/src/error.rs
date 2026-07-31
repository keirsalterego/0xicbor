//! The error type. Discriminants are fixed by the C ABI, not chosen by us.
//!
//! Upstream returns `CborError` from nearly every entry point and reserves 0 for
//! success. In Rust that success case is `Ok(())`, so this enum carries only the
//! failures and `CborNoError` has no variant here. The FFI layer maps
//! `Result<T, CborError>` back onto the C convention in one place.
//!
//! The gaps in the numbering are upstream's: errors are grouped in blocks of 256
//! by category (parsing, tagging, counting, resources, JSON) so a caller can
//! range-check a class of failure.

/// A CBOR operation that did not succeed.
///
/// `#[repr(i32)]` because these values cross the FFI boundary as C `int`.
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CborError {
    UnknownError = 1,
    /// Length asked for on an indefinite-length array, map or string.
    UnknownLength = 2,
    AdvancePastEOF = 3,
    IO = 4,

    GarbageAtEnd = 256,
    UnexpectedEOF = 257,
    UnexpectedBreak = 258,
    /// Only reachable in major type 7.
    UnknownType = 259,
    /// The type is valid CBOR but not allowed in this position.
    IllegalType = 260,
    IllegalNumber = 261,
    /// A simple value below 32 encoded in two bytes instead of one.
    IllegalSimpleType = 262,
    NoMoreStringChunks = 263,

    UnknownSimpleType = 512,
    UnknownTag = 513,
    InappropriateTagForType = 514,
    DuplicateObjectKeys = 515,
    InvalidUtf8TextString = 516,
    ExcludedType = 517,
    ExcludedValue = 518,
    ImproperValue = 519,
    OverlongEncoding = 520,
    MapKeyNotString = 521,
    MapNotSorted = 522,
    MapKeysNotUnique = 523,

    TooManyItems = 768,
    TooFewItems = 769,

    DataTooLarge = 1024,
    NestingTooDeep = 1025,
    UnsupportedType = 1026,
    UnimplementedValidation = 1027,

    JsonObjectKeyIsAggregate = 1280,
    JsonObjectKeyNotString = 1281,
    JsonNotImplemented = 1282,

    OutOfMemory = i32::MIN,
    InternalError = i32::MAX,
}

/// What every fallible operation in this crate returns.
pub type CborResult<T> = Result<T, CborError>;
