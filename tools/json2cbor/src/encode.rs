//! Turning the parsed JSON into CBOR, ported from tools/json2cbor/json2cbor.c.
//!
//! The encoder writes into a buffer of a fixed size, as upstream's does: it
//! reuses the buffer the JSON text was read into, on the grounds that CBOR is
//! never longer than the JSON it came from. That is true of everything except
//! floating-point numbers, which is why only they can grow it — and why
//! running out of room is a reachable, reportable error rather than an
//! impossibility.

use crate::json::{Member, Value};
use cbor_core::encoder::{head, major, BREAK, INDEFINITE};
use cbor_core::CborError;

/// The suffix cbordump puts on the key holding a value's metadata.
const MARKER: &[u8] = b"$cbor";

/// `CborType` values that metadata can name.
const TYPE_INTEGER: i32 = 0x00;
const TYPE_BYTE_STRING: i32 = 0x40;
const TYPE_SIMPLE: i32 = 0xe0;
const TYPE_UNDEFINED: i32 = 0xf7;
const TYPE_HALF_FLOAT: i32 = 0xf9;
const TYPE_FLOAT: i32 = 0xfa;
const TYPE_DOUBLE: i32 = 0xfb;
const TYPE_INVALID: i32 = 0xff;

const NEGATIVE_BIGNUM: u64 = 3;
const EXPECTED_BASE64: u64 = 22;
const EXPECTED_BASE16: u64 = 23;

pub struct Encoder {
    pub out: Vec<u8>,
    limit: usize,
}

impl Encoder {
    pub fn new(limit: usize) -> Self {
        Encoder {
            out: Vec::new(),
            limit,
        }
    }

    fn append(&mut self, bytes: &[u8]) -> Result<(), CborError> {
        if self.out.len() + bytes.len() > self.limit {
            return Err(CborError::OutOfMemory);
        }
        self.out.extend_from_slice(bytes);
        Ok(())
    }

    fn head(&mut self, major: u8, value: u64) -> Result<(), CborError> {
        let (bytes, len) = head(major, value);
        self.append(&bytes[..len])
    }

    fn string(&mut self, major: u8, body: &[u8]) -> Result<(), CborError> {
        self.head(major, body.len() as u64)?;
        self.append(body)
    }

    /// `cbor_encode_int`: the sign bit picks the major type and complements the
    /// magnitude, which is exactly CBOR's -1-n representation (RFC 8949 §3.1).
    fn int(&mut self, value: i64) -> Result<(), CborError> {
        let sign = (value >> 63) as u64;
        self.head((sign & 0x20) as u8, sign ^ value as u64)
    }

    fn simple(&mut self, value: u8) -> Result<(), CborError> {
        // 25 to 31 are the float encodings and the break code; a simple value
        // may not claim one of them.
        if (25..=31).contains(&value) {
            return Err(CborError::IllegalSimpleType);
        }
        self.head(major::SIMPLE, u64::from(value))
    }

    /// Head and payload go in one append, so that running out of room leaves
    /// nothing behind — `grow_and_encode_double` relies on being able to retry.
    fn floating_point(&mut self, additional_info: u8, payload: &[u8]) -> Result<(), CborError> {
        let mut item = vec![major::SIMPLE | additional_info];
        item.extend_from_slice(payload);
        self.append(&item)
    }

    fn double(&mut self, value: f64) -> Result<(), CborError> {
        self.floating_point(27, &value.to_bits().to_be_bytes())
    }

    fn float(&mut self, value: f32) -> Result<(), CborError> {
        self.floating_point(26, &value.to_bits().to_be_bytes())
    }

    fn half(&mut self, value: u16) -> Result<(), CborError> {
        self.floating_point(25, &value.to_be_bytes())
    }
}

