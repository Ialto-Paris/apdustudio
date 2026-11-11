// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! What the bytes mean, by the standards that define them: the ATR (ISO/IEC 7816-3 §8, the
//! historical bytes of ISO/IEC 7816-4 §8, and the ATR a contactless reader builds per PC/SC
//! part 3), the command APDU (ISO/IEC 7816-4 §5) and the status word (ISO/IEC 7816-4 §5.6).
//!
//! Each field says which bytes it covers, what they mean and where the standard defines them.
//! Values outside what the standards define are said to be so, never guessed. The texts are
//! written here from the standards' values; none is copied from them.

use crate::iso;

/// One decoded field, possibly with sub-fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// The bytes it covers, in hex ("3B", "96", "80 5A ...").
    pub bytes: String,
    pub name: String,
    pub meaning: String,
    /// Where the standard defines it ("ISO/IEC 7816-3 §8.3").
    pub reference: &'static str,
    pub children: Vec<Field>,
    /// The bytes break the standard (a wrong TCK, a missing byte, bytes past the end).
    pub problem: bool,
}

/// The name of a response's status word field.
pub const SW: &str = "SW1 SW2";

/// Widest bytes column in `to_text` (8 bytes); longer bytes push their row's other columns on.
const TEXT_BYTES: usize = 3 * 8 - 1;

