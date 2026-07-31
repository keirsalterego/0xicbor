//! Encoding CBOR heads (RFC 8949 §3).
//!
//! Every CBOR data item starts with a head: three bits of major type, five bits
//! of additional info, then zero to eight big-endian length bytes. That is the
//! only part of encoding that involves any real decisions, so it lives here as a
//! pure function. Buffer management stays in the FFI shim, where the pointers
//! are.

/// Major types, already shifted into the top three bits of the head byte.
pub mod major {
    pub const UNSIGNED: u8 = 0 << 5;
    pub const NEGATIVE: u8 = 1 << 5;
    pub const BYTE_STRING: u8 = 2 << 5;
    pub const TEXT_STRING: u8 = 3 << 5;
    pub const ARRAY: u8 = 4 << 5;
    pub const MAP: u8 = 5 << 5;
    pub const TAG: u8 = 6 << 5;
    pub const SIMPLE: u8 = 7 << 5;
}

/// The additional-info value meaning "indefinite length", and the break byte
/// that terminates such an item.
pub const INDEFINITE: u8 = 31;
pub const BREAK: u8 = major::SIMPLE | INDEFINITE;

/// A head is at most nine bytes: one of type, eight of length.
pub type Head = ([u8; 9], usize);

/// Builds the head for `major` carrying `value`.
///
/// RFC 8949 §3 requires the shortest form that fits, which is what the ladder of
/// comparisons below picks. Encoding 1 as `0x19 0x00 0x01` is legal CBOR but not
/// canonical, and upstream never emits it.
pub fn head(major: u8, value: u64) -> Head {
    let mut out = [0u8; 9];
    match value {
        0..=23 => {
            out[0] = major | value as u8;
            (out, 1)
        }
        24..=0xff => {
            out[0] = major | 24;
            out[1] = value as u8;
            (out, 2)
        }
        0x100..=0xffff => {
            out[0] = major | 25;
            out[1..3].copy_from_slice(&(value as u16).to_be_bytes());
            (out, 3)
        }
        0x1_0000..=0xffff_ffff => {
            out[0] = major | 26;
            out[1..5].copy_from_slice(&(value as u32).to_be_bytes());
            (out, 5)
        }
        _ => {
            out[0] = major | 27;
            out[1..9].copy_from_slice(&value.to_be_bytes());
            (out, 9)
        }
    }
}

/// The head for a container whose length is not known up front.
pub fn indefinite_head(major: u8) -> Head {
    let mut out = [0u8; 9];
    out[0] = major | INDEFINITE;
    (out, 1)
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;

    fn bytes(major: u8, value: u64) -> alloc::vec::Vec<u8> {
        let (buf, len) = head(major, value);
        buf[..len].to_vec()
    }

    #[test]
    fn picks_the_shortest_form() {
        assert_eq!(bytes(major::UNSIGNED, 0), [0x00]);
        assert_eq!(bytes(major::UNSIGNED, 23), [0x17]);
        assert_eq!(bytes(major::UNSIGNED, 24), [0x18, 0x18]);
        assert_eq!(bytes(major::UNSIGNED, 255), [0x18, 0xff]);
        assert_eq!(bytes(major::UNSIGNED, 256), [0x19, 0x01, 0x00]);
        assert_eq!(bytes(major::UNSIGNED, 65535), [0x19, 0xff, 0xff]);
        assert_eq!(
            bytes(major::UNSIGNED, 65536),
            [0x1a, 0x00, 0x01, 0x00, 0x00]
        );
        assert_eq!(
            bytes(major::UNSIGNED, u32::MAX as u64 + 1),
            [0x1b, 0, 0, 0, 1, 0, 0, 0, 0]
        );
        assert_eq!(
            bytes(major::UNSIGNED, u64::MAX),
            [0x1b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]
        );
    }

    #[test]
    fn major_type_lands_in_the_top_bits() {
        // Appendix A of RFC 8949: these are the canonical examples.
        assert_eq!(bytes(major::NEGATIVE, 9), [0x29]); // -10
        assert_eq!(bytes(major::BYTE_STRING, 4), [0x44]);
        assert_eq!(bytes(major::TEXT_STRING, 5), [0x65]);
        assert_eq!(bytes(major::ARRAY, 3), [0x83]);
        assert_eq!(bytes(major::MAP, 2), [0xa2]);
        assert_eq!(bytes(major::TAG, 0), [0xc0]);
        assert_eq!(bytes(major::SIMPLE, 24), [0xf8, 0x18]);
    }

    #[test]
    fn indefinite_and_break() {
        assert_eq!(indefinite_head(major::ARRAY).0[0], 0x9f);
        assert_eq!(indefinite_head(major::MAP).0[0], 0xbf);
        assert_eq!(indefinite_head(major::BYTE_STRING).0[0], 0x5f);
        assert_eq!(BREAK, 0xff);
    }
}