pub fn encode(enc: &mut Encoder, value: &Value, metadata: bool) -> Result<(), CborError> {
    match value {
        Value::Bool(b) => enc.simple(20 + u8::from(*b)),
        Value::Null => enc.simple(22),
        Value::Number { double, int } => {
            // An integer only stays one if the double round-trips through
            // cJSON's `int` field, so anything past INT_MAX becomes a double.
            if f64::from(*int) == *double {
                enc.int(i64::from(*int))
            } else {
                grow_and_encode_double(enc, *double)
            }
        }
        Value::Str(s) => enc.string(major::TEXT_STRING, s),

        Value::Array(items) => {
            let count = size_limited(items.len());
            open(enc, major::ARRAY, count)?;
            for item in items {
                encode(enc, item, metadata)?;
            }
            close(enc, count)
        }

        Value::Object(members) => {
            // With metadata in play the map has to be indefinite: the count
            // below would include the metadata keys, which are not emitted.
            let count = if metadata {
                None
            } else {
                size_limited(members.len())
            };
            open(enc, major::MAP, count)?;
            for member in members {
                if metadata && is_metadata_key(&member.name) {
                    continue;
                }
                enc.string(major::TEXT_STRING, &member.name)?;
                if !metadata {
                    encode(enc, &member.value, metadata)?;
                    continue;
                }
                let md = parse_metadata(find_metadata(members, &member.name));
                if md.tagged {
                    enc.head(major::TAG, md.tag)?;
                }
                encode_with_metadata(enc, &member.value, &md, metadata)?;
            }
            close(enc, count)
        }
    }
}

/// The item count for a container, or `None` for an indefinite one.
///
/// Upstream stops counting at 256 because walking a cJSON list is O(n) and it
/// would rather emit an indefinite-length container than walk it twice.
fn size_limited(len: usize) -> Option<usize> {
    (len <= 255).then_some(len)
}

fn open(enc: &mut Encoder, major: u8, count: Option<usize>) -> Result<(), CborError> {
    match count {
        Some(n) => enc.head(major, n as u64),
        None => enc.append(&[major | INDEFINITE]),
    }
}

fn close(enc: &mut Encoder, count: Option<usize>) -> Result<(), CborError> {
    // Upstream also checks that as many items were written as were promised.
    // Here the promise came from the same list that was just walked, so it
    // cannot be broken.
    match count {
        Some(_) => Ok(()),
        None => enc.append(&[BREAK]),
    }
}

fn grow_and_encode_double(enc: &mut Encoder, value: f64) -> Result<(), CborError> {
    loop {
        match enc.double(value) {
            Err(CborError::OutOfMemory) => enc.limit += 1024,
            other => return other,
        }
    }
}

fn is_metadata_key(name: &[u8]) -> bool {
    // A key of exactly "$cbor" is a real key, not a marker.
    name.len() > MARKER.len() && name.ends_with(MARKER)
}

/// The `<key>$cbor` member holding `name`'s metadata.
///
/// The lookup is `cJSON_GetObjectItem`, which compares case-insensitively, so
/// `"a$CBOR"` describes `"a"` just as well as `"a$cbor"` does.
fn find_metadata<'a>(members: &'a [Member], name: &[u8]) -> Option<&'a Value> {
    let mut wanted = name.to_vec();
    wanted.extend_from_slice(MARKER);
    members
        .iter()
        .find(|m| m.name.eq_ignore_ascii_case(&wanted))
        .map(|m| &m.value)
}

/// What cbordump recorded about a value that JSON could not carry.
struct MetaData {
    tag: u64,
    value: MetaValue,
    /// The `CborType` from `"t"`, kept as the `int` cJSON produced so that an
    /// out-of-range one can be reported the way upstream reports it.
    t: i32,
    tagged: bool,
}

/// The `"v"` field. Upstream overlays a `const char *` and a `uint8_t` in a
/// union, so which one is meaningful depends on the JSON type that was found.
enum MetaValue {
    Absent,
    Str(Vec<u8>),
    Simple(u8),
}

