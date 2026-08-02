//! CBOR to JSON, ported from src/cbortojson.c.
//!
//! The output is bytes rather than a `String` because upstream copies text
//! string payloads through unchanged — it escapes what JSON demands and
//! validates nothing, so anything not UTF-8 in the input is not UTF-8 in the
//! output either. The diagnostic-notation printer is the opposite and escapes
//! everything to ASCII, which is why it can keep using `String`.

use crate::cbor::{self, read_string, After, Head, Reader, BREAK, MAX_RECURSIONS};
use crate::pretty;
use cbor_core::fmt::format_g;
use cbor_core::CborError;
use std::io::Write;

/// `CborToJsonFlags`.
pub const ADD_METADATA: i32 = 1;
pub const TAGS_TO_OBJECTS: i32 = 2;
pub const BYTE_STRINGS_TO_BASE64URL: i32 = 4;
pub const STRINGIFY_MAP_KEYS: i32 = 8;

/// `ConversionStatusFlags`. The low byte holds a `CborType` when the value was
/// tagged, which is what `FINAL_TYPE_MASK` recovers.
const TYPE_WAS_NOT_NATIVE: i32 = 0x100;
const TYPE_WAS_TAGGED: i32 = 0x200;
const NUMBER_WAS_NAN: i32 = 0x800;
const NUMBER_WAS_INFINITE: i32 = 0x1000;
const NUMBER_WAS_NEGATIVE: i32 = 0x2000;
const FINAL_TYPE_MASK: i32 = 0xff;

/// Tags that change how a byte string is spelled (RFC 8949 §3.4.5).
const NEGATIVE_BIGNUM: u64 = 3;
const EXPECTED_BASE64: u64 = 22;
const EXPECTED_BASE16: u64 = 23;

/// `CborType` values, needed here because they end up in the metadata.
const TYPE_INTEGER: u8 = 0x00;
const TYPE_BYTE_STRING: u8 = 0x40;
const TYPE_TEXT_STRING: u8 = 0x60;
const TYPE_ARRAY: u8 = 0x80;
const TYPE_MAP: u8 = 0xa0;
const TYPE_TAG: u8 = 0xc0;
const TYPE_SIMPLE: u8 = 0xe0;
const TYPE_BOOLEAN: u8 = 0xf5;
const TYPE_NULL: u8 = 0xf6;
const TYPE_UNDEFINED: u8 = 0xf7;

#[derive(Default)]
struct Status {
    last_tag: u64,
    original_number: u64,
    flags: i32,
}

pub fn value(out: &mut Vec<u8>, r: &mut Reader, flags: i32) -> Result<(), CborError> {
    to_json(
        out,
        r,
        flags,
        MAX_RECURSIONS,
        &mut Status::default(),
        After::Stop,
    )
}