/// Fields as plain text, one per line, in the detail's columns: the bytes (in full), the name
/// (indented two spaces a level), the meaning, then the standard's clause in brackets. Each
/// column is padded to its widest entry, the bytes column to 8 bytes at most.
pub fn to_text(fields: &[Field]) -> String {
    fn rows<'a>(fs: &'a [Field], depth: usize, out: &mut Vec<(&'a Field, String)>) {
        for f in fs {
            out.push((f, format!("{}{}", "  ".repeat(depth), f.name)));
            // As deep as the detail shows them.
            if depth < 4 {
                rows(&f.children, depth + 1, out);
            }
        }
    }
    let mut all = Vec::new();
    rows(fields, 0, &mut all);
    let bytes_w = all
        .iter()
        .map(|(f, _)| f.bytes.chars().count())
        .max()
        .unwrap_or(0)
        .min(TEXT_BYTES);
    let name_w = all
        .iter()
        .map(|(_, n)| n.chars().count())
        .max()
        .unwrap_or(0);
    let mut s = String::new();
    for (f, name) in all {
        let mut line = format!(
            "{:bytes_w$}  {name:name_w$}  {}",
            f.bytes,
            f.meaning.replace('\n', " ")
        );
        if !f.reference.is_empty() {
            line.push_str(&format!("  [{}]", f.reference));
        }
        s.push_str(line.trim_end());
        s.push('\n');
    }
    s
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn field(
    bytes: &[u8],
    name: impl Into<String>,
    meaning: impl Into<String>,
    reference: &'static str,
) -> Field {
    Field {
        bytes: hex(bytes),
        name: name.into(),
        meaning: meaning.into(),
        reference,
        children: Vec::new(),
        problem: false,
    }
}

/// A field whose bytes break the standard.
fn problem(
    bytes: &[u8],
    name: impl Into<String>,
    meaning: impl Into<String>,
    reference: &'static str,
) -> Field {
    Field {
        problem: true,
        ..field(bytes, name, meaning, reference)
    }
}

const ISO3_ATR: &str = "ISO/IEC 7816-3 §8.2";
const ISO3_GLOBAL: &str = "ISO/IEC 7816-3 §8.3";
const ISO3_T1: &str = "ISO/IEC 7816-3 §11.4";
const ISO3_T15: &str = "ISO/IEC 7816-3 §8.3 (T=15)";
const ISO4_HIST: &str = "ISO/IEC 7816-4 §8.1.1";
const ISO4_CLA: &str = "ISO/IEC 7816-4 §5.4.1";
const ISO4_CMD: &str = "ISO/IEC 7816-4 §5.1";
const ISO4_SW: &str = "ISO/IEC 7816-4 §5.6";
const PCSC3: &str = "PC/SC part 3, §3.1.3.2.3";
const PCSC3_SUP: &str = "PC/SC part 3 supplement";

// ------------------------------------------------------------------------------------- ATR

/// Fi and the matching maximum clock in MHz, by TA1's high nibble (None: reserved).
const FI: [Option<(u16, &str)>; 16] = [
    Some((372, "4")),
    Some((372, "5")),
    Some((558, "6")),
    Some((744, "8")),
    Some((1116, "12")),
    Some((1488, "16")),
    Some((1860, "20")),
    None,
    None,
    Some((512, "5")),
    Some((768, "7.5")),
    Some((1024, "10")),
    Some((1536, "15")),
    Some((2048, "20")),
    None,
    None,
];

/// Di by TA1's low nibble (None: reserved).
const DI: [Option<u8>; 16] = [
    None,
    Some(1),
    Some(2),
    Some(4),
    Some(8),
    Some(16),
    Some(32),
    Some(64),
    Some(12),
    Some(20),
    None,
    None,
    None,
    None,
    None,
    None,
];

fn ta1(b: u8) -> String {
    let f = FI.get(usize::from(b >> 4)).copied().flatten();
    let d = DI.get(usize::from(b & 0x0F)).copied().flatten();
    match (f, d) {
        (Some((fi, fmax)), Some(di)) => format!(
            "Fi = {fi} (f max {fmax} MHz), Di = {di}: {} clock cycles per bit",
            u32::from(fi) / u32::from(di)
        ),
        _ => "Fi or Di reserved for future use".into(),
    }
}

fn protocol_name(t: u8) -> String {
    match t {
        0 => "T=0 (character protocol)".into(),
        1 => "T=1 (block protocol)".into(),
        15 => "T=15 (global interface bytes follow)".into(),
        14 => "T=14 (not standardized by ISO)".into(),
        _ => format!("T={t} (reserved for future use)"),
    }
}

/// The meaning of an interface byte: which one (A..D), its group i, and the protocol T the
/// group belongs to (the T given by TD(i-1); group 1 is global).
fn interface_meaning(letter: char, i: usize, t: u8, b: u8) -> (String, &'static str) {
    match (letter, i, t) {
        ('A', 1, _) => (ta1(b), ISO3_GLOBAL),
        ('B', 1, _) => (
            if b == 0 {
                "No programming voltage (deprecated parameter)".into()
            } else {
                "Programming voltage and current (deprecated: should be absent or 00)".into()
            },
            ISO3_GLOBAL,
        ),
        ('C', 1, _) => (
            if b == 0xFF {
                "N = 255: minimum guard time (12 etu with T=0, 11 etu with T=1)".into()
            } else {
                format!("N = {b}: {b} etu of extra guard time")
            },
            ISO3_GLOBAL,
        ),
        ('A', 2, _) => {
            let can_change = b & 0x80 == 0;
            let implicit = b & 0x10 != 0;
            (
                format!(
                    "Specific mode, protocol T={}; {}; {}",
                    b & 0x0F,
                    if can_change {
                        "the card can change to negotiable mode"
                    } else {
                        "the card cannot change mode"
                    },
                    if implicit {
                        "parameters defined implicitly"
                    } else {
                        "parameters from the interface bytes"
                    }
                ),
                ISO3_GLOBAL,
            )
        }
        ('B', 2, _) => (
            "Programming voltage (deprecated: should be absent)".into(),
            ISO3_GLOBAL,
        ),
        ('C', 2, 0) => (
            format!("WI = {b}: work waiting time of T=0 (960 × {b} × Fi/f)"),
            "ISO/IEC 7816-3 §10.2",
        ),
        ('A', _, 1) => (
            format!("IFSC = {b}: the card's information field size"),
            ISO3_T1,
        ),
        ('B', _, 1) => (
            format!(
                "BWI = {}, CWI = {}: block and character waiting times",
                b >> 4,
                b & 0x0F
            ),
            ISO3_T1,
        ),
        ('C', _, 1) => (
            if b & 1 == 0 {
                "Error detection code: LRC".into()
            } else {
                "Error detection code: CRC".into()
            },
            ISO3_T1,
        ),
        ('A', _, 15) => {
            let clock = match b >> 6 {
                0 => "clock stop not supported",
                1 => "clock stop in state L",
                2 => "clock stop in state H",
                _ => "clock stop with no preference",
            };
            let mut classes = Vec::new();
            for (bit, name) in [(1, "A (5 V)"), (2, "B (3 V)"), (4, "C (1.8 V)")] {
                if b & bit != 0 {
                    classes.push(name);
                }
            }
            let classes = if classes.is_empty() {
                "no class indicated".to_string()
            } else {
                format!("classes {}", classes.join(", "))
            };
            (format!("{clock}; {classes}"), ISO3_T15)
        }
        ('B', _, 15) => (
            if b == 0 {
                "No standard or proprietary use of contact C6".into()
            } else {
                format!("Use of contact C6 (SPU): 0x{b:02X}")
            },
            ISO3_T15,
        ),
        _ => (format!("Specific to T={t}"), ISO3_GLOBAL),
    }
}

/// Decodes an ATR.
pub fn atr(atr: &[u8]) -> Vec<Field> {
    let mut out = Vec::new();
    let Some((&ts, rest)) = atr.split_first() else {
        return out;
    };
    out.push(field(
        &[ts],
        "TS",
        match ts {
            0x3B => "Direct convention",
            0x3F => "Inverse convention",
            _ => "Not a valid initial character",
        },
        ISO3_ATR,
    ));
    let Some((&t0, mut rest)) = rest.split_first() else {
        return out;
    };
    let k = usize::from(t0 & 0x0F);
    out.push(field(
        &[t0],
        "T0",
        format!("{} historical bytes; {}", k, present_list(t0, 1)),
        ISO3_ATR,
    ));
    // Interface bytes.
    let mut y = t0 >> 4;
    let mut i = 1usize;
    let mut t_of_group = 0u8; // group 1 is global
    let mut protocols = Vec::new();
    loop {
        let mut td = None;
        for (bit, letter) in [(1u8, 'A'), (2, 'B'), (4, 'C'), (8, 'D')] {
            if y & bit == 0 {
                continue;
            }
            let Some((&b, r)) = rest.split_first() else {
                out.push(problem(
                    &[],
                    "Malformed",
                    "The ATR ends inside its interface bytes",
                    ISO3_ATR,
                ));
                return out;
            };
            rest = r;
            let name = format!("T{letter}{i}");
            if letter == 'D' {
                let t = b & 0x0F;
                protocols.push(t);
                out.push(field(
                    &[b],
                    name,
                    format!("{}; {}", protocol_name(t), present_list(b, i + 1)),
                    ISO3_ATR,
                ));
                td = Some(b);
            } else {
                let (meaning, reference) = interface_meaning(letter, i, t_of_group, b);
                out.push(field(&[b], name, meaning, reference));
            }
        }
        match td {
            Some(b) => {
                y = b >> 4;
                t_of_group = b & 0x0F;
                i += 1;
            }
            None => break,
        }
        if i > 32 {
            break;
        }
    }
    // Historical bytes.
    let hist = rest.get(..k.min(rest.len())).unwrap_or_default();
    rest = rest.get(hist.len()..).unwrap_or_default();
    if !hist.is_empty() {
        let mut h = field(
            hist,
            "Historical bytes",
            format!("{} bytes", hist.len()),
            ISO4_HIST,
        );
        h.children = historical(hist);
        out.push(h);
    }
    if hist.len() < k {
        out.push(problem(
            &[],
            "Malformed",
            format!("{} historical bytes missing", k - hist.len()),
            ISO3_ATR,
        ));
    }
    // TCK: present unless only T=0 is indicated.
    let needs_tck = protocols.iter().any(|t| *t != 0);
    match (rest.split_first(), needs_tck) {
        (Some((&tck, extra)), true) => {
            let sum = atr
                .get(1..)
                .unwrap_or_default()
                .iter()
                .fold(0u8, |a, b| a ^ b);
            out.push(Field {
                problem: sum != 0,
                ..field(
                    &[tck],
                    "TCK",
                    if sum == 0 {
                        "Check byte: correct (the XOR from T0 to TCK is 0)".to_string()
                    } else {
                        format!("Check byte: wrong (the XOR from T0 to TCK is {sum:02X}, not 00)")
                    },
                    ISO3_ATR,
                )
            });
            if !extra.is_empty() {
                out.push(problem(
                    extra,
                    "Malformed",
                    "Bytes after the end of the ATR",
                    ISO3_ATR,
                ));
            }
        }
        (None, true) => out.push(problem(
            &[],
            "TCK",
            "Missing (required when a protocol other than T=0 is indicated)",
            ISO3_ATR,
        )),
        (Some(_), false) => out.push(problem(
            rest,
            "Malformed",
            "Bytes after the end of the ATR (no TCK with T=0 only)",
            ISO3_ATR,
        )),
        (None, false) => {}
    }
    if let Some(cl) = contactless(atr) {
        out.push(cl);
    }
    out
}

/// "TA1, TD1 present" for the presence bits of a T0 or TD byte, group `i`.
fn present_list(b: u8, i: usize) -> String {
    let names: Vec<String> = [(0x10, 'A'), (0x20, 'B'), (0x40, 'C'), (0x80, 'D')]
        .into_iter()
        .filter(|(bit, _)| b & bit != 0)
        .map(|(_, l)| format!("T{l}{i}"))
        .collect();
    if names.is_empty() {
        "no further interface bytes".into()
    } else {
        format!("{} present", names.join(", "))
    }
}

/// The COMPACT-TLV objects of ISO/IEC 7816-4, by tag (high nibble).
fn compact_tlv_name(tag: u8) -> &'static str {
    match tag {
        0x1 => "Country code",
        0x2 => "Issuer identification number",
        0x3 => "Card service data",
        0x4 => "Initial access data",
        0x5 => "Card issuer's data",
        0x6 => "Pre-issuing data",
        0x7 => "Card capabilities",
        0x8 => "Status indicator",
        0xF => "Application identifier",
        _ => "Reserved tag",
    }
}

