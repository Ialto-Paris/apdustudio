// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! Response data read as BER-TLV (ISO/IEC 7816-4 §5.2, the basic encoding rules of ISO/IEC
//! 8825-1): each object's tag, length and value, nested for constructed objects, with the names
//! ISO/IEC 7816-4 gives the interindustry tags and the file control parameters (FCP, FCI, FMD).
//!
//! Data is shown as BER-TLV only when all of it reads as BER-TLV and its first object is
//! constructed or has an interindustry name: random bytes (a challenge, a UID) rarely do both.

use crate::decode::{field, Field};

const BER: &str = "ISO/IEC 7816-4 §5.2 (BER-TLV)";
const IDS: &str = "ISO/IEC 7816-4 §8 (interindustry data objects)";
const FCP: &str = "ISO/IEC 7816-4 §7.4 (file control parameters)";
const AUTH: &str = "ISO/IEC 7816-4 §11.5.6 (dynamic authentication template)";

/// Deepest nesting read; deeper content is shown as bytes.
const MAX_DEPTH: usize = 8;
/// Most objects read in one answer.
const MAX_OBJECTS: usize = 512;

struct Object<'a> {
    tag: u32,
    /// The tag and length bytes.
    header: &'a [u8],
    value: &'a [u8],
    constructed: bool,
}

/// Reads one object at the start of `b`; None when it isn't BER-TLV as ISO/IEC 7816-4 allows it
/// (tags of 1 to 3 bytes, definite lengths of up to 3 bytes).
fn object(b: &[u8]) -> Option<(Object<'_>, &[u8])> {
    let first = *b.first()?;
    let mut n = 1;
    if first & 0x1F == 0x1F {
        loop {
            let t = *b.get(n)?;
            n += 1;
            if t & 0x80 == 0 {
                break;
            }
            if n >= 3 {
                return None;
            }
        }
    }
    let tag = b
        .get(..n)?
        .iter()
        .fold(0u32, |acc, x| (acc << 8) | u32::from(*x));
    let l = *b.get(n)?;
    n += 1;
    let len = match l {
        0x00..=0x7F => usize::from(l),
        0x81..=0x83 => {
            let k = usize::from(l & 0x0F);
            let v = b
                .get(n..n + k)?
                .iter()
                .fold(0usize, |acc, x| (acc << 8) | usize::from(*x));
            n += k;
            v
        }
        _ => return None,
    };
    let value = b.get(n..n.checked_add(len)?)?;
    let rest = b.get(n + len..)?;
    Some((
        Object {
            tag,
            header: b.get(..n)?,
            value,
            constructed: first & 0x20 != 0,
        },
        rest,
    ))
}

/// Every object of `b`, or None when the bytes aren't wholly BER-TLV.
fn objects<'a>(mut b: &'a [u8], count: &mut usize) -> Option<Vec<Object<'a>>> {
    let mut out = Vec::new();
    while !b.is_empty() {
        *count += 1;
        if *count > MAX_OBJECTS {
            return None;
        }
        let (o, rest) = object(b)?;
        out.push(o);
        b = rest;
    }
    Some(out)
}