fn to_json(
    out: &mut Vec<u8>,
    r: &mut Reader,
    flags: i32,
    nesting: i32,
    status: &mut Status,
    after: After,
) -> Result<(), CborError> {
    status.flags = 0;
    if nesting == 0 {
        return Err(CborError::NestingTooDeep);
    }
    let h = r.head()?;
    match h.cbor_type() {
        TYPE_ARRAY | TYPE_MAP => {
            let array = h.major == 4;
            r.pos += h.size;
            // Upstream enters the container before writing the bracket, so a
            // container it refuses to enter produces no output at all.
            let count = cbor::enter(r, &h)?;
            out.push(if array { b'[' } else { b'{' });
            if array {
                array_to_json(out, r, &h, count, flags, nesting - 1, status)?;
            } else {
                map_to_json(out, r, &h, count, flags, nesting - 1, status)?;
            }
            // Unlike the pretty printer, this one writes the closing bracket
            // before it leaves the container.
            out.push(if array { b']' } else { b'}' });
            after.apply(r)?;
            // Containers never lose anything in translation.
            status.flags = 0;
            return Ok(());
        }

        // Numbers, null and booleans are spelled the same in both notations.
        TYPE_INTEGER | TYPE_NULL | TYPE_BOOLEAN => {
            let mut text = String::new();
            let res = pretty::value(&mut text, r, pretty::DEFAULT_FLAGS, MAX_RECURSIONS, after);
            out.extend_from_slice(text.as_bytes());
            return res.map(|_| ());
        }

        // Both string forms are built whole before anything is written, so a
        // failure to step off the end of one leaves no output at all — the
        // opposite of the pretty printer, which streams as it decodes.
        TYPE_BYTE_STRING => {
            let data = read_string(r)?;
            status.flags = TYPE_WAS_NOT_NATIVE;
            after.apply(r)?;
            out.push(b'"');
            out.extend_from_slice(&base64(&data, B64URL, false));
            out.push(b'"');
            return Ok(());
        }

        TYPE_TEXT_STRING => {
            let data = read_string(r)?;
            after.apply(r)?;
            out.push(b'"');
            escape(out, &data);
            out.push(b'"');
            return Ok(());
        }

        TYPE_TAG => return tagged_to_json(out, r, flags, nesting - 1, status, after),

        TYPE_SIMPLE => {
            status.flags = TYPE_WAS_NOT_NATIVE;
            status.original_number = h.value;
            let _ = write!(out, "\"simple({})\"", h.value);
            r.pos += h.size;
        }

        TYPE_UNDEFINED => {
            status.flags = TYPE_WAS_NOT_NATIVE;
            out.extend_from_slice(b"\"undefined\"");
            r.pos += h.size;
        }

        _ => {
            number(out, &h, status);
            r.pos += h.size;
        }
    }
    after.apply(r)
}

/// The float cases. JSON has one number type, so everything here is lossy and
/// the loss is what the metadata records.
fn number(out: &mut Vec<u8>, h: &Head, status: &mut Status) {
    let val = match h.ai {
        25 => {
            status.flags = TYPE_WAS_NOT_NATIVE;
            f64::from(cbor_core::half::decode(h.value as u16))
        }
        26 => {
            status.flags = TYPE_WAS_NOT_NATIVE;
            f64::from(f32::from_bits(h.value as u32))
        }
        _ => f64::from_bits(h.value),
    };

    if val.is_nan() {
        out.extend_from_slice(b"null");
        status.flags |= NUMBER_WAS_NAN;
    } else if val.is_infinite() {
        out.extend_from_slice(b"null");
        status.flags |= NUMBER_WAS_INFINITE;
        if val < 0.0 {
            status.flags |= NUMBER_WAS_NEGATIVE;
        }
    } else {
        let magnitude = val.abs();
        // 2^64: anything smaller and integral is printed as an integer, which
        // keeps every bit of it that JSON can carry.
        if magnitude < 18446744073709551616.0 && (magnitude as u64) as f64 == magnitude {
            let _ = write!(
                out,
                "{}{}",
                if val < 0.0 { "-" } else { "" },
                magnitude as u64
            );
            status.flags |= TYPE_WAS_NOT_NATIVE;
        } else {
            out.extend_from_slice(format_g(val, 17).as_bytes());
        }
    }
}

fn array_to_json(
    out: &mut Vec<u8>,
    r: &mut Reader,
    h: &Head,
    count: u64,
    flags: i32,
    nesting: i32,
    status: &mut Status,
) -> Result<(), CborError> {
    let mut comma: &[u8] = b"";
    if h.indefinite() {
        while r.peek() != Some(BREAK) {
            out.extend_from_slice(comma);
            comma = b",";
            to_json(out, r, flags, nesting, status, After::BreakOrNext)?;
        }
        r.pos += 1;
    } else {
        for i in 0..count {
            out.extend_from_slice(comma);
            comma = b",";
            let after = if i + 1 < count {
                After::Next
            } else {
                After::Stop
            };
            to_json(out, r, flags, nesting, status, after)?;
        }
    }
    Ok(())
}