fn historical(h: &[u8]) -> Vec<Field> {
    let Some((&cat, body)) = h.split_first() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match cat {
        0x80 => {
            out.push(field(
                &[cat],
                "Category indicator",
                "COMPACT-TLV data objects follow",
                ISO4_HIST,
            ));
            out.extend(compact_tlv(body));
        }
        0x00 => {
            out.push(field(
                &[cat],
                "Category indicator",
                "COMPACT-TLV data objects, then a 3-byte status indicator",
                ISO4_HIST,
            ));
            let n = body.len().saturating_sub(3);
            out.extend(compact_tlv(body.get(..n).unwrap_or_default()));
            if let Some(st) = body.get(n..).filter(|s| s.len() == 3) {
                out.push(status_indicator(st));
            }
        }
        0x10 => {
            out.push(field(
                &[cat],
                "Category indicator",
                "A DIR data reference follows",
                ISO4_HIST,
            ));
            if !body.is_empty() {
                out.push(field(
                    body,
                    "DIR data reference",
                    "Reference of the card's DIR data",
                    ISO4_HIST,
                ));
            }
        }
        0x81..=0x8F => out.push(field(
            h,
            "Category indicator",
            "Reserved for future use",
            ISO4_HIST,
        )),
        _ => out.push(field(
            h,
            "Proprietary",
            "Category indicator outside ISO/IEC 7816-4: the content is the issuer's",
            ISO4_HIST,
        )),
    }
    out
}

fn compact_tlv(mut b: &[u8]) -> Vec<Field> {
    let mut out = Vec::new();
    while let Some((&tl, rest)) = b.split_first() {
        let tag = tl >> 4;
        let len = usize::from(tl & 0x0F);
        let Some(value) = rest.get(..len) else {
            out.push(problem(
                b,
                "Malformed",
                "A COMPACT-TLV object runs past the historical bytes",
                ISO4_HIST,
            ));
            break;
        };
        let all = b.get(..=len).unwrap_or_default();
        let name = compact_tlv_name(tag);
        let mut f = match tag {
            0x7 => {
                let mut f = field(all, name, hex(value), ISO4_HIST);
                f.children = card_capabilities(value);
                f
            }
            0x8 => {
                let mut f = field(all, name, "", ISO4_HIST);
                f.meaning = status_indicator(value).meaning;
                f
            }
            _ => field(all, name, hex(value), ISO4_HIST),
        };
        if f.meaning.is_empty() {
            f.meaning = hex(value);
        }
        out.push(f);
        b = rest.get(len..).unwrap_or_default();
    }
    out
}

