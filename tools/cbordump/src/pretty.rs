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

/// Renders one item, and reports whether it filled a slot in its container.
///
/// It always does, except for a tag that runs out of recursion budget: upstream
/// prints the marker and returns without touching the item the tag applies to,
/// and a tag of its own never counts towards a container's total. The item is
/// therefore still owed, and the enclosing container renders it next -- so the
/// answer here is `false` and only there.
pub fn value(
    out: &mut String,
    r: &mut Reader,
    flags: i32,
    recursions_left: i32,
    after: After,
) -> Result<bool, CborError> {
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
        2 | 3 => return string(out, r, &h, flags, after).map(|()| true),
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
            return Ok(true);
        }
        6 => {
            let _ = write!(out, "{}{}(", h.value, indicator(r.buf, r.pos, flags));
            r.pos += h.size;
            // Upstream steps off the tag with cbor_value_advance_fixed, which
            // preparses whatever follows, and gives up if that fails -- before
            // it looks at how much recursion budget is left. Recursing does the
            // same check on the way in, so this only matters on the branch that
            // does not recurse: a document ending in a tag at the depth limit
            // has run out of data, and saying so beats printing a marker for a
            // nesting problem that is not the reason the render stopped.
            r.preparse()?;
            let filled = if recursions_left > 0 {
                // A tag does not occupy a slot in its container, so the item it
                // tags inherits what happens after the pair of them.
                value(out, r, flags, recursions_left - 1, after)?
            } else {
                // The tagged item is deliberately left unread, matching
                // upstream, which means the slot it was going to fill is still
                // open. See this function's doc comment.
                out.push_str(RECURSION_LIMIT);
                false
            };
            out.push(')');
            return Ok(filled);
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
    after.apply(r)?;
    Ok(true)
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

/// What may follow the item about to be rendered inside an indefinite
/// container that has taken `filled` items so far.
///
/// A break always ends an indefinite array. In a map it may only arrive on a
/// pair boundary, so it is allowed after this item exactly when this item makes
/// the count even.
fn break_after(is_map: bool, filled: u64) -> After {
    if is_map && filled.is_multiple_of(2) {
        After::Next
    } else {
        After::BreakOrNext
    }
}

/// The same question for a definite container: whatever follows the last item
/// belongs to the enclosing container, so this one stops looking.
fn stop_after(owed: u64) -> After {
    if owed > 1 {
        After::Next
    } else {
        After::Stop
    }
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
            // Same pair rule the rendering path below enforces: this container
            // is not being printed, but it still has to be well formed for the
            // dump to carry on after it.
            let mut items = 0u64;
            while r.peek() != Some(BREAK) {
                cbor::skip(r, MAX_RECURSIONS)?;
                items += 1;
            }
            if h.major == 5 && !items.is_multiple_of(2) {
                return Err(CborError::UnexpectedBreak);
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
        // Whether a break ends the container depends on how many items it has
        // taken, and a tag that ran out of recursion budget renders without
        // taking one. So the count that decides it is the filled slots, not the
        // rendered positions -- upstream counts the same way, by only toggling
        // its map-key flag on items that are not tags.
        let mut filled = 0u64;
        let mut first = true;
        loop {
            if r.peek() == Some(BREAK) {
                // Halfway through a pair, a break is not an ending.
                if is_map && !filled.is_multiple_of(2) {
                    return Err(CborError::UnexpectedBreak);
                }
                break;
            }
            if !first {
                out.push_str(", ");
            }
            first = false;
            // A break is only an ending on an even boundary, and the item about
            // to be rendered is the one that moves the count. Rejecting it here
            // rather than after the fact is what upstream does -- it checks at
            // the item's own advance -- and it matters for a tagged item, whose
            // closing bracket is never reached.
            if value(out, r, flags, recursions_left, break_after(is_map, filled))? {
                filled += 1;
            }
            if !is_map {
                continue;
            }
            out.push_str(": ");
            if r.peek() == Some(BREAK) {
                // An even count means the break really does end the container,
                // but the render is committed to a value. Upstream's iterator
                // is left holding CborInvalidType and its printer has an arm
                // for exactly that: the word, and the type as the error.
                out.push_str("invalid");
                return Err(CborError::UnknownType);
            }
            if value(out, r, flags, recursions_left, break_after(is_map, filled))? {
                filled += 1;
            }
        }
        r.pos += 1;
    } else {
        // Driven by how many items the container still owes rather than by a
        // fixed index, because a tag that hits the recursion limit renders
        // without filling its slot and the item it was tagging is then rendered
        // by the next turn of this loop. Upstream gets that for free: its loop
        // condition is the iterator's own remaining count, which the tag never
        // decremented.
        //
        // A map renders a whole pair per turn, and the count is only consulted
        // at the top, so a map whose slots run out mid-pair still owes a value.
        let mut owed = count;
        let mut first = true;
        while owed > 0 {
            if !first {
                out.push_str(", ");
            }
            first = false;
            if value(out, r, flags, recursions_left, stop_after(owed))? {
                owed -= 1;
            }
            if !is_map {
                continue;
            }
            out.push_str(": ");
            if owed == 0 {
                // Nothing left to spend on the value half. Upstream's iterator
                // is holding CborInvalidType by now, and its printer has the
                // arm for that: the word, and the type as the error.
                out.push_str("invalid");
                return Err(CborError::UnknownType);
            }
            if value(out, r, flags, recursions_left, stop_after(owed))? {
                owed -= 1;
            }
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
