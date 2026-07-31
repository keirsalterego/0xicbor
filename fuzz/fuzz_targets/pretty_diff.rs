//! Differential fuzzing of the pretty printer against upstream tinycbor.
//!
//! The same bytes go to two places:
//!
//!   * this port, through the C entry points `cbor_parser_init` and
//!     `cbor_value_to_pretty_advance` exported by the cbor-ffi rlib -- the
//!     exact symbols that make up the shipped `libtinycbor.a`;
//!   * `oracle/cbor-oracle`, a separate executable built from upstream C,
//!     spawned as a subprocess and fed the input on stdin.
//!
//! Out-of-process is the whole point. The shipped Rust artifact contains no C:
//! no `cc` crate, no `bindgen`, no build.rs, no link against upstream. A pipe
//! is the only channel between the two implementations, so that property is
//! enforced by the architecture rather than asserted in a README.
//!
//! ## What is compared
//!
//! The error code, always, exactly. And stdout, whenever the parse succeeded.
//!
//! stdout is deliberately not compared on the error paths, and this is a real
//! difference rather than an oversight: upstream streams the rendering out as
//! it walks the value, so a value that fails halfway leaves a partial prefix on
//! stdout (`\x83\x01` prints `[1` and then returns CborErrorUnexpectedEOF). The
//! port renders into a `String` first and writes nothing at all when the walk
//! fails. Both agree on every byte of a successful render and on every error
//! code; they disagree on how much of a doomed render reaches the sink. The
//! oracle is left faithful -- it still streams -- so the raw bytes are on hand
//! if that behaviour is ever tightened.

#![no_main]

use std::ffi::c_void;
use std::io::Write;
use std::mem::MaybeUninit;
use std::os::raw::{c_char, c_int};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
// The cbor-ffi package names its library target `tinycbor`, after the artifact.
use tinycbor::{CborParser, CborValue};

/// Built by `fuzz/oracle/build.sh`. Resolved at compile time so the harness
/// cannot silently fall back to some other binary on $PATH.
const ORACLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/oracle/cbor-oracle");

/// Bigger inputs cost a subprocess round trip without buying coverage; every
/// interesting shape in the pretty printer fits well under this.
const MAX_INPUT: usize = 16 * 1024;

extern "C" {
    // The port, via the C ABI it ships.
    fn cbor_parser_init(
        buffer: *const u8,
        size: usize,
        flags: u32,
        parser: *mut CborParser,
        it: *mut CborValue,
    ) -> c_int;
    fn cbor_value_to_pretty_advance(out: *mut c_void, value: *mut CborValue) -> c_int;

    // libc, to give the FILE*-taking entry point an in-memory sink. This is the
    // system C library every Rust binary already links; it is not upstream
    // tinycbor and not compiled from source here.
    fn open_memstream(bufp: *mut *mut c_char, sizep: *mut usize) -> *mut c_void;
    fn fclose(stream: *mut c_void) -> c_int;
    fn free(ptr: *mut c_void);
}

/// Runs the input through this port. Returns `(CborError, stdout bytes)`.
fn ours(data: &[u8]) -> (c_int, Vec<u8>) {
    // An empty slice's `as_ptr()` is dangling; hand C a real address instead.
    // The length is still zero, so nothing is read through it either way.
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
            err = cbor_value_to_pretty_advance(sink, &mut value);
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

/// Runs the input through the upstream C oracle. Returns `(CborError, stdout)`,
/// or `None` if the machine would not run it right now.
///
/// The error comes off stderr, not the exit status: `CborError` runs up to
/// `INT_MAX` and an exit status carries eight bits.
///
/// `None` covers spawn and pipe failures only. This harness forks several
/// hundred processes a second, and under that load the kernel intermittently
/// answers EAGAIN or EMFILE. That is a fact about the machine, not a
/// difference between the two implementations, so the input is skipped instead
/// of being reported as a divergence -- otherwise the artifact directory fills
/// up with inputs that do not reproduce. A missing oracle is a different thing
/// and is caught once, loudly, below.
fn theirs(data: &[u8]) -> Option<(c_int, Vec<u8>)> {
    static PRESENT: OnceLock<()> = OnceLock::new();
    PRESENT.get_or_init(|| {
        assert!(
            std::path::Path::new(ORACLE).exists(),
            "no oracle at {ORACLE} -- run fuzz/oracle/build.sh"
        );
    });

    let mut child = Command::new(ORACLE)
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
        "oracle died on a signal ({}) for input {}",
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

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }

    let (our_err, our_out) = ours(data);
    let Some((their_err, their_out)) = theirs(data) else {
        return;
    };

    if our_err != their_err {
        panic!(
            "DIVERGENCE (error code)\n  \
             input : {}\n  \
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
    // hold different amounts of a rendering that was never going to finish.
    if their_err == 0 && our_out != their_out {
        panic!(
            "DIVERGENCE (stdout)\n  \
             input : {}\n  \
             ours  : {}\n  \
             theirs: {}\n  \
             (both returned CborNoError)",
            hex(data),
            show(&our_out),
            show(&their_out),
        );
    }
});
