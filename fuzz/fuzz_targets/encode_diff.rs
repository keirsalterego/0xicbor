//! Differential fuzzing of the encoder against upstream tinycbor.
//!
//! The other three targets read CBOR. This one writes it, which is the half of
//! the library nothing differential had reached, for a reason that looks like a
//! good one until you look twice: an encoder takes calls rather than bytes, so
//! there is no input to hand it.
//!
//! So the fuzzer's bytes become the calls. Each input is a little program — a
//! two-byte output buffer size, then a stream of opcodes — run against both
//! implementations, and what gets compared is the error from every call, the
//! bytes each one wrote, and how much more room each says it needed.
//!
//! ## The buffer size is the interesting operand
//!
//! Upstream's encoder does not stop when it runs out of room. It switches its
//! union from a write pointer to a byte counter, keeps walking the calls, and
//! answers `CborErrorOutOfMemory` while accumulating what a big enough buffer
//! would have taken. That bookkeeping is most of the non-obvious code in
//! `cborencoder.c`, and it is unreachable with a buffer that always fits — so
//! the size comes out of the input, small, and most programs overrun.
//!
//! ## Two interpreters
//!
//! The program format is specified once, in the comment above
//! `run_encoder_program` in `fuzz/oracle/cbor-oracle.c`, and implemented twice.
//! They have to agree about the program before they can disagree about the
//! encoder, so an early divergence is far more likely to be the two readers
//! than the two encoders.

#![no_main]

use std::mem::MaybeUninit;
use std::os::raw::{c_char, c_int, c_void};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use std::io::Write;

use libfuzzer_sys::fuzz_target;
use tinycbor::CborEncoder;

/// Built by `fuzz/oracle/build.sh`. Resolved at compile time so the harness
/// cannot silently fall back to some other binary on $PATH.
const ORACLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/oracle/cbor-oracle");

/// Bigger programs cost a subprocess round trip without buying coverage.
const MAX_INPUT: usize = 16 * 1024;

/// Mirrors the oracle's `ENC_BUF_MAX`, `ENC_MAX_DEPTH` and `ENC_OPS`.
const BUF_MAX: usize = 1024;
const MAX_DEPTH: usize = 16;
const OPS: u8 = 19;

/// `CborIndefiniteLength`.
const INDEFINITE: usize = usize::MAX;

/// `CborType` values the floating-point entry point dispatches on.
const TYPE_HALF_FLOAT: c_int = 0xf9;
const TYPE_FLOAT: c_int = 0xfa;
const TYPE_DOUBLE: c_int = 0xfb;

/// `CborSimpleType` values behind the inline boolean/null/undefined helpers.
const SIMPLE_FALSE: u8 = 20;
const SIMPLE_NULL: u8 = 22;
const SIMPLE_UNDEFINED: u8 = 23;

extern "C" {
    fn cbor_encoder_init(e: *mut CborEncoder, buffer: *mut u8, size: usize, flags: c_int);
    fn cbor_encode_uint(e: *mut CborEncoder, value: u64) -> c_int;
    fn cbor_encode_int(e: *mut CborEncoder, value: i64) -> c_int;
    fn cbor_encode_negative_int(e: *mut CborEncoder, absolute_value: u64) -> c_int;
    fn cbor_encode_simple_value(e: *mut CborEncoder, value: u8) -> c_int;
    fn cbor_encode_tag(e: *mut CborEncoder, tag: u64) -> c_int;
    fn cbor_encode_text_string(e: *mut CborEncoder, s: *const c_char, len: usize) -> c_int;
    fn cbor_encode_byte_string(e: *mut CborEncoder, s: *const u8, len: usize) -> c_int;
    fn cbor_encode_floating_point(e: *mut CborEncoder, ty: c_int, value: *const c_void) -> c_int;
    fn cbor_encode_float_as_half_float(e: *mut CborEncoder, value: f32) -> c_int;
    fn cbor_encode_raw(e: *mut CborEncoder, raw: *const u8, len: usize) -> c_int;
    fn cbor_encoder_create_array(p: *mut CborEncoder, c: *mut CborEncoder, len: usize) -> c_int;
    fn cbor_encoder_create_map(p: *mut CborEncoder, c: *mut CborEncoder, len: usize) -> c_int;
    fn cbor_encoder_close_container(p: *mut CborEncoder, c: *const CborEncoder) -> c_int;
    fn cbor_encoder_close_container_checked(p: *mut CborEncoder, c: *const CborEncoder) -> c_int;
}

