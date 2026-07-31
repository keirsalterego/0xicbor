//! `cbordump`, a pure-Rust rewrite of intel/tinycbor's tool of the same name.
//!
//! The contract is tools/cbordump/cbordump.c: the same flags, the same output,
//! the same exit codes, and the same diagnostics — including the ones glibc's
//! `getopt` prints rather than the program. Where upstream does something
//! surprising the comment says so and the surprise stays.

mod cbor;
mod json;
mod pretty;

use cbor_core::errstr::error_string;
use cbor_core::CborError;
use std::fs::File;
use std::io::{self, Read, Write};
use std::process::ExitCode;

const USAGE: &str = "\
Usage: cbordump [OPTION]... [FILE]...
Interprets FILEs as CBOR binary data and dumps the content to stdout.

Options:
 -c       Print a CBOR dump (see RFC 7049) (default)
 -j       Print a JSON equivalent version
 -h       Print this help output and exit
When JSON output is active, the following options are recognized:
 -M       Add metadata so converting back to CBOR is possible
 -O       Convert CBOR tags to JSON objects
 -S       Stringify non-text string map keys
 -U       Convert all CBOR byte strings to Base64url regardless of tags
When CBOR dump is active, the following options are recognized:
 -f       Show text and byte string fragments
 -n       Show overlong encoding of CBOR numbers and length";

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let mut print_json = false;
    let mut json_flags = 0;
    let mut cbor_flags = pretty::DEFAULT_FLAGS;

    let mut opts = Getopt::new(&argv, "MOSUcjhfn");
    while let Some(opt) = opts.next() {
        match opt {
            Ok('c') => print_json = false,
            Ok('j') => print_json = true,
            Ok('f') => cbor_flags |= pretty::SHOW_STRING_FRAGMENTS,
            // The help text calls this "show overlong encoding of CBOR numbers
            // and length", but the flags it sets are the indeterminate-length
            // indicator, which is on by default anyway, and the numeric
            // encoding indicators, which only swap the float suffixes f16 and
            // f for _1 and _2. CborPrettyIndicateOverlongNumbers is never set
            // by this tool at all. Copied verbatim, mismatch included.
            Ok('n') => {
                cbor_flags |=
                    pretty::INDICATE_INDETERMINATE_LENGTH | pretty::NUMERIC_ENCODING_INDICATORS;
            }
            Ok('M') => json_flags |= json::ADD_METADATA,
            Ok('O') => json_flags |= json::TAGS_TO_OBJECTS,
            Ok('S') => json_flags |= json::STRINGIFY_MAP_KEYS,
            Ok('U') => json_flags |= json::BYTE_STRINGS_TO_BASE64URL,
            Ok('h') => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            Ok(_) => unreachable!("the match and the option spec list the same letters"),
            Err(c) => {
                eprintln!("Unknown option -{c}.");
                println!("{USAGE}");
                return ExitCode::FAILURE;
            }
        }
    }

    let flags = if print_json { json_flags } else { cbor_flags };
    let files = opts.operands();

    // With no file arguments the input is stdin under the name "-". Note that
    // a "-" argument is *not* stdin: upstream passes it to fopen and fails.
    if files.is_empty() {
        let mut data = Vec::new();
        if let Err(e) = io::stdin().read_to_end(&mut data) {
            eprintln!("-: {}", strerror(&e));
            return ExitCode::FAILURE;
        }
        if let Err(e) = dump(&data, print_json, flags) {
            eprintln!("-: {}", error_text(e));
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    for name in files {
        let mut file = match File::open(name) {
            Ok(f) => f,
            // perror("open"), so the name of the file that would not open is
            // not part of the message.
            Err(e) => {
                eprintln!("open: {}", strerror(&e));
                return ExitCode::FAILURE;
            }
        };
        let mut data = Vec::new();
        if let Err(e) = file.read_to_end(&mut data) {
            eprintln!("{name}: {}", strerror(&e));
            return ExitCode::FAILURE;
        }
        if let Err(e) = dump(&data, print_json, flags) {
            eprintln!("{name}: {}", error_text(e));
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn error_text(err: CborError) -> &'static str {
    error_string(err as i32).to_str().unwrap_or("unknown error")
}

fn dump(data: &[u8], print_json: bool, flags: i32) -> Result<(), CborError> {
    let mut r = cbor::Reader::new(data);
    let mut out: Vec<u8> = Vec::new();
    let result = if print_json {
        json::value(&mut out, &mut r, flags)
    } else {
        let mut text = String::new();
        let result = pretty::value(
            &mut text,
            &mut r,
            flags,
            cbor::MAX_RECURSIONS,
            cbor::After::Stop,
        );
        out.extend_from_slice(text.as_bytes());
        result
    };

    // Upstream writes to stdout as it formats, so whatever it managed before
    // hitting an error is on the user's terminal already. Keep that.
    let stdout = io::stdout();
    let mut sink = stdout.lock();
    let _ = sink.write_all(&out);
    result?;
    let _ = sink.write_all(b"\n");

    // One document per file, with nothing after it.
    if r.pos != data.len() {
        return Err(CborError::GarbageAtEnd);
    }
    Ok(())
}

/// `strerror(errno)`. Rust appends " (os error N)" to the text it gets from the
/// C library; `perror` and `strerror` do not, so take it back off.
fn strerror(e: &io::Error) -> String {
    let text = e.to_string();
    match e.raw_os_error() {
        Some(code) => text
            .trim_end_matches(format!(" (os error {code})").as_str())
            .to_string(),
        None => text,
    }
}

/// `getopt(3)` as glibc implements it, cut down to what this tool needs.
///
/// No option here takes an argument, so what is left is bundling (`-cj`), `--`
/// ending the scan, the diagnostic glibc writes for an unknown letter, and
/// where the scan stops. cbordump.c defines `_POSIX_C_SOURCE` and not
/// `_GNU_SOURCE`, which binds `getopt` to glibc's `__posix_getopt`: it does
/// *not* permute, so the first operand ends the options and `cbordump f -j`
/// looks for a file called "-j".
struct Getopt<'a> {
    argv: &'a [String],
    spec: &'a str,
    /// Index of the argument being examined.
    at: usize,
    /// Offset of the next letter inside a bundle, or 0 when between arguments.
    inside: usize,
    operands: Vec<&'a str>,
    /// Set by `--`: everything after it is an operand whatever it looks like.
    ended: bool,
}

impl<'a> Getopt<'a> {
    fn new(argv: &'a [String], spec: &'a str) -> Self {
        Getopt {
            argv,
            spec,
            at: 1,
            inside: 0,
            operands: Vec::new(),
            ended: false,
        }
    }

    #[allow(clippy::should_implement_trait)] // not an Iterator: `operands` outlives the scan
    fn next(&mut self) -> Option<Result<char, char>> {
        loop {
            if self.inside > 0 {
                let arg = self.argv[self.at].as_bytes();
                let c = char::from(arg[self.inside]);
                self.inside += 1;
                if self.inside == arg.len() {
                    self.inside = 0;
                    self.at += 1;
                }
                if self.spec.contains(c) {
                    return Some(Ok(c));
                }
                eprintln!("{}: invalid option -- '{c}'", self.argv[0]);
                return Some(Err(c));
            }

            let arg = self.argv.get(self.at)?.as_str();
            // A lone "-" is an operand, not an empty bundle.
            if self.ended || arg == "-" || !arg.starts_with('-') {
                // The scan stops here: whatever follows is an operand too.
                self.operands
                    .extend(self.argv[self.at..].iter().map(String::as_str));
                self.at = self.argv.len();
                return None;
            }
            if arg == "--" {
                self.ended = true;
                self.at += 1;
                continue;
            }
            self.inside = 1;
        }
    }

    fn operands(self) -> Vec<&'a str> {
        self.operands
    }
}