fn status_indicator(v: &[u8]) -> Field {
    let meaning = match v {
        [lcs] => format!("Life cycle status 0x{lcs:02X}"),
        [a, b] => format!("SW {a:02X}{b:02X}"),
        [lcs, a, b] => format!("Life cycle status 0x{lcs:02X}, SW {a:02X}{b:02X}"),
        _ => hex(v),
    };
    field(v, "Status indicator", meaning, ISO4_HIST)
}

fn card_capabilities(v: &[u8]) -> Vec<Field> {
    let mut out = Vec::new();
    if let Some(&b) = v.first() {
        let flags = [
            (0x80, "DF selection by full DF name"),
            (0x40, "by partial DF name"),
            (0x20, "by path"),
            (0x10, "by file identifier"),
            (0x08, "implicit DF selection"),
            (0x04, "short EF identifier supported"),
            (0x02, "record number supported"),
            (0x01, "record identifier supported"),
        ];
        out.push(field(&[b], "Selection methods", bits(b, &flags), ISO4_HIST));
    }
    if let Some(&b) = v.get(1) {
        out.push(field(
            &[b],
            "Data coding byte",
            format!("0x{b:02X}"),
            ISO4_HIST,
        ));
    }
    if let Some(&b) = v.get(2) {
        let mut parts = Vec::new();
        if b & 0x80 != 0 {
            parts.push("command chaining".to_string());
        }
        if b & 0x40 != 0 {
            parts.push("extended Lc and Le".to_string());
        }
        let channels = match (b >> 3) & 0x03 {
            0 => "no logical channel",
            1 => "logical channels assigned by the interface device",
            2 => "logical channels assigned by the card",
            _ => "logical channels assigned by the card or the interface device",
        };
        parts.push(channels.into());
        parts.push(format!("up to {} logical channels", (b & 0x07) + 1));
        out.push(field(&[b], "Third byte", parts.join(", "), ISO4_HIST));
    }
    out
}

fn bits(b: u8, flags: &[(u8, &str)]) -> String {
    let on: Vec<&str> = flags
        .iter()
        .filter(|(m, _)| b & m != 0)
        .map(|(_, n)| *n)
        .collect();
    if on.is_empty() {
        "none".into()
    } else {
        on.join(", ")
    }
}

/// Standards of a contactless storage card, by PC/SC part 3's SS byte.
fn storage_standard(ss: u8) -> &'static str {
    match ss {
        0x01 => "ISO/IEC 14443 A, part 1",
        0x02 => "ISO/IEC 14443 A, part 2",
        0x03 => "ISO/IEC 14443 A, part 3",
        0x05 => "ISO/IEC 14443 B, part 1",
        0x06 => "ISO/IEC 14443 B, part 2",
        0x07 => "ISO/IEC 14443 B, part 3",
        0x09 => "ISO/IEC 15693, part 1",
        0x0A => "ISO/IEC 15693, part 2",
        0x0B => "ISO/IEC 15693, part 3",
        0x0C => "ISO/IEC 15693, part 4",
        0x11 => "FeliCa",
        _ => "not in PC/SC part 3's list",
    }
}

/// Card names of PC/SC part 3's NN NN bytes (the common ones).
fn storage_card(nn: u16) -> &'static str {
    match nn {
        0x0001 => "MIFARE Classic 1K",
        0x0002 => "MIFARE Classic 4K",
        0x0003 => "MIFARE Ultralight",
        0x0026 => "MIFARE Mini",
        0x003A => "MIFARE Ultralight C",
        0xF004 => "Topaz / Jewel",
        0xF011 => "FeliCa 212 kbit/s",
        0xF012 => "FeliCa 424 kbit/s",
        _ => "not in this list",
    }
}

/// The ATR a contactless reader builds per PC/SC part 3: 3B 8n 80 01, then n historical bytes.
fn contactless(atr: &[u8]) -> Option<Field> {
    let [0x3B, t0, 0x80, 0x01, hist @ ..] = atr else {
        return None;
    };
    if t0 >> 4 != 0x8 {
        return None;
    }
    let n = usize::from(t0 & 0x0F);
    let h = hist.get(..n)?;
    let mut f = field(
        atr.get(..4).unwrap_or_default(),
        "Contactless card",
        "",
        PCSC3,
    );
    if let [0x80, 0x4F, 0x0C, 0xA0, 0x00, 0x00, 0x03, 0x06, ss, n1, n2, ..] = h {
        let nn = u16::from_be_bytes([*n1, *n2]);
        f.meaning = format!(
            "Storage card (PC/SC RID A0 00 00 03 06): {}, {}",
            storage_standard(*ss),
            storage_card(nn)
        );
        f.children = vec![
            field(&[*ss], "Standard (SS)", storage_standard(*ss), PCSC3_SUP),
            field(&[*n1, *n2], "Card name (NN)", storage_card(nn), PCSC3_SUP),
        ];
    } else if let [a0, a1, a2, a3, p0, p1, p2, mbli] = h {
        f.meaning = "ISO/IEC 14443-4 card, probably type B: the reader copied the ATQB".into();
        f.children = vec![
            field(
                &[*a0, *a1, *a2, *a3],
                "Application data",
                "From the ATQB",
                PCSC3,
            ),
            field(
                &[*p0, *p1, *p2],
                "Protocol info",
                format!(
                    "Bit rates 0x{p0:02X}; frame size index {}, protocol type {}; FWI {}",
                    p1 >> 4,
                    p1 & 0x0F,
                    p2 >> 4
                ),
                PCSC3,
            ),
            field(
                &[*mbli],
                "MBLI and CID",
                format!("MBLI {}, CID {}", mbli >> 4, mbli & 0x0F),
                PCSC3,
            ),
        ];
    } else {
        f.meaning =
            "ISO/IEC 14443-4 card, type A: the historical bytes are those of its ATS".into();
    }
    Some(f)
}