fn map_to_json(
    out: &mut Vec<u8>,
    r: &mut Reader,
    h: &Head,
    count: u64,
    flags: i32,
    nesting: i32,
    status: &mut Status,
) -> Result<(), CborError> {
    let mut comma: &[u8] = b"";
    let mut done = 0u64;
    loop {
        if h.indefinite() {
            if r.peek() == Some(BREAK) {
                r.pos += 1;
                break;
            }
        } else if done == count {
            break;
        }
        out.extend_from_slice(comma);
        comma = b",";

        let key_type = r.head()?.cbor_type();
        let key = if key_type == TYPE_TEXT_STRING {
            let data = read_string(r)?;
            let mut escaped = Vec::new();
            escape(&mut escaped, &data);
            escaped
        } else if flags & STRINGIFY_MAP_KEYS != 0 {
            stringify_map_key(r)?
        } else {
            return Err(CborError::JsonObjectKeyNotString);
        };

        // Upstream validated the value's head on its way out of the key, so
        // this is where a malformed value is reported — before the key is
        // written, not after.
        let value_type = r.head()?.cbor_type();
        out.push(b'"');
        out.extend_from_slice(&key);
        out.extend_from_slice(b"\":");

        done += 2;
        // The value carries the check of whatever follows the pair, so a map
        // that runs out here is reported before this pair's metadata is
        // written rather than after it.
        let after = if h.indefinite() {
            After::BreakOrNext
        } else if done < count {
            After::Next
        } else {
            After::Stop
        };
        let res = to_json(out, r, flags, nesting, status, after);

        if flags & ADD_METADATA != 0 && res.is_ok() {
            if key_type != TYPE_TEXT_STRING {
                out.push(b',');
                out.push(b'"');
                out.extend_from_slice(&key);
                out.extend_from_slice(b"$keycbordump\":true");
            }
            if status.flags != 0 {
                out.push(b',');
                out.push(b'"');
                out.extend_from_slice(&key);
                out.extend_from_slice(b"$cbor\":{");
                add_value_metadata(out, value_type, status);
                out.push(b'}');
            }
        }
        res?;
    }
    Ok(())
}

/// A non-text map key, rendered as diagnostic notation and then escaped so it
/// can sit inside JSON quotes. json2cbor cannot read these back.
fn stringify_map_key(r: &mut Reader) -> Result<Vec<u8>, CborError> {
    let mut text = String::new();
    pretty::value(
        &mut text,
        r,
        pretty::DEFAULT_FLAGS,
        MAX_RECURSIONS,
        After::Next,
    )?;
    let mut out = Vec::new();
    escape(&mut out, text.as_bytes());
    Ok(out)
}

fn tagged_to_json(
    out: &mut Vec<u8>,
    r: &mut Reader,
    flags: i32,
    nesting: i32,
    status: &mut Status,
    after: After,
) -> Result<(), CborError> {
    let h = r.head()?;

    if flags & TAGS_TO_OBJECTS != 0 {
        let tag = h.value;
        r.pos += h.size;
        // Upstream advances past the tag before it writes anything, so a tag
        // with nothing after it produces no output at all.
        let inner_type = r.head()?.cbor_type();
        let _ = write!(out, "{{\"tag{tag}\":");
        // A tag takes up no slot in its container, so the item it tags inherits
        // whatever happens after the pair of them.
        to_json(out, r, flags, nesting, status, after)?;
        if flags & ADD_METADATA != 0 && status.flags != 0 {
            let _ = write!(out, ",\"tag{tag}$cbor\":{{");
            add_value_metadata(out, inner_type, status);
            out.push(b'}');
        }
        out.push(b'}');
        status.flags = TYPE_WAS_NOT_NATIVE | i32::from(TYPE_TAG);
        return Ok(());
    }

    // find_tagged_type: nested tags collapse to the innermost one.
    let mut levels = nesting;
    let mut inner = h;
    while inner.major == 6 {
        if levels == 0 {
            return Err(CborError::NestingTooDeep);
        }
        levels -= 1;
        status.last_tag = inner.value;
        r.pos += inner.size;
        inner = r.head()?;
    }
    let inner_type = inner.cbor_type();
    let tag = status.last_tag;

    if inner_type == TYPE_BYTE_STRING
        && flags & BYTE_STRINGS_TO_BASE64URL == 0
        && matches!(tag, NEGATIVE_BIGNUM | EXPECTED_BASE16 | EXPECTED_BASE64)
    {
        let data = read_string(r)?;
        after.apply(r)?;
        let (prefix, encoded) = match tag {
            NEGATIVE_BIGNUM => ("~", base64(&data, B64URL, false)),
            EXPECTED_BASE64 => ("", base64(&data, B64, true)),
            _ => ("", base16(&data)),
        };
        out.push(b'"');
        out.extend_from_slice(prefix.as_bytes());
        out.extend_from_slice(&encoded);
        out.push(b'"');
        status.flags = TYPE_WAS_NOT_NATIVE | TYPE_WAS_TAGGED | i32::from(TYPE_BYTE_STRING);
        return Ok(());
    }

    to_json(out, r, flags, nesting, status, after)?;
    status.flags |= TYPE_WAS_TAGGED | i32::from(inner_type);
    Ok(())
}

