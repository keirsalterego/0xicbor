//! `json2cbor`, a pure-Rust rewrite of intel/tinycbor's tool of the same name.
//!
//! The contract is tools/json2cbor/json2cbor.c, which links against cJSON. The
//! grammar it accepts is therefore cJSON's rather than RFC 8259's, and the
//! quirks that follow from that are reproduced in json.rs. So are the ones
//! that follow from reusing the input buffer for the output, in encode.rs.

mod encode;
mod json;

use cbor_core::errstr::error_string;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::process::ExitCode;

const USAGE: &str = "\
Usage: json2cbor [OPTION]... [FILE]...
Reads JSON content from FILE and converts to CBOR.

Options:
 -M       Interpret metadata added by cbordump tool
";

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let mut metadata = false;

    let mut opts = Getopt::new(&argv, "M");
    while let Some(opt) = opts.next() {
        match opt {
            Ok('M') => metadata = true,
            Ok(_) => unreachable!("the match and the option spec list the same letters"),
            // "h" is not in the option spec, so asking for help is an unknown
            // option: the help is printed but the exit code is a failure, and
            // upstream's `case 'h'` is unreachable. Kept.
            Err(c) => {
                eprintln!("Unknown option -{c}.");
                println!("{USAGE}");
                return ExitCode::FAILURE;
            }
        }
    }

    // The usage line says FILE..., but only the first operand is ever read.
    let operands = opts.operands();
    let path = operands.first().copied().filter(|name| *name != "-");
    let name = path.unwrap_or("-");

    let (data, capacity) = match read_input(path) {
        Ok(pair) => pair,
        Err(e) => return e,
    };

    let Some(document) = json::parse(&data) else {
        eprintln!("json2cbor: {name}: could not parse.");
        return ExitCode::FAILURE;
    };

    let mut encoder = encode::Encoder::new(capacity);
    if let Err(e) = encode::encode(&mut encoder, &document, metadata) {
        let text = error_string(e as i32).to_str().unwrap_or("unknown error");
        eprintln!("json2cbor: {name}: error encoding to CBOR: {text}");
        return ExitCode::FAILURE;
    }

    let _ = io::stdout().write_all(&encoder.out);
    ExitCode::SUCCESS
}

/// Reads the whole input, and reports how much room the encoder gets.
///
/// Upstream encodes into the very buffer it read the JSON into. When it could
/// measure the file up front that buffer is one byte longer than the file, for
/// the terminating NUL; when it had to read in chunks the NUL goes in the
/// slack at the end of the last chunk and the encoder is told the shorter
/// size. Both are reproduced, because they decide when the encoder runs out of
/// room and says so.
fn read_input(path: Option<&str>) -> Result<(Vec<u8>, usize), ExitCode> {
    let mut data = Vec::new();
    let Some(path) = path else {
        return match io::stdin().read_to_end(&mut data) {
            Ok(_) => {
                let capacity = data.len();
                Ok((data, capacity))
            }
            Err(e) => {
                eprintln!("read: {}", strerror(&e));
                Err(ExitCode::FAILURE)
            }
        };
    };

    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("open: {}", strerror(&e));
            return Err(ExitCode::FAILURE);
        }
    };
    let size = file
        .seek(SeekFrom::End(0))
        .ok()
        .and_then(|n| usize::try_from(n).ok());
    if size.is_some() {
        let _ = file.rewind();
    }
    if let Err(e) = file.read_to_end(&mut data) {
        eprintln!("read: {}", strerror(&e));
        return Err(ExitCode::FAILURE);
    }
    let capacity = size.map_or(data.len(), |n| n + 1);
    Ok((data, capacity))
}

/// `strerror(errno)`. Rust appends " (os error N)" to the text it gets from the
/// C library; `perror` does not, so take it back off.
fn strerror(e: &io::Error) -> String {
    let text = e.to_string();
    match e.raw_os_error() {
        Some(code) => text
            .trim_end_matches(format!(" (os error {code})").as_str())
            .to_string(),
        None => text,
    }
}

/// `getopt(3)` as glibc implements it, cut down to what this tool needs: no
/// option takes an argument, so what is left is bundling, `--` ending the
/// scan, the diagnostic glibc writes for an unknown letter, and GNU's
/// permutation of operands to the end.
///
/// A copy of cbordump's. Two forty-line scanners in two binaries beat a third
/// crate in the workspace to hold one of them.
struct Getopt<'a> {
    argv: &'a [String],
    spec: &'a str,
    at: usize,
    inside: usize,
    operands: Vec<&'a str>,
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
                self.operands.push(arg);
                self.at += 1;
                continue;
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
