// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! Reading APDUs the ISO/IEC 7816-4 way: the status word's meaning, the command's fields and
//! instruction name, and a hex dump. Plain functions over bytes; the window shows them.

/// What a status word part means when it is a count or a value.
#[derive(Clone, Copy)]
enum Arg {
    None,
    /// SW2 as a number (61XX, 6CXX).
    Sw2,
    /// The low nibble of SW2 (63CX).
    Nibble,
}

/// One rule: the status words `sw & mask == value` mean `text` (`{}` is the argument).
struct Rule {
    value: u16,
    mask: u16,
    arg: Arg,
    text: &'static str,
}

const fn exact(value: u16, text: &'static str) -> Rule {
    Rule {
        value,
        mask: 0xFFFF,
        arg: Arg::None,
        text,
    }
}

/// Most specific first: exact values, then patterns.
static SW_RULES: &[Rule] = &[
    exact(0x9000, "Success"),
    exact(0x6200, "Warning: no information given, memory unchanged"),
    exact(
        0x6281,
        "Warning: part of the returned data may be corrupted",
    ),
    exact(
        0x6282,
        "Warning: end of file or record reached before Le bytes",
    ),
    exact(0x6283, "Warning: selected file deactivated"),
    exact(0x6284, "Warning: file control information not formatted"),
    exact(0x6285, "Warning: selected file in termination state"),
    exact(0x6286, "Warning: no input data available from a sensor"),
    exact(0x6300, "Warning: no information given, memory changed"),
    exact(0x6381, "Warning: file filled up by the last write"),
    exact(0x6400, "Execution error: memory unchanged"),
    exact(
        0x6401,
        "Execution error: immediate response required by the card",
    ),
    exact(0x6500, "Execution error: memory changed"),
    exact(0x6581, "Execution error: memory failure"),
    exact(0x6700, "Wrong length"),
    exact(0x6800, "Functions in CLA not supported"),
    exact(0x6881, "Logical channel not supported"),
    exact(0x6882, "Secure messaging not supported"),
    exact(0x6883, "Last command of the chain expected"),
    exact(0x6884, "Command chaining not supported"),
    exact(0x6900, "Command not allowed"),
    exact(0x6981, "Command incompatible with the file structure"),
    exact(0x6982, "Security status not satisfied"),
    exact(0x6983, "Authentication method blocked"),
    exact(0x6984, "Reference data not usable"),
    exact(0x6985, "Conditions of use not satisfied"),
    exact(0x6986, "Command not allowed: no current EF"),
    exact(0x6987, "Expected secure messaging data objects missing"),
    exact(0x6988, "Incorrect secure messaging data objects"),
    exact(0x6A00, "Wrong parameters P1-P2"),
    exact(0x6A80, "Incorrect parameters in the data field"),
    exact(0x6A81, "Function not supported"),
    exact(0x6A82, "File or application not found"),
    exact(0x6A83, "Record not found"),
    exact(0x6A84, "Not enough memory space in the file"),
    exact(0x6A85, "Nc inconsistent with the TLV structure"),
    exact(0x6A86, "Incorrect parameters P1-P2"),
    exact(0x6A87, "Nc inconsistent with P1-P2"),
    exact(0x6A88, "Referenced data or reference data not found"),
    exact(0x6A89, "File already exists"),
    exact(0x6A8A, "DF name already exists"),
    exact(0x6B00, "Wrong parameters P1-P2"),
    exact(0x6D00, "Instruction code not supported or invalid"),
    exact(0x6E00, "Class not supported"),
    exact(0x6F00, "No precise diagnosis"),
    Rule {
        value: 0x6100,
        mask: 0xFF00,
        arg: Arg::Sw2,
        text: "Success, {} more bytes available (GET RESPONSE)",
    },
    Rule {
        value: 0x6C00,
        mask: 0xFF00,
        arg: Arg::Sw2,
        text: "Wrong Le: {} bytes available",
    },
    Rule {
        value: 0x63C0,
        mask: 0xFFF0,
        arg: Arg::Nibble,
        text: "Warning: counter at {} (e.g. tries left)",
    },
    Rule {
        value: 0x6200,
        mask: 0xFF00,
        arg: Arg::None,
        text: "Warning: memory unchanged",
    },
    Rule {
        value: 0x6300,
        mask: 0xFF00,
        arg: Arg::None,
        text: "Warning: memory changed",
    },
    Rule {
        value: 0x6400,
        mask: 0xFF00,
        arg: Arg::None,
        text: "Execution error: memory unchanged",
    },
    Rule {
        value: 0x6500,
        mask: 0xFF00,
        arg: Arg::None,
        text: "Execution error: memory changed",
    },
    Rule {
        value: 0x6800,
        mask: 0xFF00,
        arg: Arg::None,
        text: "Functions in CLA not supported",
    },
    Rule {
        value: 0x6900,
        mask: 0xFF00,
        arg: Arg::None,
        text: "Command not allowed",
    },
    Rule {
        value: 0x6A00,
        mask: 0xFF00,
        arg: Arg::None,
        text: "Wrong parameters",
    },
];

