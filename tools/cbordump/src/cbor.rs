//! The slice-based CBOR reader that both output formats sit on.
//!
//! Upstream's parser is an incremental cursor that can also sit on a stream.
//! This one only ever sees a whole buffer, because `dumpFile` slurps the file
//! before parsing either way. What it does keep is upstream's error taxonomy
//! and, less obviously, *when* each error is raised — see [`Reader::preparse`].

use cbor_core::CborError;

/// `CBOR_PARSER_MAX_RECURSIONS`.
pub const MAX_RECURSIONS: i32 = 1024;

/// The stop code for indefinite-length items (RFC 8949 §3.2.1).
pub const BREAK: u8 = 0xff;

/// A decoded item head: major type, additional info, and the argument the
/// head carries (RFC 8949 §3). For major type 7 the argument doubles as the
/// raw bits of a float or the value of a simple type.
#[derive(Clone, Copy)]
pub struct Head {
    pub major: u8,
    pub ai: u8,
    pub value: u64,
    /// Bytes occupied by the head itself, 1 to 9.
    pub size: usize,
}

impl Head {
    pub fn indefinite(&self) -> bool {
        self.ai == 31
    }

    /// The `CborType` this head maps to, as the C enum spells it. Only the
    /// JSON converter needs these, and only to put them in its metadata.
    pub fn cbor_type(&self) -> u8 {
        match self.major {
            // Both integer major types are one CborType.
            1 => 0x00,
            7 => match self.ai {
                20 | 21 => 0xf5,
                22 => 0xf6,
                23 => 0xf7,
                25 => 0xf9,
                26 => 0xfa,
                27 => 0xfb,
                _ => 0xe0,
            },
            m => m << 5,
        }
    }
}

/// What upstream's parser does the moment an item is consumed.
///
/// `preparse_next_value` validates the head of the *following* item as part of
/// finishing the current one, unless the enclosing container has just run out.
/// Plain recursive descent would instead notice a malformed item when it got to
/// it — one closing bracket, one comma or one whole string later. Since both
/// tools write to stdout as they go, that difference is visible in what reaches
/// the terminal before the error does, so each item's conversion carries the
/// action to run at its end.
#[derive(Clone, Copy)]
pub enum After {
    /// Nothing follows this item inside its container.
    Stop,
    /// Another item follows; its head is checked now.
    Next,
    /// An indefinite container, where a break code is an ending and not an item.
    BreakOrNext,
}

impl After {
    pub fn apply(self, r: &Reader) -> Result<(), CborError> {
        match self {
            After::Stop => Ok(()),
            After::BreakOrNext if r.peek() == Some(BREAK) => Ok(()),
            After::Next | After::BreakOrNext => r.preparse(),
        }
    }
}

pub struct Reader<'a> {
    pub buf: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    pub fn peek(&self) -> Option<u8> {
        self.buf.get(self.pos).copied()
    }

    pub fn head(&self) -> Result<Head, CborError> {
        head_at(self.buf, self.pos)
    }

    /// Validates the head of the item after the current one.
    ///
    /// See [`After`] for why the timing of this matters.
    pub fn preparse(&self) -> Result<(), CborError> {
        self.head().map(|_| ())
    }

    /// `len` bytes at `self.pos`, advancing past them.
    pub fn take(&mut self, len: usize) -> Result<&'a [u8], CborError> {
        let end = self.pos.checked_add(len).ok_or(CborError::UnexpectedEOF)?;
        let out = self
            .buf
            .get(self.pos..end)
            .ok_or(CborError::UnexpectedEOF)?;
        self.pos = end;
        Ok(out)
    }
}

/// Decodes the head at `pos` without consuming it.
pub fn head_at(buf: &[u8], pos: usize) -> Result<Head, CborError> {
    let descriptor = *buf.get(pos).ok_or(CborError::UnexpectedEOF)?;
    let major = descriptor >> 5;
    let ai = descriptor & 0x1f;

    let count = match ai {
        0..=23 => {
            return Ok(Head {
                major,
                ai,
                value: u64::from(ai),
                size: 1,
            })
        }
        24..=27 => 1usize << (ai - 24),
        // Only the four types that carry a payload can be indefinite. On the
        // rest the same encoding is the break code, which is a stray
        // terminator in major type 7 and simply undefined elsewhere.
        31 => {
            return match major {
                2..=5 => Ok(Head {
                    major,
                    ai,
                    value: 0,
                    size: 1,
                }),
                7 => Err(CborError::UnexpectedBreak),
                _ => Err(CborError::IllegalNumber),
            }
        }
        // 28, 29 and 30 are reserved for future versions of CBOR.
        _ => {
            return Err(if major == 7 {
                CborError::UnknownType
            } else {
                CborError::IllegalNumber
            })
        }
    };

    let bytes = buf
        .get(pos + 1..pos + 1 + count)
        .ok_or(CborError::UnexpectedEOF)?;
    let value = bytes.iter().fold(0u64, |acc, &b| acc << 8 | u64::from(b));

    // A two-byte simple value must not repeat something the one-byte form
    // could have said (RFC 8949 §3.3).
    if major == 7 && ai == 24 && value < 32 {
        return Err(CborError::IllegalSimpleType);
    }
    Ok(Head {
        major,
        ai,
        value,
        size: 1 + count,
    })
}

