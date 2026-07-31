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
use std::io::{self, IsTerminal, Read, Write};
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
    let mut sink = Stdout::new();
    let code = run(&mut sink);
    sink.flush();
    code
}

fn run(sink: &mut Stdout) -> ExitCode {
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
                sink.write(USAGE.as_bytes());
                sink.write(b"\n");
                return ExitCode::SUCCESS;
            }
            Ok(_) => unreachable!("the match and the option spec list the same letters"),
            Err(c) => {
                eprintln!("Unknown option -{c}.");
                sink.write(USAGE.as_bytes());
                sink.write(b"\n");
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
        if let Err(e) = dump(sink, &data, print_json, flags) {
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
        if let Err(e) = dump(sink, &data, print_json, flags) {
            eprintln!("{name}: {}", error_text(e));
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn error_text(err: CborError) -> &'static str {
    error_string(err as i32).to_str().unwrap_or("unknown error")
}

fn dump(sink: &mut Stdout, data: &[u8], print_json: bool, flags: i32) -> Result<(), CborError> {
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
    // hitting an error has been written already. Keep that.
    sink.write(&out);
    result?;
    sink.write(b"\n");

    // One document per file, with nothing after it.
    if r.pos != data.len() {
        return Err(CborError::GarbageAtEnd);
    }
    Ok(())
}

/// stdout with libc's buffering: by line when it is a terminal, by block when
/// it is anything else, and flushed on exit either way.
///
/// Writing straight through would be simpler, but the buffering decides
/// whether a dump or the error that follows it comes out first when both
/// streams are captured together, and that is part of what the tool prints.
struct Stdout {
    pending: Vec<u8>,
    by_line: bool,
}

impl Stdout {
    fn new() -> Self {
        Stdout {
            pending: Vec::new(),
            by_line: io::stdout().is_terminal(),
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        if !self.by_line {
            return;
        }
        if let Some(last) = self.pending.iter().rposition(|&b| b == b'\n') {
            let tail = self.pending.split_off(last + 1);
            let _ = io::stdout().write_all(&self.pending);
            self.pending = tail;
        }
    }

    fn flush(&mut self) {
        let _ = io::stdout().write_all(&self.pending);
        self.pending.clear();
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders a document the way `dump` does, but into a string.
    fn render(bytes: &[u8], print_json: bool, flags: i32) -> Result<String, CborError> {
        let mut r = cbor::Reader::new(bytes);
        let mut out = Vec::new();
        if print_json {
            json::value(&mut out, &mut r, flags)?;
        } else {
            let mut text = String::new();
            pretty::value(
                &mut text,
                &mut r,
                flags,
                cbor::MAX_RECURSIONS,
                cbor::After::Stop,
            )?;
            out = text.into_bytes();
        }
        if r.pos != bytes.len() {
            return Err(CborError::GarbageAtEnd);
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    #[test]
    fn diagnostic_notation() {
        // RFC 8949 Appendix A, plus the shapes with an encoding of their own.
        for (bytes, want) in [
            (&b"\x00"[..], "0"),
            (b"\x20", "-1"),
            (
                b"\x3b\xff\xff\xff\xff\xff\xff\xff\xff",
                "-18446744073709551616",
            ),
            (b"\x44\x01\x02\x03\x04", "h'01020304'"),
            (b"\x64\xc3\xa9\x21\x21", "\"\\u00E9!!\""),
            (b"\x83\x01\x02\x03", "[1, 2, 3]"),
            (b"\x9f\x01\xff", "[_ 1]"),
            (b"\xa1\x61\x61\x01", "{\"a\": 1}"),
            (b"\xbf\xff", "{_ }"),
            (b"\xc0\x61\x61", "0(\"a\")"),
            (b"\xf4", "false"),
            (b"\xf6", "null"),
            (b"\xf7", "undefined"),
            (b"\xf8\x20", "simple(32)"),
            (b"\xf9\x3c\x00", "1.f16"),
            (b"\xfa\x47\xc3\x50\x00", "100000.f"),
            (
                b"\xfb\x40\x09\x21\xfb\x54\x44\x2d\x18",
                "3.1415926535897931",
            ),
            (b"\xfb\x7f\xf8\x00\x00\x00\x00\x00\x00", "nan"),
            // Merged fragments take their indicator from the first chunk, so
            // this one has none while an empty chunked string has "_".
            (b"\x5f\x41\x61\xff", "h'61'"),
            (b"\x5f\xff", "h''_"),
        ] {
            assert_eq!(
                render(bytes, false, pretty::DEFAULT_FLAGS).as_deref(),
                Ok(want)
            );
        }
    }

    #[test]
    fn json_conversion() {
        let flags = json::ADD_METADATA;
        assert_eq!(
            render(b"\x83\x01\x02\x03", true, 0).as_deref(),
            Ok("[1,2,3]")
        );
        assert_eq!(
            render(b"\xa1\x61\x61\x01", true, 0).as_deref(),
            Ok("{\"a\":1}")
        );
        // Byte strings are not native to JSON, and the metadata says so.
        assert_eq!(
            render(b"\xa1\x61\x61\x44\x01\x02\x03\x04", true, flags).as_deref(),
            Ok("{\"a\":\"AQIDBA\",\"a$cbor\":{\"t\":64}}")
        );
        // Tag 23 asks for base16 and is obeyed unless -U overrides it.
        assert_eq!(
            render(b"\xd7\x44\x01\x02\x03\x04", true, 0).as_deref(),
            Ok("\"01020304\"")
        );
        assert_eq!(
            render(
                b"\xd7\x44\x01\x02\x03\x04",
                true,
                json::BYTE_STRINGS_TO_BASE64URL
            )
            .as_deref(),
            Ok("\"AQIDBA\"")
        );
        // A non-string key fails unless it may be stringified.
        assert_eq!(
            render(b"\xa1\x01\x02", true, 0),
            Err(CborError::JsonObjectKeyNotString)
        );
        assert_eq!(
            render(b"\xa1\x01\x02", true, json::STRINGIFY_MAP_KEYS).as_deref(),
            Ok("{\"1\":2}")
        );
    }

    #[test]
    fn malformed_input_is_named_the_way_upstream_names_it() {
        for (bytes, want) in [
            (&b""[..], CborError::UnexpectedEOF),
            (b"\x18", CborError::UnexpectedEOF),
            (b"\x1c", CborError::IllegalNumber),
            (b"\xfc", CborError::UnknownType),
            (b"\xff", CborError::UnexpectedBreak),
            (b"\xf8\x00", CborError::IllegalSimpleType),
            (b"\x1f", CborError::IllegalNumber),
            (b"\x5f\x61\x61\xff", CborError::IllegalType),
            (b"\x63\xff\xff\xff", CborError::InvalidUtf8TextString),
            (b"\x00\x00", CborError::GarbageAtEnd),
            (b"\xbf\x01\xff", CborError::UnexpectedBreak),
        ] {
            assert_eq!(
                render(bytes, false, pretty::DEFAULT_FLAGS),
                Err(want),
                "{bytes:02x?}"
            );
        }
    }

    #[test]
    fn options_stop_at_the_first_operand() {
        let argv: Vec<String> = ["cbordump", "-cj", "file", "-j"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut opts = Getopt::new(&argv, "MOSUcjhfn");
        assert!(matches!(opts.next(), Some(Ok('c'))));
        assert!(matches!(opts.next(), Some(Ok('j'))));
        assert!(opts.next().is_none());
        // The trailing -j is a file name, not an option.
        assert_eq!(opts.operands(), ["file", "-j"]);
    }
}
