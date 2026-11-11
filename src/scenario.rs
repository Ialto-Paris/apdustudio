// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! ApduStudio scripts (`.apdu` files): reading, writing and comparing answers.
//!
//! ```text
//! apdustudio-script 1
//! # A comment.
//! atr: 3B 88 80 01 00 00 00 00 77 81 80 00 7F
//!
//! step: Select the master file
//! card: 00 A4 00 00 02 3F 00
//! expect: 90 00
//! ```
//!
//! The first line names the format and its version. Every other line is blank, a comment (`#`),
//! a keyword line (`atr:`, `step:`, `card:`, `reader:`, `expect:`), or the continuation of the
//! line above it (it starts with a space or a tab). `card:` sends its bytes to the card
//! (SCardTransmit), `reader:` to the reader as a CCID escape command (SCardControl). In an
//! expected answer `?` matches any hex digit and a final `*` ends the comparison. A command
//! holding a placeholder (`<PIN, 4 bytes>`) is skipped until it's filled in.

use std::fmt;
use std::path::Path;

/// The file extension of a script.
pub const EXTENSION: &str = "apdu";
/// The word on a script's first line, before its format version.
pub const HEADER: &str = "apdustudio-script";
/// The format version this ApduStudio reads and writes.
pub const VERSION: u32 = 1;
/// Most commands a script may hold: a bound against a runaway file, not a rule of the format.
pub const MAX_COMMANDS: usize = 10_000;

/// Where a command goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Target {
    /// To the card, with SCardTransmit.
    #[default]
    Card,
    /// To the reader, as a CCID escape command (SCardControl).
    Reader,
}

