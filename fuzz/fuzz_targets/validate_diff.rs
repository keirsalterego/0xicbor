//! Differential fuzzing of `cbor_value_validate` against upstream tinycbor.
//!
//! The third target, and the last public entry point that had no differential
//! coverage. `pretty_diff` and `json_diff` compare renderers; this one compares
//! a predicate, so there is no output to diff and the error code is the whole
//! answer. That makes it the strictest of the three: any disagreement at all is
//! a divergence, with no "the sinks legitimately hold different amounts of a
//! doomed render" exception to carve out.
//!
//! ## The flags come out of the input
//!
//! `CborValidationFlags` is a 27-bit matrix and almost every bit gates a
//! separate check: shortest-form integers and floats, sorted maps, unique keys,
//! tag use, UTF-8, no-undefined, no-tags, finite floats, unknown simple types,
//! unknown tags, complete data. Validating with the default flags exercises
//! almost none of it, so the first four bytes of the input are the bitmask and
//! the rest is the document.
//!
//! `CborValidateStrictest` is `~0U`, so a fuzzer that can produce any u32 can
//! reach every combination, including the ones upstream never names.

#![no_main]

use std::mem::MaybeUninit;
use std::os::raw::c_int;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use std::io::Write;

use libfuzzer_sys::fuzz_target;
use tinycbor::{CborParser, CborValue};

/// Built by `fuzz/oracle/build.sh`. Resolved at compile time so the harness
/// cannot silently fall back to some other binary on $PATH.
const ORACLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/oracle/cbor-oracle");

/// Bigger inputs cost a subprocess round trip without buying coverage.
const MAX_INPUT: usize = 16 * 1024;

extern "C" {
    fn cbor_parser_init(
        buffer: *const u8,
        size: usize,
        flags: u32,
        parser: *mut CborParser,
        it: *mut CborValue,
    ) -> c_int;
    fn cbor_value_validate(it: *const CborValue, flags: u32) -> c_int;
}

/// Runs the input through this port. Returns the `CborError`.
fn ours(data: &[u8], flags: u32) -> c_int {
    // An empty slice's `as_ptr()` is dangling; hand C a real address instead.
    let pad = [0u8; 1];
    let ptr = if data.is_empty() {
        pad.as_ptr()
    } else {
        data.as_ptr()
    };

    // SAFETY: `parser` and `value` are owned here and live across both calls,
    // and `cbor_parser_init` writes every field of both before anything reads
    // them.
    unsafe {
        let mut parser = MaybeUninit::<CborParser>::zeroed().assume_init();
        let mut value = MaybeUninit::<CborValue>::zeroed().assume_init();

        let err = cbor_parser_init(ptr, data.len(), 0, &mut parser, &mut value);
        if err != 0 {
            return err;
        }
        cbor_value_validate(&value, flags)
    }
}

/// Runs the input through the upstream C oracle, or `None` if the machine would
/// not run it right now.
///
/// `None` covers spawn and pipe failures only. This harness forks several
/// hundred processes a second and the kernel intermittently answers EAGAIN or
/// EMFILE under that load, which is a fact about the machine rather than a
/// difference between the two implementations.
fn theirs(data: &[u8], flags: u32) -> Option<c_int> {
    static PRESENT: OnceLock<()> = OnceLock::new();
    PRESENT.get_or_init(|| {
        assert!(
            std::path::Path::new(ORACLE).exists(),
            "no oracle at {ORACLE} -- run fuzz/oracle/build.sh"
        );
    });

    let mut child = Command::new(ORACLE)
        .arg("validate")
        .arg(flags.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    // Taking the handle closes it after the write, which is what makes the
    // oracle's fread() see EOF.
    let mut stdin = child.stdin.take().expect("piped");
    let written = stdin.write_all(data);
    drop(stdin);

    let out = child.wait_with_output().ok()?;
    written.ok()?;

    // No exit code means a signal killed it. Upstream faulting on an input is a
    // genuine finding, so this one is not swallowed.
    assert!(
        out.status.code().is_some(),
        "oracle died on a signal ({}) for input {} flags {flags}",
        out.status,
        hex(data)
    );

    let code = String::from_utf8_lossy(&out.stderr);
    let code = code
        .trim()
        .parse::<c_int>()
        .unwrap_or_else(|_| panic!("oracle wrote no error code, stderr was {code:?}"));
    Some(code)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fuzz_target!(|input: &[u8]| {
    // First four bytes are the flag matrix, the rest is the document.
    if input.len() < 4 {
        return;
    }
    let (head, data) = input.split_at(4);
    if data.len() > MAX_INPUT {
        return;
    }
    let flags = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);

    let our_err = ours(data, flags);
    let Some(their_err) = theirs(data, flags) else {
        return;
    };

    assert_eq!(
        our_err,
        their_err,
        "DIVERGENCE\n  input : {}\n  flags : {flags:#010x}\n  ours  : {our_err}\n  theirs: {their_err}",
        hex(data),
    );
});