// ------------------------------------------------------------------------------------- APDU

fn class(cla: u8) -> Field {
    let meaning = match cla {
        0xFF => {
            "Invalid in ISO/IEC 7816-4; used by PC/SC part 3 for commands to the reader".to_string()
        }
        0x00..=0x1F => {
            let sm = match (cla >> 2) & 0x03 {
                0 => "no secure messaging",
                1 => "proprietary secure messaging",
                2 => "secure messaging, header not processed",
                _ => "secure messaging, header authenticated",
            };
            format!(
                "Inter-industry; {}; {}; logical channel {}",
                if cla & 0x10 != 0 {
                    "chained: more commands follow"
                } else {
                    "last or only command of a chain"
                },
                sm,
                cla & 0x03
            )
        }
        0x40..=0x7F => format!(
            "Inter-industry (further); {}; {}; logical channel {}",
            if cla & 0x20 != 0 {
                "secure messaging, header not processed"
            } else {
                "no secure messaging"
            },
            if cla & 0x10 != 0 {
                "chained: more commands follow"
            } else {
                "last or only command of a chain"
            },
            (cla & 0x0F) + 4
        ),
        0x20..=0x3F => "Reserved for future use".to_string(),
        _ => "Proprietary class: its commands are the card issuer's".to_string(),
    };
    field(&[cla], "CLA", meaning, ISO4_CLA)
}

fn pcsc_pseudo(ins: u8, p1: u8) -> Option<String> {
    Some(match ins {
        0xCA => match p1 {
            0x00 => "GET DATA: the card's UID".into(),
            0x01 => "GET DATA: the historical bytes of the ATS (type A)".into(),
            _ => format!("GET DATA, P1 0x{p1:02X}"),
        },
        0x82 => "LOAD KEYS".into(),
        0x86 => "GENERAL AUTHENTICATE".into(),
        0xB0 => "READ BINARY (storage card)".into(),
        0xD6 => "UPDATE BINARY (storage card)".into(),
        _ => return None,
    })
}

fn p1p2(cla: u8, ins: u8, p1: u8, p2: u8) -> Vec<Field> {
    if cla == 0xFF || cla & 0x80 != 0 {
        return vec![
            field(&[p1], "P1", "Defined by the command", ISO4_CMD),
            field(&[p2], "P2", "Defined by the command", ISO4_CMD),
        ];
    }
    let (m1, m2) = match ins {
        0xA4 => (
            match p1 {
                0x00 => "Select by file identifier (MF, DF or EF)",
                0x01 => "Select a child DF",
                0x02 => "Select an EF under the current DF",
                0x03 => "Select the parent DF",
                0x04 => "Select by DF name",
                0x08 => "Select by path from the MF",
                0x09 => "Select by path from the current DF",
                _ => "Reserved selection",
            }
            .to_string(),
            format!(
                "{}; {}",
                match p2 & 0x03 {
                    0 => "first or only occurrence",
                    1 => "last occurrence",
                    2 => "next occurrence",
                    _ => "previous occurrence",
                },
                match (p2 >> 2) & 0x03 {
                    0 => "return the FCI",
                    1 => "return the FCP",
                    2 => "return the FMD",
                    _ => "no response data, or proprietary",
                }
            ),
        ),
        // Odd INS of the data unit and data object commands: P1-P2 name the file, and the
        // offset or tags are in the data field.
        0xB1 | 0xD7 | 0xD1 | 0x0F | 0xA1 | 0xCB | 0xDB | 0xCD | 0xD9 => {
            let fid = u16::from_be_bytes([p1, p2]);
            let what = match fid {
                0x0000 => "the current EF".to_string(),
                0x3FFF => "the current DF".to_string(),
                _ if p1 == 0 && p2 >> 5 == 0 && p2 != 0 => {
                    format!("short EF identifier {}", p2 & 0x1F)
                }
                _ => format!("file identifier {fid:04X}"),
            };
            (
                format!("With P2: {what}"),
                "With P1 (the file); the offset or tags are in the data field".to_string(),
            )
        }
        0xB0 | 0xD6 | 0xD0 | 0x0E | 0xA0 => {
            if p1 & 0x80 != 0 {
                (
                    format!("Short EF identifier {}", p1 & 0x1F),
                    format!("Offset {p2}"),
                )
            } else {
                let off = u16::from_be_bytes([p1 & 0x7F, p2]);
                (
                    format!("Offset {off} (high byte)"),
                    format!("Offset {off} (low byte)"),
                )
            }
        }
        0xB2 | 0xB3 | 0xDC | 0xDD | 0xD2 | 0xE2 | 0xA2 | 0x0C | 0x06 | 0x08 => {
            let sfi = p2 >> 3;
            let ef = if sfi == 0 {
                "current EF".to_string()
            } else {
                format!("short EF identifier {sfi}")
            };
            let mode = match p2 & 0x07 {
                4 => "record number P1",
                5 => "records from P1 to the last",
                6 => "records from the last to P1",
                0 => "first occurrence of record identifier P1",
                1 => "last occurrence of record identifier P1",
                2 => "next occurrence of record identifier P1",
                3 => "previous occurrence of record identifier P1",
                _ => "reserved",
            };
            (
                if p1 == 0 {
                    "Current record".to_string()
                } else {
                    format!("Record {p1}")
                },
                format!("{ef}; {mode}"),
            )
        }
        0x20 | 0x21 | 0x24 | 0x26 | 0x28 | 0x2C | 0x82 | 0x88 => (
            if p1 == 0 {
                "No information given".to_string()
            } else {
                format!("Algorithm or qualifier 0x{p1:02X}")
            },
            if p2 == 0 {
                "No information given".to_string()
            } else {
                format!(
                    "{} reference {}",
                    if p2 & 0x80 != 0 { "Specific" } else { "Global" },
                    p2 & 0x1F
                )
            },
        ),
        0xCA | 0xDA | 0xCC | 0xD8 => (
            format!("Tag {:04X} (high byte)", u16::from_be_bytes([p1, p2])),
            format!("Tag {:04X} (low byte)", u16::from_be_bytes([p1, p2])),
        ),
        0x70 => (
            match p1 {
                0x00 => "Open a logical channel",
                0x80 => "Close a logical channel",
                _ => "Reserved",
            }
            .to_string(),
            if p2 == 0 {
                "Channel chosen by the card".to_string()
            } else {
                format!("Channel {p2}")
            },
        ),
        _ => (
            "Defined by the command".to_string(),
            "Defined by the command".to_string(),
        ),
    };
    vec![
        field(&[p1], "P1", m1, ISO4_CMD),
        field(&[p2], "P2", m2, ISO4_CMD),
    ]
}