/// `cbor_encoder_get_extra_bytes_needed`, which is `static inline` in `cbor.h`
/// and so compiles into the caller rather than into the library. There is no
/// symbol to link against; reading the two fields is what the C does, and is
/// itself a check that the layout is right.
///
/// SAFETY: `e` must point at an initialised encoder.
unsafe fn extra_bytes_needed(e: &CborEncoder) -> usize {
    if e.end.is_null() {
        // The union's other arm: a count, not an address.
        e.data.0 as usize
    } else {
        0
    }
}

/// Walks `prog` handing out operand slices, and reports when one runs off the
/// end so the program stops exactly where the oracle's stops.
struct Program<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Program<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let out = self.bytes.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(out)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }

    fn u64(&mut self) -> Option<u64> {
        self.take(8)
            .map(|b| u64::from_le_bytes(b.try_into().expect("8 bytes")))
    }
}

/// Runs the program through this port, returning the transcript and the last
/// call's error.
fn ours(prog: &[u8]) -> (Vec<u8>, c_int) {
    if prog.len() < 2 {
        return (Vec::new(), 0);
    }
    let bufsize = (usize::from(u16::from_le_bytes([prog[0], prog[1]]))) % (BUF_MAX + 1);
    let mut outbuf = vec![0u8; BUF_MAX];
    let mut transcript = Vec::new();
    let mut p = Program {
        bytes: prog,
        pos: 2,
    };
    let mut err: c_int = 0;

    // SAFETY: every encoder in `stack` is written by `cbor_encoder_init` or by
    // a `create_*` call before anything reads it, `outbuf` outlives all of them,
    // and `depth` is kept inside the array.
    unsafe {
        let mut stack: Vec<CborEncoder> = (0..=MAX_DEPTH)
            .map(|_| MaybeUninit::<CborEncoder>::zeroed().assume_init())
            .collect();
        let mut depth = 0usize;
        cbor_encoder_init(&mut stack[0], outbuf.as_mut_ptr(), bufsize, 0);

        while let Some(opcode) = p.u8() {
            let cur: *mut CborEncoder = &mut stack[depth];
            match opcode % OPS {
                0 => {
                    let Some(v) = p.u64() else { break };
                    err = cbor_encode_uint(cur, v);
                }
                1 => {
                    let Some(v) = p.u64() else { break };
                    err = cbor_encode_int(cur, v as i64);
                }
                2 => {
                    let Some(v) = p.u64() else { break };
                    err = cbor_encode_negative_int(cur, v);
                }
                3 => {
                    let Some(v) = p.u8() else { break };
                    err = cbor_encode_simple_value(cur, v);
                }
                4 => {
                    let Some(v) = p.u64() else { break };
                    err = cbor_encode_tag(cur, v);
                }
                op @ (5 | 6) => {
                    let Some(len) = p.u8() else { break };
                    let Some(s) = p.take(usize::from(len)) else {
                        break;
                    };
                    err = if op == 5 {
                        cbor_encode_text_string(cur, s.as_ptr() as *const c_char, s.len())
                    } else {
                        cbor_encode_byte_string(cur, s.as_ptr(), s.len())
                    };
                }
                7 => {
                    let Some(b) = p.take(4) else { break };
                    let f = f32::from_le_bytes(b.try_into().expect("4 bytes"));
                    err = cbor_encode_floating_point(
                        cur,
                        TYPE_FLOAT,
                        &f as *const f32 as *const c_void,
                    );
                }
                8 => {
                    let Some(b) = p.take(8) else { break };
                    let d = f64::from_le_bytes(b.try_into().expect("8 bytes"));
                    err = cbor_encode_floating_point(
                        cur,
                        TYPE_DOUBLE,
                        &d as *const f64 as *const c_void,
                    );
                }
                9 => {
                    let Some(b) = p.take(2) else { break };
                    let h = u16::from_le_bytes(b.try_into().expect("2 bytes"));
                    err = cbor_encode_floating_point(
                        cur,
                        TYPE_HALF_FLOAT,
                        &h as *const u16 as *const c_void,
                    );
                }
                10 => {
                    let Some(b) = p.take(4) else { break };
                    let f = f32::from_le_bytes(b.try_into().expect("4 bytes"));
                    err = cbor_encode_float_as_half_float(cur, f);
                }
                11 => {
                    let Some(len) = p.u8() else { break };
                    let Some(s) = p.take(usize::from(len)) else {
                        break;
                    };
                    err = cbor_encode_raw(cur, s.as_ptr(), s.len());
                }
                op @ (12 | 13) => {
                    if depth == MAX_DEPTH {
                        // The oracle steps over the operand without reading it,
                        // so this has to leave the cursor in the same place.
                        p.pos += 1;
                        continue;
                    }
                    let Some(len) = p.u8() else { break };
                    let n = if len == 0xff {
                        INDEFINITE
                    } else {
                        usize::from(len)
                    };
                    let child: *mut CborEncoder = &mut stack[depth + 1];
                    err = if op == 12 {
                        cbor_encoder_create_array(cur, child, n)
                    } else {
                        cbor_encoder_create_map(cur, child, n)
                    };
                    depth += 1;
                }
                op @ (14 | 15) => {
                    if depth == 0 {
                        continue;
                    }
                    let parent: *mut CborEncoder = &mut stack[depth - 1];
                    let child: *const CborEncoder = &stack[depth];
                    err = if op == 14 {
                        cbor_encoder_close_container(parent, child)
                    } else {
                        cbor_encoder_close_container_checked(parent, child)
                    };
                    depth -= 1;
                }
                16 => {
                    let Some(v) = p.u8() else { break };
                    err = cbor_encode_simple_value(cur, SIMPLE_FALSE + (v & 1));
                }
                17 => err = cbor_encode_simple_value(cur, SIMPLE_NULL),
                _ => err = cbor_encode_simple_value(cur, SIMPLE_UNDEFINED),
            }
            transcript.extend_from_slice(&err.to_le_bytes());
        }

        let needed = extra_bytes_needed(&stack[0]);
        transcript.extend_from_slice(&outbuf[..bufsize]);
        transcript.extend_from_slice(&(needed.min(0xffff_ffff) as u32).to_le_bytes());
    }

    (transcript, err)
}