/// The meaning of status word `sw1 sw2`, when ISO 7816-4 defines one.
pub fn sw_meaning(sw1: u8, sw2: u8) -> Option<String> {
    let sw = u16::from_be_bytes([sw1, sw2]);
    let rule = SW_RULES.iter().find(|r| sw & r.mask == r.value)?;
    Some(match rule.arg {
        Arg::None => rule.text.to_string(),
        Arg::Sw2 => {
            let n = if sw2 == 0 { 256 } else { u16::from(sw2) };
            rule.text.replacen("{}", &n.to_string(), 1)
        }
        Arg::Nibble => rule.text.replacen("{}", &(sw2 & 0x0F).to_string(), 1),
    })
}

/// The status word at the end of an answer and its meaning ("6A83", "Record not found").
pub fn sw_of(reply: &[u8]) -> Option<(u8, u8, Option<String>)> {
    match reply {
        [.., a, b] => Some((*a, *b, sw_meaning(*a, *b))),
        _ => None,
    }
}

/// What a status word says about the command, by its ISO/IEC 7816-4 category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwLevel {
    /// 9000, 61xx: normal processing.
    Normal,
    /// 62xx, 63xx: warning processing. Also a status word outside ISO/IEC 7816-4, which the
    /// card's own specification may define as a success.
    Warning,
    /// 64xx to 6Fxx: execution error or checking error. The command wasn't carried out as asked.
    Error,
}

/// The level of status word `sw1 sw2`.
pub fn sw_level(sw1: u8, sw2: u8) -> SwLevel {
    match sw1 {
        0x90 if sw2 == 0 => SwLevel::Normal,
        0x61 => SwLevel::Normal,
        0x64..=0x6F => SwLevel::Error,
        _ => SwLevel::Warning,
    }
}

/// The level of the status word ending `reply`; None when the answer is shorter than one.
pub fn level_of(reply: &[u8]) -> Option<SwLevel> {
    match reply {
        [.., a, b] => Some(sw_level(*a, *b)),
        _ => None,
    }
}

/// Why an answer that nothing was expected of isn't a success: "SW 6D00, an error:
/// Instruction code not supported or invalid". None when its status word is normal processing.
pub fn sw_problem(reply: &[u8]) -> Option<String> {
    let (a, b, meaning) = sw_of(reply)?;
    let sw = format!("SW {a:02X}{b:02X}");
    Some(match (sw_level(a, b), meaning) {
        (SwLevel::Normal, _) => return None,
        // Their meanings already start with "Warning:".
        (SwLevel::Warning, Some(m)) if matches!(a, 0x62 | 0x63) => format!("{sw}: {m}"),
        (SwLevel::Warning, _) if matches!(a, 0x62 | 0x63) => format!("{sw}, a warning"),
        (SwLevel::Warning, _) => format!("{sw}, outside ISO/IEC 7816-4"),
        (SwLevel::Error, Some(m)) => format!("{sw}, an error: {m}"),
        (SwLevel::Error, None) => format!("{sw}, an error"),
    })
}