/// Decodes a command APDU.
pub fn command(apdu: &[u8]) -> Vec<Field> {
    let Some(c) = iso::parse_command(apdu) else {
        return vec![field(
            apdu,
            "Command",
            format!(
                "{} bytes: no ISO/IEC 7816-4 case matches this length",
                apdu.len()
            ),
            ISO4_CMD,
        )];
    };
    let mut out = vec![class(c.cla)];
    let ins_meaning = if c.cla == 0xFF {
        pcsc_pseudo(c.ins, c.p1).unwrap_or_else(|| "Not a PC/SC part 3 command".into())
    } else if c.ins >> 4 == 6 || c.ins >> 4 == 9 {
        "Invalid instruction (6X and 9X are T=0 procedure bytes)".into()
    } else {
        let name = iso::instruction_name(c.cla, c.ins).unwrap_or(if c.cla & 0x80 != 0 {
            "Proprietary instruction"
        } else {
            "Instruction not in this list"
        });
        if c.ins & 1 == 1 && c.cla & 0x80 == 0 {
            format!("{name} (odd INS: data in BER-TLV)")
        } else {
            name.to_string()
        }
    };
    out.push(field(
        &[c.ins],
        "INS",
        ins_meaning,
        if c.cla == 0xFF { PCSC3_SUP } else { ISO4_CMD },
    ));
    out.extend(p1p2(c.cla, c.ins, c.p1, c.p2));
    let case = match (c.lc.is_some(), c.le.is_some()) {
        (false, false) => 1,
        (false, true) => 2,
        (true, false) => 3,
        (true, true) => 4,
    };
    let length = if c.extended { "extended" } else { "short" };
    // Each part shows its own bytes: Lc (1 byte, or 00 and 2 bytes when extended), the data,
    // then Le (what is left).
    let mut b = field(
        &[],
        "Body",
        format!("Case {case}, {length} lengths"),
        ISO4_CMD,
    );
    let mut le_at = 4usize;
    if let Some(lc) = c.lc {
        let lc_len = if c.extended { 3 } else { 1 };
        let lc_bytes = apdu.get(4..4 + lc_len).unwrap_or_default();
        b.children.push(field(
            lc_bytes,
            "Lc",
            format!("{lc} bytes of data"),
            ISO4_CMD,
        ));
        b.children
            .push(field(c.data, "Data", format!("{lc} bytes"), ISO4_CMD));
        le_at = 4 + lc_len + c.data.len();
    }
    if let Some(le) = c.le {
        b.children.push(field(
            apdu.get(le_at..).unwrap_or_default(),
            "Le",
            format!("Up to {le} bytes expected back"),
            ISO4_CMD,
        ));
    }
    if case > 1 {
        out.push(b);
    } else {
        out.push(field(
            &[],
            "Body",
            "Case 1: no data, no answer data expected",
            ISO4_CMD,
        ));
    }
    out
}