impl Target {
    /// The keyword of a command line, and the channel's name in reports.
    pub fn keyword(self) -> &'static str {
        match self {
            Target::Card => "card",
            Target::Reader => "reader",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Command {
    pub target: Target,
    /// The bytes as written: hex digits, spaces and placeholders (`<...>`).
    pub text: String,
    /// The `step:` line before it (empty when there is none).
    pub description: String,
    /// The expected answer, if any (`expect:`).
    pub expected: Option<String>,
    /// The command holds a placeholder, to fill in before it can be sent.
    pub incomplete: bool,
}

impl Command {
    pub fn new(target: Target, text: &str) -> Command {
        Command {
            target,
            text: text.to_string(),
            incomplete: has_placeholder(text),
            ..Command::default()
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Scenario {
    pub commands: Vec<Command>,
    /// The ATR the card must have before a run (`atr:`).
    pub atr_expected: Option<String>,
    /// The file held more than `MAX_COMMANDS` commands: only the first ones were read.
    pub too_many: bool,
}

/// Why a script couldn't be read: the line (counted from 1) and what is wrong with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// Why a script file couldn't be opened.
#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Parse(ParseError),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io(e) => e.fmt(f),
            LoadError::Parse(e) => e.fmt(f),
        }
    }
}

/// A command holds a placeholder (`<...>`) to fill in before it can be sent.
pub fn has_placeholder(text: &str) -> bool {
    text.contains('<')
}

/// Checks the bytes of a command: hex digits, spaces and whole placeholders, with an even
/// number of digits when there is no placeholder.
fn check_command(text: &str) -> Result<(), String> {
    let mut digits = 0usize;
    let mut placeholder = false;
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c == '<' {
            let Some(end) = rest.find('>') else {
                return Err("a placeholder isn't closed with '>'".into());
            };
            placeholder = true;
            rest = rest.get(end + 1..).unwrap_or_default();
            continue;
        }
        if c.is_ascii_hexdigit() {
            digits += 1;
        } else if !c.is_whitespace() {
            return Err(format!("'{c}' isn't a hex digit"));
        }
        rest = rest.get(c.len_utf8()..).unwrap_or_default();
    }
    if digits == 0 && !placeholder {
        return Err("no bytes to send".into());
    }
    if !placeholder && digits % 2 == 1 {
        return Err("an odd number of hex digits".into());
    }
    Ok(())
}

/// Checks an expected answer (or ATR): hex digits, `?` for any digit, spaces, and `*` only at
/// the end.
pub fn check_expected(text: &str) -> Result<(), String> {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if t.is_empty() {
        return Err("nothing to compare with".into());
    }
    let body = t.strip_suffix('*').unwrap_or(&t);
    if let Some(c) = body.chars().find(|c| !c.is_ascii_hexdigit() && *c != '?') {
        return Err(if c == '*' {
            "'*' may only end the expected answer".into()
        } else {
            format!("'{c}' isn't a hex digit, '?' or a final '*'")
        });
    }
    if body.len() % 2 == 1 {
        return Err("an odd number of digits".into());
    }
    Ok(())
}

/// What the last keyword line was, for the continuation lines after it.
#[derive(Clone, Copy, PartialEq)]
enum Open {
    None,
    Atr,
    Step,
    Command,
    Expect,
}

/// Reads a script.
pub fn parse(content: &str) -> Result<Scenario, ParseError> {
    let mut sc = Scenario::default();
    let mut header = false;
    let mut step: Option<(usize, String)> = None;
    let mut open = Open::None;
    let err = |line: usize, message: String| Err(ParseError { line, message });
    for (n, raw) in content.lines().enumerate() {
        let line = n + 1;
        let raw = raw.trim_end();
        let text = raw.trim_start();
        if text.is_empty() || text.starts_with('#') {
            open = Open::None;
            continue;
        }
        if !header {
            let mut words = text.split_whitespace();
            if words.next() != Some(HEADER) {
                return err(
                    line,
                    format!("an ApduStudio script starts with \"{HEADER} {VERSION}\""),
                );
            }
            match words.next().map(str::parse::<u32>) {
                Some(Ok(VERSION)) => {}
                Some(Ok(v)) if v > VERSION => {
                    return err(
                        line,
                        format!(
                            "format {v} is newer than this ApduStudio reads ({VERSION}): use a newer ApduStudio"
                        ),
                    )
                }
                _ => return err(line, format!("expected \"{HEADER} {VERSION}\"")),
            }
            header = true;
            continue;
        }
        if raw.starts_with([' ', '\t']) {
            // The line above goes on.
            let more = format!(" {text}");
            match open {
                Open::None => {
                    return err(
                        line,
                        "an indented line continues the line above, and there is none".into(),
                    )
                }
                Open::Atr => {
                    if let Some(a) = sc.atr_expected.as_mut() {
                        a.push_str(&more);
                    }
                }
                Open::Step => {
                    if let Some((_, s)) = step.as_mut() {
                        s.push_str(&more);
                    }
                }
                Open::Command => {
                    if let Some(c) = sc.commands.last_mut() {
                        c.text.push_str(&more);
                    }
                }
                Open::Expect => {
                    if let Some(e) = sc.commands.last_mut().and_then(|c| c.expected.as_mut()) {
                        e.push_str(&more);
                    }
                }
            }
            continue;
        }
        // Checks the value the last keyword line built, now that it's complete.
        if let Err(m) = close(&sc, open) {
            return err(line - 1, m);
        }
        let Some((key, value)) = text.split_once(':') else {
            return err(
                line,
                format!("\"{text}\" is neither a keyword line (atr:, step:, card:, reader:, expect:) nor a comment"),
            );
        };
        let value = value.trim().to_string();
        match key.trim().to_ascii_lowercase().as_str() {
            "atr" => {
                if sc.atr_expected.is_some() {
                    return err(line, "a second atr: line".into());
                }
                sc.atr_expected = Some(value);
                open = Open::Atr;
            }
            "step" => {
                if step.is_some() {
                    return err(line, "a second step: line before its command".into());
                }
                step = Some((line, value));
                open = Open::Step;
            }
            k @ ("card" | "reader") => {
                if sc.commands.len() >= MAX_COMMANDS {
                    sc.too_many = true;
                    return Ok(sc);
                }
                let target = if k == "card" {
                    Target::Card
                } else {
                    Target::Reader
                };
                let mut c = Command::new(target, &value);
                c.description = step.take().map(|(_, s)| s).unwrap_or_default();
                sc.commands.push(c);
                open = Open::Command;
            }
            "expect" => {
                if step.is_some() {
                    return err(line, "expect: belongs right after its command".into());
                }
                match sc.commands.last_mut() {
                    None => return err(line, "expect: before any command".into()),
                    Some(c) if c.expected.is_some() => {
                        return err(line, "a second expect: for the same command".into())
                    }
                    Some(c) => c.expected = Some(value),
                }
                open = Open::Expect;
            }
            other => {
                return err(
                    line,
                    format!("unknown keyword \"{other}:\" (atr:, step:, card:, reader:, expect:)"),
                )
            }
        }
    }
    if !header {
        return err(
            1,
            format!("empty: a script starts with \"{HEADER} {VERSION}\""),
        );
    }
    let last = content.lines().count();
    if let Err(m) = close(&sc, open) {
        return err(last, m);
    }
    if let Some((line, _)) = step {
        return err(line, "step: without a command after it".into());
    }
    Ok(sc)
}

/// Checks the value of the keyword line `open` once its continuation lines are read.
fn close(sc: &Scenario, open: Open) -> Result<(), String> {
    match open {
        Open::Atr => sc
            .atr_expected
            .as_deref()
            .map_or(Ok(()), check_expected)
            .map_err(|m| format!("atr: {m}")),
        Open::Command => sc.commands.last().map_or(Ok(()), |c| {
            check_command(&c.text).map_err(|m| format!("{}: {m}", c.target.keyword()))
        }),
        Open::Expect => sc
            .commands
            .last()
            .and_then(|c| c.expected.as_deref())
            .map_or(Ok(()), check_expected)
            .map_err(|m| format!("expect: {m}")),
        Open::None | Open::Step => Ok(()),
    }
}

pub fn load(path: &Path) -> Result<Scenario, LoadError> {
    let bytes = std::fs::read(path).map_err(LoadError::Io)?;
    parse(&String::from_utf8_lossy(&bytes)).map_err(LoadError::Parse)
}

/// The script as text.
pub fn to_text(sc: &Scenario) -> String {
    let mut out = format!("{HEADER} {VERSION}\n");
    if let Some(a) = &sc.atr_expected {
        out.push_str(&format!("\natr: {}\n", a.trim()));
    }
    for c in &sc.commands {
        out.push('\n');
        if !c.description.trim().is_empty() {
            out.push_str(&format!("step: {}\n", c.description.trim()));
        }
        out.push_str(&format!("{}: {}\n", c.target.keyword(), c.text.trim()));
        if let Some(e) = &c.expected {
            out.push_str(&format!("expect: {}\n", e.trim()));
        }
    }
    out
}

pub fn save(path: &Path, sc: &Scenario) -> std::io::Result<()> {
    std::fs::write(path, to_text(sc))
}

pub enum HexError {
    NotHex,
    Incomplete,
}

/// Turns "00 A4 04 00" or "00A40400" into bytes.
pub fn parse_hex(text: &str) -> Result<Vec<u8>, HexError> {
    let digits: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(HexError::NotHex);
    }
    if !digits.len().is_multiple_of(2) {
        return Err(HexError::Incomplete);
    }
    // Every digit was checked above, so each pair parses.
    Ok(digits
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .filter_map(|pair| std::str::from_utf8(pair).ok())
        .filter_map(|pair| u8::from_str_radix(pair, 16).ok())
        .collect())
}