fn parse_metadata(md: Option<&Value>) -> MetaData {
    let mut result = MetaData {
        tag: 0,
        value: MetaValue::Absent,
        t: TYPE_INVALID,
        tagged: false,
    };
    let Some(Value::Object(members)) = md else {
        return result;
    };

    for m in members {
        match m.name.as_slice() {
            b"tag" => match &m.value {
                Value::Str(s) => match scan_u64(s, 10) {
                    // sscanf reports end-of-input as EOF and a subject it could
                    // not convert as zero conversions, and only the first of
                    // those counts as a failure here. So a tag of "nonsense"
                    // is "tagged" with whatever the tag already was: zero.
                    Scan::Eof => eprintln!("json2cbor: could not parse tag: {}", show(s)),
                    Scan::NoMatch => result.tagged = true,
                    Scan::Value(v) => {
                        result.tag = v;
                        result.tagged = true;
                    }
                },
                // Upstream prints the valuestring of a non-string, which is a
                // null pointer, and glibc renders that as "(null)".
                _ => eprintln!("json2cbor: could not parse tag: (null)"),
            },
            b"t" => result.t = value_int(&m.value),
            b"v" => {
                result.value = match &m.value {
                    Value::Number { int, .. } => MetaValue::Simple(*int as u8),
                    Value::Str(s) => MetaValue::Str(s.clone()),
                    _ => MetaValue::Absent,
                }
            }
            _ => {}
        }
    }
    result
}

/// cJSON's `valueint`, which is zero for everything that is not a number —
/// except `true`, which the parser sets to one.
fn value_int(value: &Value) -> i32 {
    match value {
        Value::Number { int, .. } => *int,
        Value::Bool(true) => 1,
        _ => 0,
    }
}

fn encode_with_metadata(
    enc: &mut Encoder,
    item: &Value,
    md: &MetaData,
    metadata: bool,
) -> Result<(), CborError> {
    match md.t {
        // An integer with more than 53 bits of precision, written as a sign
        // and a hexadecimal magnitude.
        TYPE_INTEGER => {
            if let MetaValue::Str(v) = &md.value {
                // Anything but a leading "+" means negative, including a
                // magnitude with no sign in front of it at all.
                let positive = v.first() == Some(&b'+');
                let rest = v.get(1..).unwrap_or_default();
                let magnitude = match scan_u64(rest, 16) {
                    Scan::Eof => {
                        eprintln!("json2cbor: could not parse number: {}", show(rest));
                        None
                    }
                    // sscanf leaves its destination alone when it converts
                    // nothing, and upstream never initialised it.
                    Scan::NoMatch => Some(0),
                    Scan::Value(n) => Some(n),
                };
                if let Some(n) = magnitude {
                    return if positive {
                        enc.head(major::UNSIGNED, n)
                    } else {
                        // cbor_encode_negative_int is given the magnitude and
                        // stores n-1 (RFC 8949 §3.1).
                        enc.head(major::NEGATIVE, n.wrapping_sub(1))
                    };
                }
            } else {
                // Upstream reads the missing string through a null pointer.
                eprintln!("json2cbor: could not parse number: (null)");
            }
        }

        TYPE_BYTE_STRING => {
            if let Value::Str(s) = item {
                let decoded = match md.tag {
                    EXPECTED_BASE64 => decode_base64(s, BASE64),
                    EXPECTED_BASE16 => decode_base16(s),
                    // The "~" that marks a negative bignum is not payload.
                    NEGATIVE_BIGNUM => decode_base64(s.get(1..).unwrap_or_default(), BASE64URL),
                    _ => decode_base64(s, BASE64URL),
                };
                match decoded {
                    Some(bytes) => return enc.string(major::BYTE_STRING, &bytes),
                    None => eprintln!(
                        "json2cbor: could not decode encoded byte string: {}",
                        show(s)
                    ),
                }
            } else {
                eprintln!("json2cbor: could not decode encoded byte string: (null)");
            }
        }

        TYPE_SIMPLE => {
            let value = match md.value {
                MetaValue::Simple(n) => n,
                // The union again: with no number in "v" the byte upstream
                // reads is either the zero it was initialised to or part of a
                // string pointer. Zero for both.
                _ => 0,
            };
            return enc.simple(value);
        }

        TYPE_UNDEFINED => return enc.simple(23),

        TYPE_HALF_FLOAT | TYPE_FLOAT | TYPE_DOUBLE => {
            let value = match &md.value {
                MetaValue::Str(s) if s == b"nan" => f64::NAN,
                MetaValue::Str(s) if s == b"-inf" => f64::NEG_INFINITY,
                MetaValue::Str(s) if s == b"inf" => f64::INFINITY,
                MetaValue::Str(s) => {
                    eprintln!("json2cbor: invalid floating-point value: {}", show(s));
                    return encode(enc, item, metadata);
                }
                // A number in "v" leaves upstream with a pointer that is not
                // one and it dereferences it; treated here as no "v" at all.
                MetaValue::Absent | MetaValue::Simple(_) => match item {
                    Value::Number { double, .. } => *double,
                    _ => 0.0,
                },
            };
            return match md.t {
                TYPE_DOUBLE => enc.double(value),
                TYPE_FLOAT => enc.float(value as f32),
                _ => enc.half(cbor_core::half::encode(value as f32)),
            };
        }

        TYPE_INVALID => {}
        other => eprintln!("json2cbor: invalid CBOR type: {other}"),
    }

    encode(enc, item, metadata)
}