/// The name of a tag, by ISO/IEC 7816-4, inside the template `parent` (context-specific tags
/// take their meaning from it).
fn name(tag: u32, parent: Option<u32>) -> Option<&'static str> {
    let fcp = matches!(parent, Some(0x62 | 0x64 | 0x6F));
    Some(match tag {
        0x06 => "Object identifier",
        0x42 => "Issuer identification number",
        0x43 => "Card service data",
        0x44 => "Initial access data",
        0x45 => "Card issuer's data",
        0x46 => "Pre-issuing data",
        0x47 => "Card capabilities",
        0x48 => "Status information",
        0x4D => "Extended header list",
        0x4F => "Application identifier (AID)",
        0x50 => "Application label",
        0x51 => "File reference",
        0x52 => "Command to perform",
        0x53 => "Discretionary data",
        0x54 => "Offset data object",
        0x5C => "Tag list",
        0x5D => "Header list",
        0x5F50 => "Uniform resource locator",
        0x61 => "Application template",
        0x62 => "File control parameters (FCP)",
        0x64 => "File management data (FMD)",
        0x6F => "File control information (FCI)",
        0x73 => "Discretionary data template",
        0x7C => "Dynamic authentication template",
        0x80 if fcp => "Data bytes in the file (without structural information)",
        0x81 if fcp => "Data bytes in the file (with structural information)",
        0x82 if fcp => "File descriptor",
        0x83 if fcp => "File identifier",
        0x84 if fcp => "DF name",
        0x85 if fcp => "Proprietary information",
        0x86 if fcp => "Security attribute (proprietary format)",
        0x87 if fcp => "EF holding an extension of the FCI",
        0x88 if fcp => "Short EF identifier",
        0x8A if fcp => "Life cycle status",
        0x8B if fcp => "Security attribute (expanded format, referenced)",
        0x8C if fcp => "Security attribute (compact format)",
        0x8D if fcp => "EF holding security environment templates",
        0x8E if fcp => "Channel security attribute",
        0xA0 if fcp => "Security attribute template for data objects",
        0xA1 if fcp => "Security attribute template (proprietary format)",
        0xA5 if fcp => "Proprietary information",
        0xAB if fcp => "Security attribute template (expanded format)",
        0xAC if fcp => "Cryptographic mechanism identifier template",
        0x80 if parent == Some(0x7C) => "Witness",
        0x81 if parent == Some(0x7C) => "Challenge",
        0x82 if parent == Some(0x7C) => "Response",
        0x83 if parent == Some(0x7C) => "Committed challenge",
        0x84 if parent == Some(0x7C) => "Authentication code",
        0x85 if parent == Some(0x7C) => "Exponential",
        _ => return None,
    })
}

fn reference(tag: u32, parent: Option<u32>) -> &'static str {
    match (tag, parent) {
        (0x62 | 0x64 | 0x6F, _) | (_, Some(0x62 | 0x64 | 0x6F)) => FCP,
        (0x7C, _) | (_, Some(0x7C)) => AUTH,
        _ if name(tag, parent).is_some() => IDS,
        _ => BER,
    }
}

/// A tag's class, by its first byte's two high bits.
fn class(tag: u32) -> &'static str {
    let first = if tag > 0xFFFF {
        tag >> 16
    } else if tag > 0xFF {
        tag >> 8
    } else {
        tag
    };
    match first >> 6 {
        0 => "universal class",
        1 => "application class",
        2 => "context-specific class",
        _ => "private class",
    }
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Printable ASCII in quotes, for labels and URLs.
fn text(v: &[u8]) -> Option<String> {
    (!v.is_empty() && v.iter().all(|c| (0x20..0x7F).contains(c)))
        .then(|| format!("\"{}\"", String::from_utf8_lossy(v)))
}

fn number(v: &[u8]) -> Option<u64> {
    (!v.is_empty() && v.len() <= 8).then(|| v.iter().fold(0u64, |a, x| (a << 8) | u64::from(*x)))
}

/// What a primitive value says, beyond its bytes, for the tags whose coding ISO/IEC 7816-4 sets.
fn value_meaning(tag: u32, parent: Option<u32>, v: &[u8]) -> Option<String> {
    let fcp = matches!(parent, Some(0x62 | 0x64 | 0x6F));
    match tag {
        0x50 | 0x5F50 => text(v),
        0x4F => aid(v),
        0x84 if fcp => aid(v)
            .map(|a| format!("Coded as an AID: {a}"))
            .or_else(|| text(v)),
        0x80 | 0x81 if fcp => number(v).map(|n| format!("{n} bytes")),
        0x82 if fcp => file_descriptor(v),
        0x83 if fcp => match v {
            [0x3F, 0x00] => Some("3F00, the MF".into()),
            [a, b] => Some(format!("{a:02X}{b:02X}")),
            _ => None,
        },
        0x88 if fcp => match v {
            [] => Some("No short EF identifier".into()),
            [b] if b & 0x07 == 0 => Some(format!("Short EF identifier {}", b >> 3)),
            _ => None,
        },
        0x8A if fcp => match v {
            [b] => Some(life_cycle(*b).into()),
            _ => None,
        },
        _ => None,
    }
}