/// The head of the next chunk of an indefinite-length string: its size and the
/// payload length, or `None` at the break code, which is left in place for the
/// caller — upstream consumes it in `finish_string_iteration`.
pub fn chunk_head(buf: &[u8], pos: usize, major: u8) -> Result<Option<(usize, usize)>, CborError> {
    let descriptor = *buf.get(pos).ok_or(CborError::UnexpectedEOF)?;
    if descriptor == BREAK {
        return Ok(None);
    }
    if descriptor >> 5 != major {
        return Err(CborError::IllegalType);
    }
    let ai = descriptor & 0x1f;
    if ai < 24 {
        return Ok(Some((1, usize::from(ai))));
    }
    // Note this rejects 31 as well: a chunk may not itself be chunked, and
    // upstream reports that as an illegal number rather than an illegal type.
    if ai > 27 {
        return Err(CborError::IllegalNumber);
    }
    let count = 1usize << (ai - 24);
    let bytes = buf
        .get(pos + 1..pos + 1 + count)
        .ok_or(CborError::UnexpectedEOF)?;
    let value = bytes.iter().fold(0u64, |acc, &b| acc << 8 | u64::from(b));
    let len = usize::try_from(value).map_err(|_| CborError::DataTooLarge)?;
    Ok(Some((1 + count, len)))
}

/// Reads a byte or text string whole, joining the chunks of an
/// indefinite-length one. Both conversions that use this discard the chunk
/// boundaries anyway.
pub fn read_string(r: &mut Reader) -> Result<Vec<u8>, CborError> {
    let h = r.head()?;
    r.pos += h.size;
    if !h.indefinite() {
        let len = usize::try_from(h.value).map_err(|_| CborError::DataTooLarge)?;
        return Ok(r.take(len)?.to_vec());
    }
    let mut out = Vec::new();
    while let Some((head_len, len)) = chunk_head(r.buf, r.pos, h.major)? {
        r.pos += head_len;
        out.extend_from_slice(r.take(len)?);
    }
    r.pos += 1; // the break code
    Ok(out)
}

/// Steps over one item, contents and all: `cbor_value_advance`.
///
/// Only the recursion-limit path needs this, which is why it produces no text.
pub fn skip(r: &mut Reader, levels_left: i32) -> Result<(), CborError> {
    let h = r.head()?;
    match h.major {
        2 | 3 => {
            read_string(r)?;
        }
        4 | 5 => {
            if levels_left == 0 {
                return Err(CborError::NestingTooDeep);
            }
            r.pos += h.size;
            let count = enter(r, &h)?;
            if h.indefinite() {
                while r.peek() != Some(BREAK) {
                    skip(r, levels_left - 1)?;
                }
                r.pos += 1;
            } else {
                for _ in 0..count {
                    skip(r, levels_left - 1)?;
                }
            }
        }
        // A tag and the item it tags are two items to upstream's iterator, but
        // skipping one has to skip both to land on the right byte.
        6 => {
            r.pos += h.size;
            skip(r, levels_left)?;
        }
        _ => r.pos += h.size,
    }
    Ok(())
}

/// Steps into an array or map, returning how many items it holds — twice the
/// pair count for a map, and `u64::MAX` for an indefinite one.
///
/// The head must already have been consumed. Upstream's `enter_container`
/// keeps its item counter in a `uint32_t`, and rejects rather than truncates
/// anything that will not fit; those two rejections are the `DataTooLarge`s.
pub fn enter(r: &Reader, h: &Head) -> Result<u64, CborError> {
    if h.indefinite() {
        if r.peek() != Some(BREAK) {
            r.preparse()?;
        }
        return Ok(u64::MAX);
    }
    if h.value >= u64::from(u32::MAX) {
        return Err(CborError::DataTooLarge);
    }
    let is_map = h.major == 5;
    if is_map && h.value > u64::from(u32::MAX / 2) {
        return Err(CborError::DataTooLarge);
    }
    let count = if is_map { h.value * 2 } else { h.value };
    if count > 0 {
        r.preparse()?;
    }
    Ok(count)
}