/// Runs the same program through the upstream C oracle, or `None` if the
/// machine would not run it right now.
///
/// `None` covers spawn and pipe failures only. This harness forks several
/// hundred processes a second and the kernel intermittently answers EAGAIN or
/// EMFILE under that load, which is a fact about the machine rather than a
/// difference between the two implementations.
fn theirs(prog: &[u8]) -> Option<(Vec<u8>, c_int)> {
    static PRESENT: OnceLock<()> = OnceLock::new();
    PRESENT.get_or_init(|| {
        assert!(
            std::path::Path::new(ORACLE).exists(),
            "no oracle at {ORACLE} -- run fuzz/oracle/build.sh"
        );
    });

    let mut child = Command::new(ORACLE)
        .arg("encode")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    // Taking the handle closes it after the write, which is what makes the
    // oracle's fread() see EOF.
    let mut stdin = child.stdin.take().expect("piped");
    let written = stdin.write_all(prog);
    drop(stdin);

    let out = child.wait_with_output().ok()?;
    written.ok()?;

    // No exit code means a signal killed it. Upstream faulting on a program is
    // a genuine finding, so this one is not swallowed.
    assert!(
        out.status.code().is_some(),
        "oracle died on a signal ({}) for program {}",
        out.status,
        hex(prog)
    );

    let code = String::from_utf8_lossy(&out.stderr);
    let code = code
        .trim()
        .parse::<c_int>()
        .unwrap_or_else(|_| panic!("oracle wrote no error code, stderr was {code:?}"));
    Some((out.stdout, code))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Names where two transcripts part company, so a failure points at an opcode
/// rather than at two screens of hex.
fn first_difference(a: &[u8], b: &[u8]) -> String {
    match a.iter().zip(b).position(|(x, y)| x != y) {
        Some(i) => format!("byte {i} of {} / {}", a.len(), b.len()),
        None => format!("lengths {} and {}", a.len(), b.len()),
    }
}

fuzz_target!(|input: &[u8]| {
    if input.len() > MAX_INPUT {
        return;
    }

    let (our_out, our_err) = ours(input);
    let Some((their_out, their_err)) = theirs(input) else {
        return;
    };

    assert_eq!(
        our_err,
        their_err,
        "DIVERGENCE (final error)\n  program: {}\n  ours   : {our_err}\n  theirs : {their_err}",
        hex(input),
    );
    assert!(
        our_out == their_out,
        "DIVERGENCE (transcript)\n  program: {}\n  at     : {}\n  ours   : {}\n  theirs : {}",
        hex(input),
        first_difference(&our_out, &their_out),
        hex(&our_out),
        hex(&their_out),
    );
});
