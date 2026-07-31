//! CBOR diagnostic notation, ported from src/cborpretty.c.
//!
//! Upstream streams each fragment through a printf-style callback; this builds
//! one `String`. The bytes are the same, and the caller prints what it has even
//! when an error follows, which is what keeps the partial output identical for
//! malformed input.

use crate::cbor::{self, chunk_head, head_at, After, Head, Reader, BREAK, MAX_RECURSIONS};
use cbor_core::fmt::{escape_utf8, format_g};
use cbor_core::CborError;
use std::fmt::Write;

/// `CborPrettyFlags`.
pub const NUMERIC_ENCODING_INDICATORS: i32 = 0x01;
pub const INDICATE_INDETERMINATE_LENGTH: i32 = 0x02;
pub const INDICATE_OVERLONG_NUMBERS: i32 = 0x04;
pub const SHOW_STRING_FRAGMENTS: i32 = 0x100;
/// `CborPrettyDefaultFlags`, which is what the JSON converter borrows.
pub const DEFAULT_FLAGS: i32 = INDICATE_INDETERMINATE_LENGTH;

const RECURSION_LIMIT: &str = "<nesting too deep, recursion stopped>";

/// The `_N` suffix that follows an item, from `resolve_indicator()`.
///
/// `_` marks an indefinite length. `_0`..`_3` mark a head written in more bytes
/// than the argument needed; cbordump can never ask for those, because its `-n`
/// sets `CborPrettyNumericEncodingIndicators` rather than
/// `CborPrettyIndicateOverlongNumbers` — see the note in main.rs.
///
/// Reads the raw descriptor rather than a decoded [`Head`] on purpose: it is
/// also called on the break code, where it has to answer `_` and not fail.
fn indicator(buf: &[u8], pos: usize, flags: i32) -> &'static str {
    const SUFFIXES: [&str; 8] = ["_0", "_1", "_2", "_3", "", "", "", "_"];
    let Some(&descriptor) = buf.get(pos) else {
        return "";
    };
    let ai = descriptor & 0x1f;
    if ai < 24 {
        return "";
    }
    if flags & INDICATE_INDETERMINATE_LENGTH != 0 && ai == 31 {
        return "_";
    }
    if flags & INDICATE_OVERLONG_NUMBERS == 0 {
        return "";
    }
    let Ok(h) = head_at(buf, pos) else {
        return "";
    };
    // The shortest additional information that could have carried this value.
    let expected = 23
        + u8::from(h.value >= 24)
        + u8::from(h.value > 0xff)
        + u8::from(h.value > 0xffff)
        + u8::from(h.value > 0xffff_ffff);
    if expected == ai {
        ""
    } else {
        SUFFIXES[usize::from(ai - 24)]
    }
}

/// `convertToUint64`: the magnitude of `v`, if `v` is a whole number that a
/// `u64` can hold. Upstream prints those as an integer with a trailing dot
/// instead of in `%g` form, so `1.0` reads as `1.` and not `1`.
fn as_whole_u64(v: f64) -> Option<u64> {
    let v = v.abs();
    // 2^64. NaN fails this comparison, which is the point.
    // NaN spelled out rather than left to fall out of a negated comparison.
    if v.is_nan() || v >= 18446744073709551616.0 {
        return None;
    }
    let i = v as u64;
    (i as f64 == v).then_some(i)
}

pub fn value(
    out: &mut String,
    r: &mut Reader,
    flags: i32,
    recursions_left: i32,
    after: After,
) -> Result<(), CborError> {
    let h = r.head()?;
    match h.major {
        0 => {
            let _ = write!(out, "{}", h.value);
            out.push_str(indicator(r.buf, r.pos, flags));
            r.pos += h.size;
        }
        1 => {
            // Stored as -1-n (RFC 8949 §3.1), so the magnitude printed is n+1
            // and the largest one overflows a u64 by exactly one.
            match h.value.checked_add(1) {
                Some(magnitude) => {
                    let _ = write!(out, "-{magnitude}");
                }
                None => out.push_str("-18446744073709551616"),
            }
            out.push_str(indicator(r.buf, r.pos, flags));
            r.pos += h.size;
        }
        2 | 3 => return string(out, r, &h, flags, after),
        4 | 5 => {
            let ind = indicator(r.buf, r.pos, flags);
            out.push(if h.major == 4 { '[' } else { '{' });
            out.push_str(ind);
            if !ind.is_empty() {
                out.push(' ');
            }
            r.pos += h.size;
            let count = cbor::enter(r, &h)?;
            container(out, r, &h, count, flags, recursions_left - 1)?;
            // Leaving the container is what validates whatever follows it, and
            // upstream does that before it writes the closing bracket.
            after.apply(r)?;
            out.push(if h.major == 4 { ']' } else { '}' });
            return Ok(());
        }
        6 => {
            let _ = write!(out, "{}{}(", h.value, indicator(r.buf, r.pos, flags));
            r.pos += h.size;
            if recursions_left > 0 {
                // A tag does not occupy a slot in its container, so the item it
                // tags inherits what happens after the pair of them.
                value(out, r, flags, recursions_left - 1, after)?;
            } else {
                // Upstream does not skip the tagged item here, so the caller
                // is left mid-stream and reports garbage at the end. Kept.
                out.push_str(RECURSION_LIMIT);
            }
            out.push(')');
            return Ok(());
        }
        _ => {
            match h.ai {
                20 => out.push_str("false"),
                21 => out.push_str("true"),
                22 => out.push_str("null"),
                23 => out.push_str("undefined"),
                25..=27 => float(out, &h, flags),
                _ => {
                    let _ = write!(out, "simple({})", h.value);
                }
            }
            r.pos += h.size;
        }
    }
    after.apply(r)
}

