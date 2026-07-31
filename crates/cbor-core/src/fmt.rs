//! Formatting that has to match C byte for byte.
//!
//! The pretty printer's output is compared against fixtures produced by
//! `printf`, so "close enough" is not a passing grade. These two functions are
//! the parts where Rust's defaults differ from C's and the difference shows up
//! in the diff.

use alloc::format;
use alloc::string::{String, ToString};

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
        // C prints an unsigned "nan" here; glibc only signs it with %+g.
        return "nan".to_string();
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
