//! Formatting that has to match C byte for byte.
//!
//! The pretty printer's output is compared against fixtures produced by
//! `printf`, so "close enough" is not a passing grade. These two functions are
//! the parts where Rust's defaults differ from C's and the difference shows up
//! in the diff.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// C's `%.*g`, which Rust has no equivalent for.
///
/// The rule from C99 §7.19.6.1: let X be the base-10 exponent. If
/// `-4 <= X < precision`, print in `%f` style with `precision - 1 - X` digits
/// after the point; otherwise `%e` style with `precision - 1`. Either way,
/// trailing zeros come off, because the `#` flag is not in play.
///
/// The pretty printer calls this with 17, which is `DBL_DECIMAL_DIG` — enough
/// digits that every double round-trips.
pub fn format_g(v: f64, precision: usize) -> String {
    if v.is_nan() {
        // glibc prints the sign bit of a NaN, so -NaN is "-nan". Rust's own
        // Display drops it, and Python's %g normalises it away, so both are
        // misleading references here — this was found by differential fuzzing
        // against the C library, not by reasoning about it.
        return if v.is_sign_negative() { "-nan" } else { "nan" }.to_string();
    }
    if v.is_infinite() {
        return if v < 0.0 { "-inf" } else { "inf" }.to_string();
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0" } else { "0" }.to_string();
    }

    // Rust's {:e} gives the exponent for free, which is the one number the
    // C rule is written in terms of.
    let sci = format!("{:.*e}", precision.saturating_sub(1), v);
    let (mantissa, exponent) = sci.split_once('e').expect("{:e} always emits an e");
    let x: i32 = exponent
        .parse()
        .expect("{:e} always emits a decimal exponent");

    if x < -4 || x >= precision as i32 {
        // C writes the exponent with a sign and at least two digits.
        format!(
            "{}e{}{:02}",
            strip_zeros(mantissa),
            if x < 0 { '-' } else { '+' },
            x.abs()
        )
    } else {
        let after_point = (precision as i32 - 1 - x).max(0) as usize;
        strip_zeros(&format!("{:.*}", after_point, v))
    }
}