/// The interindustry instructions, by INS byte: ISO/IEC 7816-4 (2013 edition, which added the
/// record activation and the NEXT DATA commands), and the three of ISO/IEC 7816-7 (SCQL,
/// transaction and user operations). An odd INS is the same command with its data in BER-TLV.
static INSTRUCTIONS: &[(u8, &str)] = &[
    (0x04, "DEACTIVATE FILE"),
    (0x06, "DEACTIVATE RECORD(S)"),
    (0x08, "ACTIVATE RECORD(S)"),
    (0x0C, "ERASE RECORD(S)"),
    (0x0E, "ERASE BINARY"),
    (0x0F, "ERASE BINARY"),
    (0x10, "PERFORM SCQL OPERATION"),
    (0x12, "PERFORM TRANSACTION OPERATION"),
    (0x14, "PERFORM USER OPERATION"),
    (0x20, "VERIFY"),
    (0x21, "VERIFY"),
    (0x22, "MANAGE SECURITY ENVIRONMENT"),
    (0x24, "CHANGE REFERENCE DATA"),
    (0x26, "DISABLE VERIFICATION REQUIREMENT"),
    (0x28, "ENABLE VERIFICATION REQUIREMENT"),
    (0x2A, "PERFORM SECURITY OPERATION"),
    (0x2C, "RESET RETRY COUNTER"),
    (0x44, "ACTIVATE FILE"),
    (0x46, "GENERATE ASYMMETRIC KEY PAIR"),
    (0x70, "MANAGE CHANNEL"),
    (0x82, "EXTERNAL / MUTUAL AUTHENTICATE"),
    (0x84, "GET CHALLENGE"),
    (0x86, "GENERAL AUTHENTICATE"),
    (0x87, "GENERAL AUTHENTICATE"),
    (0x88, "INTERNAL AUTHENTICATE"),
    (0xA0, "SEARCH BINARY"),
    (0xA1, "SEARCH BINARY"),
    (0xA2, "SEARCH RECORD"),
    (0xA4, "SELECT"),
    (0xB0, "READ BINARY"),
    (0xB1, "READ BINARY"),
    (0xB2, "READ RECORD(S)"),
    (0xB3, "READ RECORD(S)"),
    (0xC0, "GET RESPONSE"),
    (0xC2, "ENVELOPE"),
    (0xC3, "ENVELOPE"),
    (0xCA, "GET DATA"),
    (0xCB, "GET DATA"),
    (0xCC, "GET NEXT DATA"),
    (0xCD, "GET NEXT DATA"),
    (0xD0, "WRITE BINARY"),
    (0xD1, "WRITE BINARY"),
    (0xD2, "WRITE RECORD"),
    (0xD6, "UPDATE BINARY"),
    (0xD7, "UPDATE BINARY"),
    (0xD8, "PUT NEXT DATA"),
    (0xD9, "PUT NEXT DATA"),
    (0xDA, "PUT DATA"),
    (0xDB, "PUT DATA"),
    (0xDC, "UPDATE RECORD"),
    (0xDD, "UPDATE RECORD"),
    (0xE0, "CREATE FILE"),
    (0xE2, "APPEND RECORD"),
    (0xE4, "DELETE FILE"),
    (0xE6, "TERMINATE DF"),
    (0xE8, "TERMINATE EF"),
    (0xFE, "TERMINATE CARD USAGE"),
];

/// The name of an interindustry instruction (CLA without the proprietary bit 0x80).
pub fn instruction_name(cla: u8, ins: u8) -> Option<&'static str> {
    if cla & 0x80 != 0 {
        return None;
    }
    INSTRUCTIONS
        .iter()
        .find(|(i, _)| *i == ins)
        .map(|(_, n)| *n)
}

/// A command APDU split into its fields (ISO 7816-4 cases 1 to 4, short or extended).
#[derive(Debug, PartialEq, Eq)]
pub struct Command<'a> {
    pub cla: u8,
    pub ins: u8,
    pub p1: u8,
    pub p2: u8,
    /// Nc: the length of `data` (None: no data field).
    pub lc: Option<usize>,
    pub data: &'a [u8],
    /// Ne: the most bytes expected back (None: none expected).
    pub le: Option<usize>,
    pub extended: bool,
}

/// Splits a command APDU; None when its length matches no ISO case.
pub fn parse_command(b: &[u8]) -> Option<Command<'_>> {
    let (&[cla, ins, p1, p2], body) = (b.first_chunk::<4>()?, b.get(4..)?);
    let cmd = |lc, data, le, extended| Command {
        cla,
        ins,
        p1,
        p2,
        lc,
        data,
        le,
        extended,
    };
    let short_le = |v: u8| if v == 0 { 256 } else { usize::from(v) };
    let long_le = |hi: u8, lo: u8| match usize::from(u16::from_be_bytes([hi, lo])) {
        0 => 65536,
        n => n,
    };
    match body {
        [] => Some(cmd(None, &[], None, false)),
        [le] => Some(cmd(None, &[], Some(short_le(*le)), false)),
        // Extended case 2: 00 Le1 Le2.
        [0, hi, lo] => Some(cmd(None, &[], Some(long_le(*hi, *lo)), true)),
        [0, hi, lo, rest @ ..] => {
            // Extended cases 3 and 4: 00 Lc1 Lc2 data [Le1 Le2].
            let lc = usize::from(u16::from_be_bytes([*hi, *lo]));
            if lc == 0 {
                return None;
            }
            match rest.len().checked_sub(lc)? {
                0 => Some(cmd(Some(lc), rest, None, true)),
                2 => {
                    let data = rest.get(..lc)?;
                    let &[h, l] = rest.get(lc..)? else {
                        return None;
                    };
                    Some(cmd(Some(lc), data, Some(long_le(h, l)), true))
                }
                _ => None,
            }
        }
        [lc, rest @ ..] => {
            let lc = usize::from(*lc);
            match rest.len().checked_sub(lc)? {
                0 => Some(cmd(Some(lc), rest, None, false)),
                1 => {
                    let data = rest.get(..lc)?;
                    let le = *rest.get(lc)?;
                    Some(cmd(Some(lc), data, Some(short_le(le)), false))
                }
                _ => None,
            }
        }
    }
}

