//! Differential fuzzing of the JSON converter against upstream tinycbor.
//!
//! Same shape as `pretty_diff`, pointed at the other output path. The port runs
//! `cbor_value_to_json_advance` through the C entry points it ships;
//! `oracle/cbor-oracle json FLAGS` runs upstream's in a separate process, fed
//! the same bytes on stdin. Nothing links the two together but a pipe, which is
//! what keeps "the shipped artifact contains no C" checkable rather than
//! asserted.
//!
//! ## Why this target exists separately
//!
//! The pretty printer and the JSON converter share a parser and almost nothing
//! else. JSON has to reject what diagnostic notation is happy to render: a map
//! key that is not a string, an integer too large for a double, a byte string
//! with no tag saying how to encode it. It also has its own escaping rules,
//! its own base64 and base16 encoders, and a `$cbor` metadata sidecar. None of
//! that is reached by fuzzing `cbor_value_to_pretty_advance`.
//!
//! ## The flags come out of the input
//!
//! `CborToJsonFlags` changes the output substantially -- tags become objects,
//! byte strings become base64url, map keys get stringified -- so a fuzzer that
//! only ever passed the default would leave most of the converter dark. The
//! first byte of the input selects the bitmask and the rest is the CBOR. Both
//! sides get the same value, so a divergence is always about the conversion.
//!
//! ## What is compared
//!
//! The error code always, exactly. stdout whenever the conversion succeeded.
//!
//! stdout is not compared on the error paths, for the same reason as the pretty
//! target: upstream streams its output as it walks and leaves a partial prefix
//! behind when a conversion fails partway, while this port renders into a
//! buffer and emits nothing at all. Both agree on every byte of a successful
//! conversion and on every error code.

#![no_main]

use std::ffi::c_void;
use std::io::Write;
use std::mem::MaybeUninit;
use std::os::raw::{c_char, c_int};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use tinycbor::{CborParser, CborValue};

/// Built by `fuzz/oracle/build.sh`. Resolved at compile time so the harness
/// cannot silently fall back to some other binary on $PATH.
const ORACLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/oracle/cbor-oracle");

/// Bigger inputs cost a subprocess round trip without buying coverage.
const MAX_INPUT: usize = 16 * 1024;

/// Every bit `CborToJsonFlags` defines: AddMetadata, TagsToObjects,
/// ByteStringsToBase64Url, StringifyMapKeys. The rest of the byte is ignored so
/// that flipping it does not look like a new input to the corpus.
const FLAG_BITS: c_int = 0b1111;

extern "C" {
    // The port, via the C ABI it ships.
    fn cbor_parser_init(
        buffer: *const u8,
        size: usize,
        flags: u32,
        parser: *mut CborParser,
        it: *mut CborValue,
    ) -> c_int;
    fn cbor_value_to_json_advance(out: *mut c_void, value: *mut CborValue, flags: c_int) -> c_int;

    // libc, to give the FILE*-taking entry point an in-memory sink.
    fn open_memstream(bufp: *mut *mut c_char, sizep: *mut usize) -> *mut c_void;
    fn fclose(stream: *mut c_void) -> c_int;
    fn free(ptr: *mut c_void);
}

/// Runs the input through this port. Returns `(CborError, stdout bytes)`.
fn ours(data: &[u8], flags: c_int) -> (c_int, Vec<u8>) {
    // An empty slice's `as_ptr()` is dangling; hand C a real address instead.
    let pad = [0u8; 1];
    let ptr = if data.is_empty() {
        pad.as_ptr()
    } else {
        data.as_ptr()
    };

    // SAFETY: `parser` and `value` are owned here and live across both calls;
    // `cbor_parser_init` writes every field of both before anything reads them.
    // `sink` is an open FILE* until `fclose`, and `buf`/`len` are only read
    // after `fclose` has flushed and published them, as open_memstream requires.
    unsafe {
        let mut parser = MaybeUninit::<CborParser>::zeroed().assume_init();
        let mut value = MaybeUninit::<CborValue>::zeroed().assume_init();

        let mut buf: *mut c_char = std::ptr::null_mut();
        let mut len: usize = 0;
        let sink = open_memstream(&mut buf, &mut len);
        assert!(!sink.is_null(), "open_memstream failed");

        let mut err = cbor_parser_init(ptr, data.len(), 0, &mut parser, &mut value);
        if err == 0 {
            err = cbor_value_to_json_advance(sink, &mut value, flags);
        }

        fclose(sink);
        let out = if buf.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(buf as *const u8, len).to_vec()
        };
        free(buf as *mut c_void);
        (err, out)
    }
}

/// Runs the input through the upstream C oracle, or `None` if the machine would
/// not run it right now.
///
/// `None` covers spawn and pipe failures only. This harness forks several
/// hundred processes a second and the kernel intermittently answers EAGAIN or
/// EMFILE under that load, which is a fact about the machine rather than a
/// difference between the two implementations. A missing oracle is a different
/// thing and is caught once, loudly.
fn theirs(data: &[u8], flags: c_int) -> Option<(c_int, Vec<u8>)> {
    static PRESENT: OnceLock<()> = OnceLock::new();
    PRESENT.get_or_init(|| {
        assert!(
            std::path::Path::new(ORACLE).exists(),
            "no oracle at {ORACLE} -- run fuzz/oracle/build.sh"
        );
    });

    let mut child = Command::new(ORACLE)
        .arg("json")
        .arg(flags.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    // Taking the handle closes it after the write, which is what makes the
    // oracle's fread() see EOF. It drains stdin before writing anything, so
    // there is no order in which the two pipes can deadlock.
    let mut stdin = child.stdin.take().expect("piped");
    let written = stdin.write_all(data);
    drop(stdin);

    let out = child.wait_with_output().ok()?;
    written.ok()?;

    // No exit code means a signal killed it. Upstream faulting on an input is
    // a genuine finding, so this one is not swallowed.
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
    Some((code, out.stdout))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn show(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => format!("{s:?}"),
        Err(_) => format!("<non-utf8> {}", hex(bytes)),
    }
}

fuzz_target!(|input: &[u8]| {
    // First byte picks the flags, the rest is the document.
    let Some((&selector, data)) = input.split_first() else {
        return;
    };
    if data.len() > MAX_INPUT {
        return;
    }
    let flags = selector as c_int & FLAG_BITS;

    let (our_err, our_out) = ours(data, flags);
    let Some((their_err, their_out)) = theirs(data, flags) else {
        return;
    };

    if our_err != their_err {
        panic!(
            "DIVERGENCE (error code)\n  \
             input : {}\n  \
             flags : {flags}\n  \
             ours  : {our_err}\n  \
             theirs: {their_err}\n  \
             our stdout   : {}\n  \
             their stdout : {}",
            hex(data),
            show(&our_out),
            show(&their_out),
        );
    }

    // See the module comment: on the error paths the two sinks legitimately
    // hold different amounts of a conversion that was never going to finish.
    if their_err == 0 && our_out != their_out {
        panic!(
            "DIVERGENCE (stdout)\n  \
             input : {}\n  \
             flags : {flags}\n  \
             ours  : {}\n  \
             theirs: {}\n  \
             (both returned CborNoError)",
            hex(data),
            show(&our_out),
            show(&their_out),
        );
    }
});