/// Drops trailing zeros in a fractional part, and the point if nothing is left.
fn strip_zeros(s: &str) -> String {
    if !s.contains('.') {
        return s.to_string();
    }
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Appends `bytes` to `out` as a CBOR-diagnostic string body, escaping what has
/// to be escaped and rejecting malformed UTF-8.
///
/// Upstream escapes to `\uXXXX` rather than emitting raw UTF-8, and splits
/// astral characters into a surrogate pair, so the output is ASCII-only. Note
/// `0x7f` is escaped too: the condition upstream is `< 0x7f`, not `<= 0x7f`.
pub fn escape_utf8(out: &mut String, bytes: &[u8]) -> Result<(), crate::CborError> {
    let text = core::str::from_utf8(bytes).map_err(|_| crate::CborError::InvalidUtf8TextString)?;

    for ch in text.chars() {
        let uc = ch as u32;
        if uc < 0x80 {
            match ch {
                '"' | '\\' => {
                    out.push('\\');
                    out.push(ch);
                }
                '\u{8}' => out.push_str("\\b"),
                '\u{c}' => out.push_str("\\f"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                _ if (0x20..0x7f).contains(&uc) => out.push(ch),
                _ => out.push_str(&format!("\\u{uc:04X}")),
            }
        } else if uc > 0xffff {
            let hi = (uc >> 10) + 0xd7c0;
            let lo = (uc % 0x400) + 0xdc00;
            out.push_str(&format!("\\u{hi:04X}\\u{lo:04X}"));
        } else {
            out.push_str(&format!("\\u{uc:04X}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_printf_g() {
        // Spot values checked against printf("%.17g").
        assert_eq!(format_g(0.0, 17), "0");
        assert_eq!(format_g(-0.0, 17), "-0");
        assert_eq!(format_g(1.0, 17), "1");
        assert_eq!(format_g(-1.5, 17), "-1.5");
        assert_eq!(format_g(0.5, 17), "0.5");
        assert_eq!(format_g(9.8765, 17), "9.8765000000000001");
        assert_eq!(format_g(1e100, 17), "1e+100");
        assert_eq!(format_g(0.1, 17), "0.10000000000000001");
        assert_eq!(format_g(123456789.0, 17), "123456789");
        // The %f/%e switch happens at exponent 17, not at some round number.
        assert_eq!(format_g(1e16, 17), "10000000000000000");
        assert_eq!(format_g(1e17, 17), "1e+17");
        // ...and at -4 on the other side.
        assert_eq!(format_g(1e-4, 17), "0.0001");
        assert_eq!(format_g(1e-5, 17), "1.0000000000000001e-05");
        assert_eq!(format_g(1e-300, 17), "1e-300");
        assert_eq!(format_g(2.0f64.powi(64), 17), "1.8446744073709552e+19");
        assert_eq!(format_g(f64::INFINITY, 17), "inf");
        assert_eq!(format_g(f64::NEG_INFINITY, 17), "-inf");
        assert_eq!(format_g(f64::NAN, 17), "nan");
        // glibc signs a NaN. Confirmed against the C oracle, not assumed.
        assert_eq!(format_g(-f64::NAN, 17), "-nan");
    }

    #[test]
    fn strips_only_trailing_zeros() {
        assert_eq!(strip_zeros("1.500"), "1.5");
        assert_eq!(strip_zeros("1.000"), "1");
        assert_eq!(strip_zeros("100"), "100"); // no point, no stripping
        assert_eq!(strip_zeros("0.0"), "0");
    }

    #[test]
    fn escapes_what_c_escapes() {
        let mut s = String::new();
        escape_utf8(&mut s, b"hi").unwrap();
        assert_eq!(s, "hi");

        s.clear();
        escape_utf8(&mut s, b"a\"b\\c\nd\te").unwrap();
        assert_eq!(s, "a\\\"b\\\\c\\nd\\te");

        // 0x7f is a control character to C's condition, so it escapes.
        s.clear();
        escape_utf8(&mut s, b"\x7f\x1f").unwrap();
        assert_eq!(s, "\\u007F\\u001F");

        // Non-ASCII becomes \u, astral becomes a surrogate pair.
        s.clear();
        escape_utf8(&mut s, "é".as_bytes()).unwrap();
        assert_eq!(s, "\\u00E9");
        s.clear();
        escape_utf8(&mut s, "😀".as_bytes()).unwrap();
        assert_eq!(s, "\\uD83D\\uDE00");
    }

    #[test]
    fn rejects_bad_utf8() {
        let mut s = String::new();
        assert!(escape_utf8(&mut s, &[0xff, 0xfe]).is_err());
    }
}

/// Appends `bytes` to `out` as a JSON string body (RFC 8259 §7).
///
/// This is a different job from [`escape_utf8`], which is for CBOR diagnostic
/// notation. JSON allows any Unicode character between the quotes, so multi-byte
/// sequences pass straight through and only the control characters, the quote
/// and the backslash are escaped. Upstream additionally spells out the five
/// characters that have short escapes rather than emitting `\u00XX`.
///
/// Output is `Vec<u8>`, not `String`, on purpose. The input is not required to
/// be valid UTF-8 — upstream copies bytes through, and the CBOR side has its own
/// UTF-8 validation, so rejecting here would change which errors callers see.
/// Pushing an arbitrary byte into a `String` would need `unsafe`, and this crate
/// forbids it.
pub fn escape_json(out: &mut Vec<u8>, bytes: &[u8]) {
    for &c in bytes {
        match c {
            b'\x08' => out.extend_from_slice(b"\\b"),
            b'\t' => out.extend_from_slice(b"\\t"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\x0c' => out.extend_from_slice(b"\\f"),
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0x00..=0x1f => out.extend_from_slice(format!("\\u{c:04x}").as_bytes()),
            _ => out.push(c),
        }
    }
}

/// Base16, lowercase. Upstream uses this for tag 23 byte strings.
pub fn base16(out: &mut Vec<u8>, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize]);
        out.push(HEX[(b & 0xf) as usize]);
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Base64 with `=` padding (RFC 4648 §4), for byte strings tagged 22.
pub fn base64(out: &mut Vec<u8>, bytes: &[u8]) {
    encode_base64(out, bytes, B64, Some(b'='));
}

/// Base64url without padding (RFC 4648 §5), which is the default for byte
/// strings. Upstream's alphabet table has no 65th character here, so the
/// padding is simply omitted rather than replaced.
pub fn base64url(out: &mut Vec<u8>, bytes: &[u8]) {
    encode_base64(out, bytes, B64URL, None);
}

fn encode_base64(out: &mut Vec<u8>, bytes: &[u8], alphabet: &[u8; 64], pad: Option<u8>) {
    let mut chunks = bytes.chunks_exact(3);
    for c in &mut chunks {
        let v = ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32;
        for shift in [18, 12, 6, 0] {
            out.push(alphabet[((v >> shift) & 0x3f) as usize]);
        }
    }
    let rest = chunks.remainder();
    if rest.is_empty() {
        return;
    }
    // One or two bytes left. The value is left-aligned in 24 bits, so the same
    // shifts work and the unused groups become padding.
    let mut v = (rest[0] as u32) << 16;
    if rest.len() == 2 {
        v |= (rest[1] as u32) << 8;
    }
    out.push(alphabet[((v >> 18) & 0x3f) as usize]);
    out.push(alphabet[((v >> 12) & 0x3f) as usize]);
    if rest.len() == 2 {
        out.push(alphabet[((v >> 6) & 0x3f) as usize]);
    } else if let Some(p) = pad {
        out.push(p);
    }
    if let Some(p) = pad {
        out.push(p);
    }
}

#[cfg(test)]
mod json_tests {
    use super::*;
    use alloc::vec::Vec;

    fn s(v: &[u8]) -> &str {
        core::str::from_utf8(v).expect("test vectors are ascii or valid utf-8")
    }

    #[test]
    fn json_escapes_only_what_json_requires() {
        let mut v = Vec::new();
        escape_json(&mut v, b"a\"b\\c\nd\te\x01f");
        assert_eq!(s(&v), "a\\\"b\\\\c\\nd\\te\\u0001f");
        // Unlike diagnostic notation, UTF-8 passes through unescaped.
        v.clear();
        escape_json(&mut v, "héllo 😀".as_bytes());
        assert_eq!(s(&v), "héllo 😀");
        // 0x7f is not a JSON control character, so it stays raw.
        v.clear();
        escape_json(&mut v, b"\x7f");
        assert_eq!(v, b"\x7f");
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        let cases = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (input, want) in cases {
            let mut v = Vec::new();
            base64(&mut v, input.as_bytes());
            assert_eq!(s(&v), want, "base64({input:?})");
        }
    }

    #[test]
    fn base64url_is_unpadded_and_uses_the_url_alphabet() {
        let mut v = Vec::new();
        base64url(&mut v, &[0xfb, 0xff, 0xbe]);
        assert_eq!(s(&v), "-_--"); // + and / would appear here in standard base64
        v.clear();
        base64url(&mut v, b"f");
        assert_eq!(s(&v), "Zg"); // no padding
        v.clear();
        base64(&mut v, &[0xfb, 0xff, 0xbe]);
        assert_eq!(s(&v), "+/++");
    }

    #[test]
    fn base16_is_lowercase() {
        let mut v = Vec::new();
        base16(&mut v, &[0x00, 0xde, 0xad, 0xff]);
        assert_eq!(s(&v), "00deadff");
    }
}