/// What `sscanf` would report for one integer conversion.
enum Scan {
    /// Nothing to convert: end of input reached first.
    Eof,
    /// Something was there but it was not a number.
    NoMatch,
    Value(u64),
}

/// One `%llu` or `%llx` conversion, with `strtoull`'s saturation on overflow.
fn scan_u64(input: &[u8], radix: u32) -> Scan {
    let mut i = 0;
    while input.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
        i += 1;
    }
    if i == input.len() {
        return Scan::Eof;
    }
    let negate = match input[i] {
        b'-' => {
            i += 1;
            true
        }
        b'+' => {
            i += 1;
            false
        }
        _ => false,
    };
    let digits: Vec<u8> = input[i..]
        .iter()
        .copied()
        .take_while(|c| char::from(*c).is_digit(radix))
        .collect();
    if digits.is_empty() {
        return Scan::NoMatch;
    }
    let text = String::from_utf8_lossy(&digits);
    let value = u64::from_str_radix(&text, radix).unwrap_or(u64::MAX);
    Scan::Value(if negate { value.wrapping_neg() } else { value })
}

/// A byte string as `printf("%s")` would show it.
fn show(s: &[u8]) -> String {
    String::from_utf8_lossy(s).into_owned()
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const BASE64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// `decode_base64_generic`, translated as it stands.
///
/// It writes three bytes for every group it reads, including the short final
/// one, so a truncated group decodes to three bytes rather than one or two —
/// two of which are not the ones base64 says they are. That is upstream's
/// output for such input, and cbordump never produces such input.
fn decode_base64(input: &[u8], alphabet: &[u8; 64]) -> Option<Vec<u8>> {
    let mut reverse = [-1i32; 256];
    for (i, &c) in alphabet.iter().enumerate() {
        reverse[usize::from(c)] = i as i32;
    }
    // Past the end is the C string's terminator.
    let at = |i: usize| -> u8 { input.get(i).copied().unwrap_or(0) };

    let mut out = Vec::new();
    let mut i = 0;
    let mut done = false;
    loop {
        if reverse[usize::from(at(i))] < 0 || reverse[usize::from(at(i + 1))] < 0 {
            done = at(i) == 0;
            break;
        }
        let sextet = |c: u8| reverse[usize::from(c)] as u32;
        let mut bits = sextet(at(i)) << 18 | sextet(at(i + 1)) << 12;

        if at(i + 2) == b'=' || at(i + 2) == 0 {
            if at(i + 2) == b'=' && (at(i + 3) != b'=' || at(i + 4) != 0) {
                break;
            }
            bits >>= 12;
            done = true;
        } else if at(i + 3) == b'=' || at(i + 3) == 0 {
            if at(i + 3) == b'=' && at(i + 4) != 0 {
                break;
            }
            bits >>= 6;
            bits |= sextet(at(i + 2));
            done = true;
        } else {
            bits |= sextet(at(i + 2)) << 6;
            bits |= sextet(at(i + 3));
        }

        out.push((bits >> 16) as u8);
        out.push((bits >> 8) as u8);
        out.push(bits as u8);
        i += 4;
        if done {
            break;
        }
    }
    done.then_some(out)
}

fn decode_base16(input: &[u8]) -> Option<Vec<u8>> {
    // An odd trailing digit is ignored, as in upstream's strlen(string) / 2.
    let mut out = Vec::with_capacity(input.len() / 2);
    for pair in input.chunks_exact(2) {
        let hi = char::from(pair[0]).to_digit(16)?;
        let lo = char::from(pair[1]).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}
