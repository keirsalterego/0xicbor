//! Human-readable text for each error code.
//!
//! Upstream returns a `const char *` from static storage, so these are `&'static
//! CStr` and the shim hands the pointer straight out. The wording is copied
//! exactly, including upstream's unbalanced parenthesis in the UnknownLength
//! message — tests compare these strings, so a tidier version would be a
//! behaviour change dressed up as a typo fix.

use core::ffi::CStr;

/// The message for a raw `CborError` value, or the unknown-error text for a
/// code that is not one of ours.
pub fn error_string(code: i32) -> &'static CStr {
    match code {
        0 => c"",
        1 => c"unknown error",
        2 => c"unknown length (attempted to get the length of a map/array/string of indeterminate length",
        3 => c"attempted to advance past EOF",
        4 => c"I/O error",

        256 => c"garbage after the end of the content",
        257 => c"unexpected end of data",
        258 => c"unexpected 'break' byte",
        259 => c"illegal byte (encodes future extension type)",
        260 => c"mismatched string type in chunked string",
        261 => c"illegal initial byte (encodes unspecified additional information)",
        262 => c"illegal encoding of simple type smaller than 32",
        263 => c"no more byte or text strings available",

        512 => c"unknown simple type",
        513 => c"unknown tag",
        514 => c"inappropriate tag for type",
        515 => c"duplicate keys in object",
        516 => c"invalid UTF-8 content in string",
        517 => c"excluded type found",
        518 => c"excluded value found",
        // ImproperValue and OverlongEncoding share one message upstream.
        519 | 520 => c"value encoded in non-canonical form",
        // As do MapKeyNotString and JsonObjectKeyNotString.
        521 | 1281 => c"key in map is not a string",
        522 => c"map is not sorted",
        523 => c"map keys are not unique",

        768 => c"too many items added to encoder",
        769 => c"too few items added to encoder",

        1024 => c"internal error: data too large",
        1025 => c"internal error: too many nested containers found in recursive function",
        1026 => c"unsupported type",
        1027 => c"validation not implemented for the current parser state",

        1280 => c"conversion to JSON failed: key in object is an array or map",
        1282 => c"conversion to JSON failed: open_memstream unavailable",

        i32::MIN => c"out of memory/need more memory",
        i32::MAX => c"internal error",

        _ => c"unknown error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_codes_have_text() {
        assert_eq!(error_string(0).to_bytes(), b"");
        assert_eq!(error_string(257).to_bytes(), b"unexpected end of data");
        assert_eq!(
            error_string(516).to_bytes(),
            b"invalid UTF-8 content in string"
        );
        // Both members of each shared arm land on the same message.
        assert_eq!(error_string(519), error_string(520));
        assert_eq!(error_string(521), error_string(1281));
        assert_eq!(
            error_string(i32::MIN).to_bytes(),
            b"out of memory/need more memory"
        );
    }

    #[test]
    fn unknown_codes_fall_back() {
        assert_eq!(error_string(9999).to_bytes(), b"unknown error");
        assert_eq!(error_string(-7).to_bytes(), b"unknown error");
    }
}