/// An application identifier: a registered one (first digit A, ISO/IEC 7816-5) splits into its
/// 5-byte RID and its PIX. The PIX's coding is its RID owner's, so trailing zeros are only
/// counted, never called padding.
fn aid(v: &[u8]) -> Option<String> {
    match v {
        [first, ..] if first >> 4 == 0xA && v.len() >= 5 => {
            let (rid, pix) = v.split_at(5);
            Some(format!("RID {} (registered), {}", hex(rid), pix_text(pix)))
        }
        _ => None,
    }
}

/// A PIX in hex, a run of two or more trailing zeros counted ("01 02 + 6 × 00").
fn pix_text(pix: &[u8]) -> String {
    let zeros = pix.iter().rev().take_while(|b| **b == 0).count();
    let head = pix.get(..pix.len() - zeros).unwrap_or_default();
    match (head, zeros) {
        ([], 0) => "no PIX".to_string(),
        ([], 2..) => format!("PIX {zeros} × 00"),
        (_, 2..) => format!("PIX {} + {zeros} × 00", hex(head)),
        _ => format!("PIX {}", hex(pix)),
    }
}

/// "1 byte", "8 bytes".
fn size(n: usize) -> String {
    if n == 1 {
        "1 byte".to_string()
    } else {
        format!("{n} bytes")
    }
}

fn file_descriptor(v: &[u8]) -> Option<String> {
    let fdb = *v.first()?;
    if fdb & 0x80 != 0 {
        return Some("Proprietary file descriptor".into());
    }
    let category = match (fdb >> 3) & 0x07 {
        0 => "working EF",
        1 => "internal EF",
        7 => "DF",
        _ => "proprietary EF",
    };
    let mut s = String::from(category);
    if fdb & 0x40 != 0 {
        s.push_str(", shareable");
    }
    if (fdb >> 3) & 0x07 != 7 {
        let structure = match fdb & 0x07 {
            0 => "no structure given",
            1 => "transparent",
            2 => "linear fixed",
            3 => "linear fixed, TLV",
            4 => "linear variable",
            5 => "linear variable, TLV",
            6 => "cyclic",
            _ => "cyclic, TLV",
        };
        s.push_str(&format!(", {structure}"));
    }
    match v.get(1..).unwrap_or_default() {
        [] => {}
        [dc] => s.push_str(&format!("; data coding 0x{dc:02X}")),
        [dc, size] => s.push_str(&format!(
            "; data coding 0x{dc:02X}, records up to {size} bytes"
        )),
        [dc, s1, s2] => s.push_str(&format!(
            "; data coding 0x{dc:02X}, records up to {} bytes",
            u16::from_be_bytes([*s1, *s2])
        )),
        [dc, s1, s2, n @ ..] => s.push_str(&format!(
            "; data coding 0x{dc:02X}, records up to {} bytes, {} records",
            u16::from_be_bytes([*s1, *s2]),
            number(n).unwrap_or(0)
        )),
    }
    Some(s)
}

fn life_cycle(b: u8) -> &'static str {
    match b {
        0x00 => "No information given",
        0x01 => "Creation state",
        0x03 => "Initialisation state",
        0x05 | 0x07 => "Operational state, activated",
        0x04 | 0x06 => "Operational state, deactivated",
        0x0C..=0x0F => "Termination state",
        0x10.. => "Proprietary",
        _ => "Reserved for future use",
    }
}

fn fields(
    objs: Vec<Object<'_>>,
    parent: Option<u32>,
    depth: usize,
    count: &mut usize,
) -> Vec<Field> {
    let mut out = Vec::new();
    for o in objs {
        // As in the rest of the detail: the bytes column holds the object's bytes (only a
        // constructed one's header, its objects being the rows below), the name column its
        // name, and the meaning column only what the value says.
        let label = match name(o.tag, parent) {
            Some(n) => n.to_string(),
            None => format!("Tag {:X}, {}", o.tag, class(o.tag)),
        };
        let mut f = field(o.header, label, "", reference(o.tag, parent));
        let inner = (o.constructed && depth < MAX_DEPTH)
            .then(|| objects(o.value, count))
            .flatten();
        match inner {
            Some(children) => {
                f.meaning = format!("Constructed, {}", size(o.value.len()));
                f.children = fields(children, Some(o.tag), depth + 1, count);
            }
            None => {
                if !o.value.is_empty() {
                    f.bytes = format!("{} {}", f.bytes, hex(o.value));
                }
                f.meaning = match value_meaning(o.tag, parent, o.value) {
                    Some(m) => m,
                    None if o.value.is_empty() => "Empty".to_string(),
                    None if o.constructed => format!(
                        "Constructed, but its content isn't BER-TLV: {}",
                        size(o.value.len())
                    ),
                    None => size(o.value.len()),
                };
            }
        }
        out.push(f);
    }
    out
}