/// Decodes a response: its data and its status word.
pub fn response(resp: &[u8]) -> Vec<Field> {
    let Some((data, sw)) = resp.split_last_chunk::<2>() else {
        return vec![field(resp, "Answer", "Shorter than a status word", ISO4_SW)];
    };
    let mut out = Vec::new();
    if !data.is_empty() {
        let mut d = field(&[], "Data", format!("{} bytes", data.len()), ISO4_SW);
        if let Some(objects) = crate::tlv::decode(data) {
            d.meaning = format!("{} bytes, read as BER-TLV", data.len());
            d.children = objects;
        }
        out.push(d);
    }
    let [a, b] = *sw;
    let category = match a {
        0x90 if b == 0 => "Normal processing",
        0x61 => "Normal processing: more data available",
        0x62 => "Warning: the card's state is unchanged",
        0x63 => "Warning: the card's state has changed",
        0x64 => "Execution error: the card's state is unchanged",
        0x65 => "Execution error: the card's state has changed",
        0x66 => "Execution error: security related",
        0x67..=0x6F => "Checking error",
        _ => "Not an ISO/IEC 7816-4 status word: proprietary",
    };
    let mut meaning = category.to_string();
    if let Some(m) = iso::sw_meaning(a, b) {
        meaning = format!("{category}. {m}");
    }
    match a {
        0x61 => meaning.push_str(&format!(
            ". Send GET RESPONSE with Le = {:02X} to read them",
            b
        )),
        0x6C => meaning.push_str(&format!(". Send the command again with Le = {b:02X}")),
        _ => {}
    }
    out.push(field(&[a, b], SW, meaning, ISO4_SW));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(fs: &'a [Field], name: &str) -> &'a Field {
        fs.iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("no {name} in {fs:#?}"))
    }

    #[test]
    fn a_sam_atr_with_compact_tlv() {
        // Calypso SAM: TA1 96, TB1 00, 15 historical bytes in COMPACT-TLV, no TCK (T=0 only).
        let a = [
            0x3B, 0x3F, 0x96, 0x00, 0x80, 0x5A, 0x48, 0x80, 0xC1, 0x20, 0x50, 0x17, 0xAE, 0xC1,
            0x1A, 0x5C, 0x82, 0x90, 0x00,
        ];
        let f = atr(&a);
        assert_eq!(find(&f, "TS").meaning, "Direct convention");
        assert!(find(&f, "T0")
            .meaning
            .starts_with("15 historical bytes; TA1, TB1 present"));
        assert!(
            find(&f, "TA1")
                .meaning
                .contains("Fi = 512 (f max 5 MHz), Di = 32"),
            "{f:#?}"
        );
        let h = &find(&f, "Historical bytes").children;
        assert_eq!(
            find(h, "Category indicator").meaning,
            "COMPACT-TLV data objects follow"
        );
        assert_eq!(
            find(h, "Card issuer's data").bytes,
            "5A 48 80 C1 20 50 17 AE C1 1A 5C"
        );
        assert_eq!(find(h, "Status indicator").meaning, "SW 9000");
        assert!(f.iter().all(|x| x.name != "TCK" && !x.problem), "{f:#?}");
    }

    #[test]
    fn a_t1_atr_checks_its_tck_and_reads_t1_bytes() {
        // The demo card's: TD1 80 (TD2 follows, T=0), TD2 01 (T=1), 8 historical bytes, TCK.
        let ok = crate::demo::ATR.to_vec();
        let f = atr(&ok);
        assert_eq!(
            find(&f, "Historical bytes").bytes,
            "00 00 00 00 77 81 80 00"
        );
        assert!(find(&f, "TD2").meaning.starts_with("T=1"), "{f:#?}");
        assert!(find(&f, "TCK").meaning.contains("correct"));
        let mut bad = ok.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(find(&atr(&bad), "TCK").problem);
        assert!(!find(&f, "TCK").problem);
        // T=1 interface bytes: TA3 IFSC, TB3 BWI/CWI, TC3 LRC.
        let t1 = [0x3B, 0x80, 0x81, 0x31, 0xFE, 0x45, 0x00];
        let f = atr(&t1);
        assert_eq!(
            find(&f, "TA3").meaning,
            "IFSC = 254: the card's information field size"
        );
        assert!(find(&f, "TB3").meaning.starts_with("BWI = 4, CWI = 5"));
    }

    #[test]
    fn contactless_atrs_from_pc_sc_part_3() {
        // Type A (ATS historical bytes) and type B (ATQB), from a SpringCard reader.
        let a = [0x3B, 0x85, 0x80, 0x01, 0x20, 0x63, 0xCB, 0x20, 0xC1, 0x6D];
        let f = atr(&a);
        assert!(
            find(&f, "Contactless card").meaning.contains("type A"),
            "{f:#?}"
        );
        assert!(find(&f, "TCK").meaning.contains("correct"));
        let b = [
            0x3B, 0x88, 0x80, 0x01, 0, 0, 0, 0, 0x00, 0x71, 0x71, 0x00, 0x09,
        ];
        let f = atr(&b);
        let cl = find(&f, "Contactless card");
        assert!(cl.meaning.contains("type B"), "{cl:#?}");
        assert!(find(&cl.children, "Protocol info")
            .meaning
            .contains("frame size index 7"));
        // A MIFARE Classic 1K: storage card with PC/SC's RID.
        let m = [
            0x3B, 0x8F, 0x80, 0x01, 0x80, 0x4F, 0x0C, 0xA0, 0x00, 0x00, 0x03, 0x06, 0x03, 0x00,
            0x01, 0x00, 0x00, 0x00, 0x00, 0x6A,
        ];
        let cl = atr(&m);
        assert!(
            find(&cl, "Contactless card")
                .meaning
                .contains("MIFARE Classic 1K"),
            "{cl:#?}"
        );
    }

    #[test]
    fn short_or_odd_atrs_say_so_and_never_panic() {
        assert!(atr(&[]).is_empty());
        assert_eq!(atr(&[0x3B]).len(), 1);
        // T0 announces TA1 and 5 historical bytes that aren't there.
        let f = atr(&[0x3B, 0x15]);
        assert!(f
            .iter()
            .any(|x| x.meaning.contains("ends inside its interface bytes")));
        let f = atr(&[0x3B, 0x05, 0x01]);
        assert!(f
            .iter()
            .any(|x| x.meaning.contains("historical bytes missing")));
        // Every TD present: the loop is bounded.
        let mut long = vec![0x3B, 0x80];
        long.extend(std::iter::repeat_n(0x81, 100));
        let _ = atr(&long);
    }

    #[test]
    fn odd_instructions_name_a_file_in_p1_p2() {
        let f = command(&[0x00, 0xB1, 0x00, 0x05, 0x04, 0x54, 0x02, 0x00, 0x10]);
        assert_eq!(
            find(&f, "INS").meaning,
            "READ BINARY (odd INS: data in BER-TLV)"
        );
        assert_eq!(find(&f, "P1").meaning, "With P2: short EF identifier 5");
        let f = command(&[0x00, 0xCB, 0x3F, 0xFF, 0x02, 0x5C, 0x00]);
        assert_eq!(find(&f, "P1").meaning, "With P2: the current DF");
        let f = command(&[0x00, 0xCB, 0x2F, 0x00, 0x02, 0x5C, 0x00]);
        assert_eq!(find(&f, "P1").meaning, "With P2: file identifier 2F00");
        // GET DATA with an even INS: P1-P2 is a tag.
        let f = command(&[0x00, 0xCA, 0x9F, 0x7F, 0x00]);
        assert!(find(&f, "P1").meaning.starts_with("Tag 9F7F"));
        let f = command(&[0x00, 0x0C, 0x03, 0x0D]);
        assert_eq!(find(&f, "INS").meaning, "ERASE RECORD(S)");
        assert_eq!(
            find(&f, "P2").meaning,
            "short EF identifier 1; records from P1 to the last"
        );
    }

    #[test]
    fn command_headers_decode_by_iso_7816_4() {
        let f = command(&[
            0x00, 0xA4, 0x04, 0x00, 0x07, 0xA0, 0, 0, 0, 0x04, 0x10, 0x10,
        ]);
        assert!(find(&f, "CLA").meaning.contains("logical channel 0"));
        assert_eq!(find(&f, "INS").meaning, "SELECT");
        assert_eq!(find(&f, "P1").meaning, "Select by DF name");
        assert_eq!(
            find(&f, "P2").meaning,
            "first or only occurrence; return the FCI"
        );
        assert_eq!(find(&f, "Body").meaning, "Case 3, short lengths");
        let body = &find(&f, "Body").children;
        assert_eq!(find(body, "Lc").bytes, "07");
        assert_eq!(find(body, "Data").bytes, "A0 00 00 00 04 10 10");
        // Case 4 extended: Lc 00 00 02, data, Le 01 00.
        let f = command(&[0x00, 0x88, 0, 0, 0x00, 0x00, 0x02, 0xAB, 0xCD, 0x01, 0x00]);
        let body = &find(&f, "Body").children;
        assert_eq!(find(&f, "Body").meaning, "Case 4, extended lengths");
        assert_eq!(find(body, "Lc").bytes, "00 00 02");
        assert_eq!(find(body, "Le").bytes, "01 00");
        let f = command(&[0x00, 0xB2, 0x01, 0x0C, 0x00]);
        assert_eq!(
            find(&f, "P2").meaning,
            "short EF identifier 1; record number P1"
        );
        assert!(find(&f, "Body")
            .children
            .iter()
            .any(|c| c.meaning.contains("256")));
        let f = command(&[0xFF, 0xCA, 0x00, 0x00, 0x00]);
        assert!(find(&f, "CLA").meaning.contains("PC/SC part 3"));
        assert_eq!(find(&f, "INS").meaning, "GET DATA: the card's UID");
        let f = command(&[0x80, 0xCA, 0x9F, 0x36, 0x00]);
        assert!(find(&f, "CLA").meaning.starts_with("Proprietary"));
        let f = command(&[0x00, 0x20, 0x00, 0x81, 0x02, 0x12, 0x34]);
        assert_eq!(find(&f, "P2").meaning, "Specific reference 1");
        assert!(command(&[0x00, 0xA4, 0x04])
            .first()
            .unwrap()
            .meaning
            .contains("no ISO/IEC 7816-4 case"));
    }

    #[test]
    fn status_words_have_a_category_and_a_next_step() {
        let f = response(&[0x6F, 0x10, 0x90, 0x00]);
        assert_eq!(
            find(&f, "Data").meaning,
            "2 bytes",
            "6F 10: a length past the end"
        );
        let f = response(&[0x6F, 0x03, 0x84, 0x01, 0xA0, 0x90, 0x00]);
        let d = find(&f, "Data");
        assert_eq!(d.meaning, "5 bytes, read as BER-TLV");
        assert_eq!(d.children.len(), 1);
        assert!(find(&f, "SW1 SW2").meaning.starts_with("Normal processing"));
        assert!(find(&response(&[0x61, 0x1C]), "SW1 SW2")
            .meaning
            .contains("GET RESPONSE with Le = 1C"));
        assert!(find(&response(&[0x6C, 0x08]), "SW1 SW2")
            .meaning
            .contains("again with Le = 08"));
        assert!(find(&response(&[0x63, 0xC2]), "SW1 SW2")
            .meaning
            .starts_with("Warning: the card's state has changed"));
        assert!(find(&response(&[0x91, 0x00]), "SW1 SW2")
            .meaning
            .contains("proprietary"));
        assert_eq!(response(&[0x90]).len(), 1);
    }

    #[test]
    fn fields_as_text_line_up_in_columns() {
        let mut parent = field(&[0x3B], "TS", "Direct convention", "ISO/IEC 7816-3 §8.1");
        parent.children = vec![field(&[0x01], "Bit", "One", "")];
        let long = field(&[0xAA; 10], "Data", "Ten bytes", "Ref");
        let text = to_text(&[parent, long]);
        let lines: Vec<&str> = text.lines().collect();
        // The bytes column is 8 bytes wide; longer bytes push their row on.
        assert_eq!(
            lines.first().copied(),
            Some("3B                       TS     Direct convention  [ISO/IEC 7816-3 §8.1]")
        );
        assert_eq!(
            lines.get(1).copied(),
            Some("01                         Bit  One")
        );
        assert_eq!(
            lines.get(2).copied(),
            Some(format!("{}  Data   Ten bytes  [Ref]", ["AA"; 10].join(" ")).as_str())
        );
        assert_eq!(lines.len(), 3);
        assert_eq!(to_text(&[]), "");
    }
}
