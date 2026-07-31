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

/// `getopt(3)` as glibc implements it, cut down to what this tool needs.
///
/// No option takes an argument, so what is left is bundling, `--` ending the
/// scan, the diagnostic glibc writes for an unknown letter, and where the scan
/// stops. json2cbor.c defines `_GNU_SOURCE`, so this is the GNU getopt, which
/// moves operands to the end and keeps looking for options past them — unless
/// POSIXLY_CORRECT is set, which turns that off.
///
/// A near-copy of cbordump's, which gets the POSIX variant instead. Two
/// forty-line scanners in two binaries beat a third crate to hold one of them.
struct Getopt<'a> {
    argv: &'a [String],
    spec: &'a str,
    at: usize,
    inside: usize,
    operands: Vec<&'a str>,
    ended: bool,
    /// GNU getopt permutes; POSIXLY_CORRECT makes it stop at the first operand.
    permute: bool,
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
            permute: std::env::var_os("POSIXLY_CORRECT").is_none(),
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
                if !self.permute {
                    // Whatever follows the first operand is an operand too.
                    self.operands
                        .extend(self.argv[self.at..].iter().map(String::as_str));
                    self.at = self.argv.len();
                    return None;
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use cbor_core::CborError;

    fn convert(text: &str, metadata: bool) -> Result<Vec<u8>, Option<CborError>> {
        let document = json::parse(text.as_bytes()).ok_or(None)?;
        let mut encoder = encode::Encoder::new(text.len() + 1);
        encode::encode(&mut encoder, &document, metadata).map_err(Some)?;
        Ok(encoder.out)
    }

    #[test]
    fn plain_json() {
        assert_eq!(convert("[1,2,3]", false), Ok(vec![0x83, 1, 2, 3]));
        assert_eq!(
            convert("{\"a\":true}", false),
            Ok(vec![0xa1, 0x61, b'a', 0xf5])
        );
        assert_eq!(convert("null", false), Ok(vec![0xf6]));
        assert_eq!(convert("-1", false), Ok(vec![0x20]));
        assert_eq!(convert("1e4", false), Ok(vec![0x19, 0x27, 0x10]));
        // Past INT_MAX cJSON's saturated `valueint` no longer matches the
        // double, so the number stops being an integer.
        assert_eq!(
            convert("2147483647", false),
            Ok(vec![0x1a, 0x7f, 0xff, 0xff, 0xff])
        );
        assert_eq!(
            convert("2147483648", false),
            Ok(vec![0xfb, 0x41, 0xe0, 0, 0, 0, 0, 0, 0])
        );
    }

    #[test]
    fn cjson_grammar() {
        // Accepted by cJSON and not by RFC 8259, or the other way round.
        assert!(convert("\"\\/\"", false).is_ok());
        assert_eq!(convert("\"\\uZZZZ\"", false), Ok(vec![0x60])); // bad hex reads as U+0000
        assert_eq!(convert("01", false), Ok(vec![0x01])); // a leading zero is fine
        assert_eq!(convert("\u{feff}[1]", false), Ok(vec![0x81, 0x01])); // a BOM is skipped
        assert!(convert("[1,]", false).is_err());
        assert!(convert("nan", false).is_err());
        assert!(convert("[1,2,3]x", false).is_err());
    }

    #[test]
    fn metadata_restores_what_json_lost() {
        // Byte string from base64url, and the same JSON without -M.
        assert_eq!(
            convert("{\"a\":\"AQID\",\"a$cbor\":{\"t\":64}}", true),
            Ok(vec![0xbf, 0x61, b'a', 0x43, 1, 2, 3, 0xff])
        );
        assert_eq!(
            convert("{\"a\":\"AQID\",\"a$cbor\":{\"t\":64}}", false),
            Ok(b"\xa2\x61a\x64AQID\x66a$cbor\xa1\x61t\x18\x40".to_vec())
        );
        // A base64 group that is not a full four characters still decodes to
        // three bytes, two of which are not the ones base64 says. Upstream's,
        // and the reason a byte string whose length is not a multiple of three
        // does not survive a cbordump -jM round trip.
        assert_eq!(
            convert("{\"a\":\"AQIDBA\",\"a$cbor\":{\"t\":64}}", true),
            Ok(vec![0xbf, 0x61, b'a', 0x46, 1, 2, 3, 0, 0, 0x40, 0xff])
        );
        // Infinities and NaN come back through the "v" string.
        assert_eq!(
            convert("{\"a\":null,\"a$cbor\":{\"t\":251,\"v\":\"-inf\"}}", true),
            Ok(vec![
                0xbf, 0x61, b'a', 0xfb, 0xff, 0xf0, 0, 0, 0, 0, 0, 0, 0xff
            ])
        );
        // A tag wraps the value it was recorded against.
        assert_eq!(
            convert("{\"a\":1,\"a$cbor\":{\"tag\":\"55799\"}}", true),
            Ok(vec![0xbf, 0x61, b'a', 0xd9, 0xd9, 0xf7, 0x01, 0xff])
        );
        // Simple values 25 to 31 are the float encodings and the break code.
        assert_eq!(
            convert("{\"a\":0,\"a$cbor\":{\"t\":224,\"v\":25}}", true),
            Err(Some(CborError::IllegalSimpleType))
        );
    }

    #[test]
    fn buffer_runs_out_only_where_upstream_lets_it() {
        // The encoder gets the size of the JSON text plus one, and CBOR is
        // shorter than JSON except for doubles — which may grow the buffer.
        assert_eq!(
            convert("0.5", false),
            Ok(vec![0xfb, 0x3f, 0xe0, 0, 0, 0, 0, 0, 0])
        );
        // "1e5" is three bytes of JSON and five of CBOR, and an integer may
        // not grow the buffer the way a double may.
        assert_eq!(convert("1e5", false), Err(Some(CborError::OutOfMemory)));
    }

    #[test]
    fn options_move_to_the_front() {
        let argv: Vec<String> = ["json2cbor", "file", "-M", "more"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut opts = Getopt::new(&argv, "M");
        assert!(matches!(opts.next(), Some(Ok('M'))));
        assert!(opts.next().is_none());
        assert_eq!(opts.operands(), ["file", "more"]);
    }
}