/// The data as BER-TLV fields, or None when it doesn't read as BER-TLV.
pub fn decode(data: &[u8]) -> Option<Vec<Field>> {
    let mut count = 0;
    let objs = objects(data, &mut count)?;
    let first = objs.first()?;
    if !first.constructed && name(first.tag, None).is_none() {
        return None;
    }
    Some(fields(objs, None, 0, &mut count))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(fs: &'a [Field], prefix: &str) -> &'a Field {
        fs.iter()
            .find(|f| f.bytes.starts_with(prefix))
            .unwrap_or_else(|| panic!("no {prefix} in {fs:#?}"))
    }

    #[test]
    fn an_fci_after_select_reads_as_nested_objects() {
        // The demo card's answer to SELECT by AID.
        let fci = [
            0x6F, 0x0F, 0x84, 0x07, 0xA0, 0x00, 0x00, 0x00, 0x04, 0x10, 0x10, 0xA5, 0x04, 0x50,
            0x02, 0x44, 0x4D,
        ];
        let f = decode(&fci).expect("BER-TLV");
        let t = find(&f, "6F ");
        assert_eq!(t.bytes, "6F 0F");
        assert_eq!(t.meaning, "Constructed, 15 bytes");
        assert_eq!(t.name, "File control information (FCI)");
        let name = find(&t.children, "84 ");
        assert_eq!(name.bytes, "84 07 A0 00 00 00 04 10 10");
        assert_eq!(name.name, "DF name");
        assert_eq!(
            name.meaning,
            "Coded as an AID: RID A0 00 00 00 04 (registered), PIX 10 10"
        );
        let a5 = find(&t.children, "A5 ");
        assert_eq!(a5.bytes, "A5 04");
        let label = find(&a5.children, "50 ");
        assert_eq!(
            (label.bytes.as_str(), label.meaning.as_str()),
            ("50 02 44 4D", "\"DM\"")
        );
    }

    #[test]
    fn an_fcp_says_what_the_file_is() {
        // The last object (81) gives its length in the long form: 81 01.
        let fcp = [
            0x62, 0x19, 0x82, 0x05, 0x02, 0x41, 0x00, 0x1A, 0x05, 0x83, 0x02, 0x2F, 0x01, 0x88,
            0x01, 0x08, 0x8A, 0x01, 0x05, 0x80, 0x02, 0x00, 0x82, 0x81, 0x81, 0x01, 0x07,
        ];
        let f = decode(&fcp).expect("BER-TLV");
        let c = &find(&f, "62 ").children;
        assert_eq!(
            find(c, "82 ").meaning,
            "working EF, linear fixed; data coding 0x41, records up to 26 bytes, 5 records"
        );
        assert_eq!(find(c, "83 ").meaning, "2F01");
        assert_eq!(find(c, "88 ").meaning, "Short EF identifier 1");
        assert_eq!(find(c, "8A ").meaning, "Operational state, activated");
        assert_eq!(find(c, "80 ").meaning, "130 bytes");
        let long = find(c, "81 ");
        assert_eq!(
            (long.bytes.as_str(), long.meaning.as_str()),
            ("81 81 01 07", "7 bytes")
        );
    }

    #[test]
    fn data_that_is_not_ber_tlv_is_left_alone() {
        // A challenge, a UID, a status word's data: not shown as TLV.
        assert!(decode(&[0x3A, 0x91, 0x0C, 0x5E, 0x77, 0x02, 0xD4, 0x18]).is_none());
        assert!(decode(&[0x04, 0xA2, 0x2B, 0x6A, 0x3C, 0x5D, 0x80]).is_none());
        // Reads as TLV, but the first object is a primitive universal one: no.
        assert!(decode(&[0x04, 0x01, 0x00]).is_none());
        assert!(decode(&[]).is_none());
        // A length past the end, an indefinite length, a tag of 4 bytes.
        assert!(decode(&[0x6F, 0x05, 0x84, 0x01]).is_none());
        assert!(decode(&[0x6F, 0x80, 0x00, 0x00]).is_none());
        assert!(decode(&[0x7F, 0x81, 0x82, 0x83, 0x01, 0x00]).is_none());
    }

    #[test]
    fn deep_or_long_data_is_bounded() {
        // 200 nested templates: read to MAX_DEPTH, the rest as bytes.
        let mut v: Vec<u8> = Vec::new();
        for _ in 0..200 {
            let len = v.len();
            let mut outer = vec![0x73];
            if len < 0x80 {
                outer.push(len as u8);
            } else if len < 0x100 {
                outer.extend([0x81, len as u8]);
            } else {
                outer.extend([0x82, (len >> 8) as u8, len as u8]);
            }
            outer.extend(v);
            v = outer;
        }
        let f = decode(&v).expect("BER-TLV");
        let mut depth = 0;
        let mut cur = &f;
        while let Some(x) = cur.first() {
            depth += 1;
            cur = &x.children;
        }
        assert!(depth <= MAX_DEPTH + 1, "{depth}");
        // 2,000 empty objects: more than MAX_OBJECTS, so not read.
        let many: Vec<u8> = std::iter::repeat_n([0x53u8, 0x00], 2000)
            .flatten()
            .collect();
        assert!(decode(&many).is_none());
    }

    #[test]
    fn unnamed_tags_say_their_class() {
        let f = decode(&[0x70, 0x04, 0x9F, 0x36, 0x01, 0x05]).expect("BER-TLV");
        let t = find(&f, "70 ");
        assert_eq!(t.name, "Tag 70, application class");
        let atc = find(&t.children, "9F 36");
        assert_eq!(
            (atc.bytes.as_str(), atc.name.as_str(), atc.meaning.as_str()),
            ("9F 36 01 05", "Tag 9F36, context-specific class", "1 byte")
        );
    }

    #[test]
    fn a_df_name_coded_as_an_aid_counts_its_trailing_zeros() {
        // A DF name of 16 bytes: a registered RID, then a PIX ending in six zeros.
        let fci = [
            0x6F, 0x12, 0x84, 0x10, 0xA0, 0x00, 0x00, 0x04, 0x04, 0x01, 0x25, 0x09, 0x01, 0x01,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let f = decode(&fci).expect("BER-TLV");
        assert_eq!(
            find(&f[0].children, "84 ").meaning,
            "Coded as an AID: RID A0 00 00 04 04 (registered), PIX 01 25 09 01 01 + 6 × 00"
        );
        // A DF name that isn't an AID reads as text.
        let named = decode(b"\x6F\x0A\x84\x081TIC.ICA").expect("BER-TLV");
        assert_eq!(find(&named[0].children, "84 ").meaning, "\"1TIC.ICA\"");
        // An AID object; a single trailing zero is shown as it is.
        let app = decode(&[0x4F, 0x07, 0xA0, 0x00, 0x00, 0x00, 0x04, 0x10, 0x00]).expect("BER-TLV");
        assert_eq!(app[0].meaning, "RID A0 00 00 00 04 (registered), PIX 10 00");
        assert_eq!(
            aid(&[0xA0, 0, 0, 0, 3]).as_deref(),
            Some("RID A0 00 00 00 03 (registered), no PIX")
        );
        assert_eq!(
            aid(&[0xA0, 0, 0, 0, 3, 0, 0]).as_deref(),
            Some("RID A0 00 00 00 03 (registered), PIX 2 × 00")
        );
        // Not a registered AID, or too short for a RID: not split.
        assert_eq!(aid(&[0xD2, 0x50, 0, 0, 1]), None);
        assert_eq!(aid(&[0xA0, 0, 0]), None);
    }
}