fn float(out: &mut String, h: &Head, flags: i32) {
    let numeric = flags & NUMERIC_ENCODING_INDICATORS != 0;
    let (val, mut suffix) = match h.ai {
        25 => (
            f64::from(cbor_core::half::decode(h.value as u16)),
            if numeric { "_1" } else { "f16" },
        ),
        26 => (
            f64::from(f32::from_bits(h.value as u32)),
            if numeric { "_2" } else { "f" },
        ),
        _ => (f64::from_bits(h.value), ""),
    };
    // Textual suffixes disambiguate a printed number from an integer; nan and
    // inf are already unambiguous, so they lose theirs.
    if !numeric && (val.is_nan() || val.is_infinite()) {
        suffix = "";
    }
    match as_whole_u64(val) {
        Some(magnitude) => {
            if val < 0.0 {
                out.push('-');
            }
            let _ = write!(out, "{magnitude}.");
        }
        None => out.push_str(&format_g(val, 17)),
    }
    out.push_str(suffix);
}

fn container(
    out: &mut String,
    r: &mut Reader,
    h: &Head,
    count: u64,
    flags: i32,
    recursions_left: i32,
) -> Result<(), CborError> {
    if recursions_left <= 0 {
        out.push_str(RECURSION_LIMIT);
        // Upstream still walks to the end of the container so the dump can
        // continue after it.
        if h.indefinite() {
            while r.peek() != Some(BREAK) {
                cbor::skip(r, MAX_RECURSIONS)?;
            }
            r.pos += 1;
        } else {
            for _ in 0..count {
                cbor::skip(r, MAX_RECURSIONS)?;
            }
        }
        return Ok(());
    }

    let is_map = h.major == 5;
    if h.indefinite() {
        let mut first = true;
        while r.peek() != Some(BREAK) {
            if !first {
                out.push_str(", ");
            }
            first = false;
            // A break code between a key and its value is not an ending, which
            // is why only the map's value may be followed by one.
            let first_after = if is_map {
                After::Next
            } else {
                After::BreakOrNext
            };
            value(out, r, flags, recursions_left, first_after)?;
            if is_map {
                out.push_str(": ");
                value(out, r, flags, recursions_left, After::BreakOrNext)?;
            }
        }
        r.pos += 1;
    } else {
        for i in 0..count {
            if i > 0 {
                out.push_str(if is_map && i % 2 == 1 { ": " } else { ", " });
            }
            let after = if i + 1 < count {
                After::Next
            } else {
                After::Stop
            };
            value(out, r, flags, recursions_left, after)?;
        }
    }
    Ok(())
}

fn string(
    out: &mut String,
    r: &mut Reader,
    h: &Head,
    flags: i32,
    after: After,
) -> Result<(), CborError> {
    let text = h.major == 3;
    let open = if text { "\"" } else { "h'" };
    let close = if text { '"' } else { '\'' };
    let fragments = flags & SHOW_STRING_FRAGMENTS != 0 && h.indefinite();

    out.push_str(if fragments { "(_ " } else { open });

    if !h.indefinite() {
        let trailing = indicator(r.buf, r.pos, flags);
        r.pos += h.size;
        let len = usize::try_from(h.value).map_err(|_| CborError::DataTooLarge)?;
        let data = r.take(len)?;
        dump(out, data, text)?;
        // Finishing the iteration is what checks the next item, and upstream
        // does it before the closing quote goes out.
        after.apply(r)?;
        out.push(close);
        out.push_str(trailing);
        return Ok(());
    }

    r.pos += h.size;
    // With the fragments merged the indicator is resolved once, and by then the
    // string's own head is behind us — so what it describes is the *first
    // chunk*, or the break code if there are none. That is upstream's, and it
    // is why an empty chunked string prints as h''_ but h'61' does not.
    let mut trailing: Option<&str> = None;
    let mut separator = "";
    loop {
        if fragments || trailing.is_none() {
            trailing = Some(indicator(r.buf, r.pos, flags));
        }
        let Some((head_len, len)) = chunk_head(r.buf, r.pos, h.major)? else {
            break;
        };
        r.pos += head_len;
        let data = r.take(len)?;
        if fragments {
            out.push_str(separator);
            out.push_str(open);
        }
        dump(out, data, text)?;
        if fragments {
            out.push(close);
            out.push_str(trailing.unwrap_or_default());
            separator = ", ";
        }
    }
    r.pos += 1; // the break code
    after.apply(r)?;

    if fragments {
        out.push(')');
    } else {
        out.push(close);
        out.push_str(trailing.unwrap_or_default());
    }
    Ok(())
}

fn dump(out: &mut String, data: &[u8], text: bool) -> Result<(), CborError> {
    if !text {
        for b in data {
            let _ = write!(out, "{b:02x}");
        }
        return Ok(());
    }
    // Upstream escapes character by character and only notices bad UTF-8 when
    // it reaches it, by which point the good prefix has already been written.
    if let Err(e) = std::str::from_utf8(data) {
        let _ = escape_utf8(out, &data[..e.valid_up_to()]);
        return Err(CborError::InvalidUtf8TextString);
    }
    escape_utf8(out, data)
}