fn add_value_metadata(out: &mut Vec<u8>, value_type: u8, status: &Status) {
    let mut flags = status.flags;
    let mut value_type = value_type;

    if flags & TYPE_WAS_TAGGED != 0 {
        // The tagged type was stashed in the low byte; recover it and report
        // the tag separately.
        value_type = (flags & FINAL_TYPE_MASK) as u8;
        flags &= !(FINAL_TYPE_MASK | TYPE_WAS_TAGGED);
        let _ = write!(
            out,
            "\"tag\":\"{}\"{}",
            status.last_tag,
            if flags != 0 { "," } else { "" }
        );
    }
    if flags == 0 {
        return;
    }

    let _ = write!(out, "\"t\":{value_type}");
    if flags & NUMBER_WAS_NAN != 0 {
        out.extend_from_slice(b",\"v\":\"nan\"");
    }
    if flags & NUMBER_WAS_INFINITE != 0 {
        let _ = write!(
            out,
            ",\"v\":\"{}inf\"",
            if flags & NUMBER_WAS_NEGATIVE != 0 {
                "-"
            } else {
                ""
            }
        );
    }
    // NumberPrecisionWasLost exists in the C enum but nothing sets it, so the
    // "v":"+hex" form it would produce is unreachable. Left out rather than
    // written as dead code.
    if value_type == TYPE_SIMPLE {
        let _ = write!(out, ",\"v\":{}", status.original_number as i32);
    }
}

/// JSON string escaping per RFC 8259 §7, plus the five short forms upstream
/// chooses to use. Bytes above 0x7f are copied verbatim: this conversion never
/// looks at whether the payload is valid UTF-8.
fn escape(out: &mut Vec<u8>, data: &[u8]) {
    for &c in data {
        match c {
            0x08 => out.extend_from_slice(b"\\b"),
            0x09 => out.extend_from_slice(b"\\t"),
            0x0a => out.extend_from_slice(b"\\n"),
            0x0c => out.extend_from_slice(b"\\f"),
            0x0d => out.extend_from_slice(b"\\r"),
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0x00..=0x1f => {
                let _ = write!(out, "\\u00{c:02x}");
            }
            _ => out.push(c),
        }
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Upstream's `generic_dump_base64` pads from a 65th character in the
/// alphabet. The URL-safe alphabet has only 64, so the "padding" it writes is
/// the string terminator and the result comes out unpadded — hence `pad`.
fn base64(data: &[u8], alphabet: &[u8; 64], pad: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len().div_ceil(3) * 4);
    for group in data.chunks(3) {
        let mut bits = 0u32;
        for (i, &b) in group.iter().enumerate() {
            bits |= u32::from(b) << (16 - 8 * i);
        }
        out.push(alphabet[(bits >> 18) as usize & 0x3f]);
        out.push(alphabet[(bits >> 12) as usize & 0x3f]);
        for (shift, present) in [(6, group.len() > 1), (0, group.len() > 2)] {
            if present {
                out.push(alphabet[(bits >> shift) as usize & 0x3f]);
            } else if pad {
                out.push(b'=');
            }
        }
    }
    out
}

fn base16(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 2);
    for &b in data {
        let _ = write!(out, "{b:02x}");
    }
    out
}