/// Bytes as hex with a space between them: "90 00".
pub fn hex_spaced(bytes: &[u8]) -> String {
    let parts: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
    parts.join(" ")
}

/// Bytes as hex without spaces: "9000".
pub fn hex_compact(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

fn bytes(n: usize) -> String {
    if n == 1 {
        "1 byte".into()
    } else {
        format!("{n} bytes")
    }
}

/// Compares an answer with the expected one (`?` any digit, a final `*` ends the comparison).
/// Returns None when they match, or why they don't: "byte 1 is 6A, expected 90".
pub fn compare(expected: &str, answer: &[u8]) -> Option<String> {
    if let Err(m) = check_expected(expected) {
        return Some(format!("the expected answer isn't valid: {m}"));
    }
    let exp: Vec<char> = expected
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let open = exp.last() == Some(&'*');
    let exp = if open {
        &exp[..exp.len() - 1]
    } else {
        &exp[..]
    };
    let want = exp.len() / 2;
    for (i, pair) in exp.chunks(2).enumerate() {
        let Some(got) = answer.get(i) else {
            return Some(if open {
                format!(
                    "the answer has {}, expected at least {want}",
                    bytes(answer.len())
                )
            } else {
                format!("the answer has {}, expected {want}", bytes(answer.len()))
            });
        };
        let g: Vec<char> = format!("{got:02X}").chars().collect();
        let fits = pair.iter().zip(&g).all(|(e, g)| *e == '?' || e == g);
        if !fits {
            let e: String = pair.iter().collect();
            return Some(format!("byte {} is {got:02X}, expected {e}", i + 1));
        }
    }
    if !open && answer.len() != want {
        return Some(format!(
            "the answer has {}, expected {want}",
            bytes(answer.len())
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(body: &str) -> String {
        format!("{HEADER} {VERSION}\n{body}")
    }

    #[test]
    fn reads_a_script() {
        let sc = parse(&script(
            "# a comment\natr: 3B 02 14 50\n\nstep: Select the master file\n\
             card: 00 A4 00 00 02 3F 00\nexpect: 90 00\n\n\
             reader: 1A 00\n\
             step: Verify PIN\ncard: 00 20 00 80 04 <PIN, 4 bytes>\n",
        ))
        .expect("valid");
        assert_eq!(sc.atr_expected.as_deref(), Some("3B 02 14 50"));
        assert_eq!(sc.commands.len(), 3);
        let c = &sc.commands[0];
        assert_eq!(
            (c.target, c.text.as_str(), c.description.as_str()),
            (
                Target::Card,
                "00 A4 00 00 02 3F 00",
                "Select the master file"
            )
        );
        assert_eq!(c.expected.as_deref(), Some("90 00"));
        assert_eq!(sc.commands[1].target, Target::Reader);
        assert_eq!(sc.commands[1].description, "");
        assert!(sc.commands[2].incomplete);
        assert!(!sc.commands[0].incomplete);
    }

    #[test]
    fn indented_lines_continue_the_line_above() {
        let sc = parse(&script(
            "step: A long\n  description\ncard: 00 A4 04 00 07\n\tA0 00 00 00 04 10 10\nexpect: 6F\n  *\n",
        ))
        .expect("valid");
        let c = &sc.commands[0];
        assert_eq!(c.description, "A long description");
        assert_eq!(c.text, "00 A4 04 00 07 A0 00 00 00 04 10 10");
        assert_eq!(c.expected.as_deref(), Some("6F *"));
    }

    #[test]
    fn mistakes_name_their_line() {
        let bad = |body: &str| parse(&script(body)).expect_err(body);
        assert_eq!(bad("card: 00A4\ncrad: 00\n").line, 3);
        assert!(bad("crad: 00\n").message.contains("unknown keyword"));
        assert!(bad("card: 00 A4 0\n").message.contains("odd number"));
        assert!(bad("card: 00 G4\n")
            .message
            .contains("'G' isn't a hex digit"));
        assert!(bad("card: 00 <PIN\n").message.contains("isn't closed"));
        assert!(bad("expect: 9000\n").message.contains("before any command"));
        assert!(bad("card: 00\nexpect: 9000\nexpect: 9000\n")
            .message
            .contains("second expect"));
        assert!(bad("card: 00\nexpect: 9*00\n")
            .message
            .contains("may only end"));
        assert_eq!(bad("card: 00\nstep: dangling\n").line, 3);
        assert!(bad("  card: 00\n")
            .message
            .contains("continues the line above"));
        assert!(bad("atr: 3B\natr: 3B\n").message.contains("second atr"));
        assert!(bad("just text\n")
            .message
            .contains("neither a keyword line"));
        let no_header = parse("card: 00A4\n").expect_err("no header");
        assert_eq!(no_header.line, 1);
        assert!(no_header.message.contains(HEADER));
        let newer = parse(&format!("{HEADER} {}\n", VERSION + 1)).expect_err("newer");
        assert!(newer.message.contains("newer"), "{}", newer.message);
        assert!(parse("").is_err());
    }

    #[test]
    fn written_back_it_reads_the_same() {
        let sc = parse(&script(
            "atr: 3B 02 14 ??\nstep: Select\ncard: 00 A4 00 00 02 3F 00\nexpect: 90 00\n\
             reader: 1A 00\ncard: 00 20 00 80 04 <PIN>\nexpect: 63 C?\n",
        ))
        .expect("valid");
        let text = to_text(&sc);
        assert!(text.starts_with(&format!("{HEADER} {VERSION}\n")), "{text}");
        let again = parse(&text).expect("written text reads");
        assert_eq!(again.atr_expected, sc.atr_expected);
        assert_eq!(again.commands, sc.commands);
    }

    #[test]
    fn compares() {
        assert_eq!(compare("90 00", &[0x90, 0x00]), None);
        assert_eq!(compare("90 ??", &[0x90, 0x12]), None);
        assert_eq!(compare("6?83", &[0x6A, 0x83]), None);
        assert_eq!(compare("6A *", &[0x6A, 0x82]), None);
        assert_eq!(compare("*", &[]), None);
        assert_eq!(
            compare("90 00", &[0x6A, 0x82]).as_deref(),
            Some("byte 1 is 6A, expected 90")
        );
        assert_eq!(
            compare("6A 83", &[0x6A, 0x82]).as_deref(),
            Some("byte 2 is 82, expected 83")
        );
        assert_eq!(
            compare("90 00 00", &[0x90, 0x00]).as_deref(),
            Some("the answer has 2 bytes, expected 3")
        );
        assert_eq!(
            compare("90", &[0x90, 0x00]).as_deref(),
            Some("the answer has 2 bytes, expected 1")
        );
        assert_eq!(
            compare("6F 1? *", &[0x6F]).as_deref(),
            Some("the answer has 1 byte, expected at least 2")
        );
        assert!(compare("90 0X", &[0x90, 0x00])
            .is_some_and(|m| m.starts_with("the expected answer isn't valid")));
    }

    #[test]
    fn hex_text() {
        assert_eq!(hex_spaced(&[0x90, 0x0A]), "90 0A");
        assert_eq!(hex_compact(&[0x90, 0x0A]), "900A");
        assert!(matches!(parse_hex("00 a4"), Ok(b) if b == [0x00, 0xA4]));
        assert!(matches!(parse_hex("00A"), Err(HexError::Incomplete)));
        assert!(matches!(parse_hex("0G"), Err(HexError::NotHex)));
    }

    /// Multi-byte characters anywhere must not panic.
    #[test]
    fn text_with_accents_does_not_panic() {
        assert!(parse(&script("step: é\ncard: 00é\n")).is_err());
        assert!(parse(&script("card: <é> 00\n")).is_ok());
        assert!(compare("é", &[0x90]).is_some());
    }
}