/// A hex dump: offset, 16 bytes in two groups of 8, and the printable ASCII.
pub fn hexdump(bytes: &[u8]) -> String {
    let mut out = String::new();
    for (row, chunk) in bytes.chunks(16).enumerate() {
        out.push_str(&format!("{:04X}  ", row * 16));
        for i in 0..16 {
            match chunk.get(i) {
                Some(b) => out.push_str(&format!("{b:02X} ")),
                None => out.push_str("   "),
            }
            if i == 7 {
                out.push(' ');
            }
        }
        out.push(' ');
        out.extend(chunk.iter().map(|&b| {
            if (0x20..0x7F).contains(&b) {
                char::from(b)
            } else {
                '.'
            }
        }));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_words_are_ranked_by_their_iso_category() {
        for (sw1, sw2, level) in [
            (0x90, 0x00, SwLevel::Normal),
            (0x61, 0x10, SwLevel::Normal),
            (0x62, 0x83, SwLevel::Warning),
            (0x63, 0xC2, SwLevel::Warning),
            (0x64, 0x00, SwLevel::Error),
            (0x69, 0x82, SwLevel::Error),
            (0x6C, 0x10, SwLevel::Error),
            (0x6D, 0x00, SwLevel::Error),
            (0x6F, 0x00, SwLevel::Error),
            // Outside ISO/IEC 7816-4: the card's own specification says what it means.
            (0x90, 0x01, SwLevel::Warning),
            (0x91, 0x00, SwLevel::Warning),
            (0x60, 0x00, SwLevel::Warning),
        ] {
            assert_eq!(sw_level(sw1, sw2), level, "{sw1:02X}{sw2:02X}");
        }
        assert_eq!(level_of(&[0x6D]), None);
        assert_eq!(level_of(&[0x01, 0x6D, 0x00]), Some(SwLevel::Error));
    }

    #[test]
    fn a_status_word_that_is_not_a_success_says_why() {
        assert_eq!(sw_problem(&[0x90, 0x00]), None);
        assert_eq!(sw_problem(&[0x61, 0x05]), None);
        assert_eq!(
            sw_problem(&[0x6D, 0x00]).as_deref(),
            Some("SW 6D00, an error: Instruction code not supported or invalid")
        );
        assert_eq!(
            sw_problem(&[0x63, 0xC2]).as_deref(),
            Some("SW 63C2: Warning: counter at 2 (e.g. tries left)")
        );
        assert_eq!(
            sw_problem(&[0x91, 0x00]).as_deref(),
            Some("SW 9100, outside ISO/IEC 7816-4")
        );
        assert_eq!(sw_problem(&[0x90]), None);
    }

    #[test]
    fn status_words_are_explained() {
        assert_eq!(sw_meaning(0x90, 0x00).as_deref(), Some("Success"));
        assert_eq!(sw_meaning(0x6A, 0x83).as_deref(), Some("Record not found"));
        assert_eq!(
            sw_meaning(0x61, 0x05).as_deref(),
            Some("Success, 5 more bytes available (GET RESPONSE)")
        );
        assert_eq!(
            sw_meaning(0x61, 0x00).as_deref(),
            Some("Success, 256 more bytes available (GET RESPONSE)")
        );
        assert_eq!(
            sw_meaning(0x63, 0xC2).as_deref(),
            Some("Warning: counter at 2 (e.g. tries left)")
        );
        assert_eq!(
            sw_meaning(0x6C, 0x10).as_deref(),
            Some("Wrong Le: 16 bytes available")
        );
        assert_eq!(
            sw_meaning(0x69, 0x99).as_deref(),
            Some("Command not allowed"),
            "an unknown 69XX falls back to its group"
        );
        assert_eq!(sw_meaning(0x91, 0x00), None, "proprietary");
        assert_eq!(sw_of(&[0x6A, 0x83]).map(|s| (s.0, s.1)), Some((0x6A, 0x83)));
        assert_eq!(sw_of(&[0x90]), None);
    }

    #[test]
    fn every_exact_rule_comes_before_the_patterns_it_overlaps() {
        for (i, r) in SW_RULES.iter().enumerate() {
            if r.mask != 0xFFFF {
                continue;
            }
            let first = SW_RULES
                .iter()
                .position(|x| r.value & x.mask == x.value)
                .unwrap_or(usize::MAX);
            assert_eq!(first, i, "{:04X} is hidden by an earlier rule", r.value);
        }
    }

    #[test]
    fn commands_split_into_their_fields_for_each_case() {
        let c1 = parse_command(&[0x00, 0x70, 0x80, 0x01]).expect("case 1");
        assert_eq!((c1.lc, c1.le), (None, None));
        let c2 = parse_command(&[0x00, 0x84, 0x00, 0x00, 0x08]).expect("case 2");
        assert_eq!((c2.lc, c2.le), (None, Some(8)));
        let c2z = parse_command(&[0x00, 0xB0, 0x00, 0x00, 0x00]).expect("case 2, Le 0");
        assert_eq!(c2z.le, Some(256));
        let c3 = parse_command(&[0x00, 0xA4, 0x00, 0x00, 0x02, 0x3F, 0x00]).expect("case 3");
        assert_eq!((c3.lc, c3.data, c3.le), (Some(2), &[0x3F, 0x00][..], None));
        let c4 = parse_command(&[0x00, 0xA4, 0x04, 0x00, 0x02, 0xA0, 0x00, 0x00]).expect("4");
        assert_eq!((c4.lc, c4.le), (Some(2), Some(256)));
        let e2 = parse_command(&[0x00, 0xB0, 0x00, 0x00, 0x00, 0x01, 0x00]).expect("ext 2");
        assert_eq!((e2.le, e2.extended), (Some(256), true));
        let e4 =
            parse_command(&[0x00, 0xD6, 0, 0, 0x00, 0x00, 0x01, 0xAA, 0x00, 0x00]).expect("ext 4");
        assert_eq!((e4.lc, e4.data, e4.le), (Some(1), &[0xAA][..], Some(65536)));
        assert_eq!(parse_command(&[0x00, 0xA4, 0x00]), None, "too short");
        assert_eq!(
            parse_command(&[0x00, 0xA4, 0x00, 0x00, 0x05, 0x01]),
            None,
            "Lc says more data than there is"
        );
        assert_eq!(instruction_name(0x00, 0xA4), Some("SELECT"));
        assert_eq!(instruction_name(0x80, 0xA4), None, "proprietary class");
        assert_eq!(instruction_name(0x00, 0x0C), Some("ERASE RECORD(S)"));
        assert_eq!(instruction_name(0x00, 0xB1), Some("READ BINARY"));
        assert_eq!(instruction_name(0x00, 0xFE), Some("TERMINATE CARD USAGE"));
        assert_eq!(instruction_name(0x00, 0x34), None, "not in the table");
    }

    #[test]
    fn the_instruction_table_is_sorted_and_has_no_duplicates() {
        let codes: Vec<u8> = INSTRUCTIONS.iter().map(|(i, _)| *i).collect();
        assert!(codes.windows(2).all(|w| w[0] < w[1]), "{codes:02X?}");
        // An odd INS names the same command as the even one before it, when both exist.
        for (ins, name) in INSTRUCTIONS.iter().filter(|(i, _)| i & 1 == 1) {
            if let Some((_, even)) = INSTRUCTIONS.iter().find(|(i, _)| *i == ins - 1) {
                assert_eq!(name, even, "{ins:02X}");
            }
        }
    }

    #[test]
    fn the_dump_shows_offsets_hex_and_ascii() {
        let d = hexdump(b"SmartCard\x00\x01 dump test bytes!");
        let lines: Vec<&str> = d.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            "0000  53 6D 61 72 74 43 61 72  64 00 01 20 64 75 6D 70  SmartCard.. dump"
        );
        assert!(lines[1].starts_with("0010  20 74 65 73 74"), "{}", lines[1]);
        assert!(lines[1].ends_with(" test bytes!"), "{}", lines[1]);
        assert_eq!(hexdump(&[]), "");
    }
}
