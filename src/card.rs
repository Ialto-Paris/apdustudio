// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! Exchanges with the card (SCardTransmit) and the reader (standard control codes,
//! SCardGetAttrib), and the texts shown for the answers. Only PC/SC: no code of one maker.

use std::fmt;
use std::time::{Duration, Instant};

use crate::reader::{Attribute, CardLink, Error, State, Status};

/// Why an exchange with the card failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExchangeError {
    /// No card connection is open.
    NoCard,
    /// The reader or the card refused: the smart card service's error.
    Reader(Error),
}

/// Why a command can't go out: no connection is open.
pub const NO_CONNECTION: &str = "No card connection: connect first.";

impl fmt::Display for ExchangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExchangeError::NoCard => f.write_str(NO_CONNECTION),
            ExchangeError::Reader(e) => f.write_str(&error_text(*e)),
        }
    }
}

/// Sends an APDU with SCardTransmit. Returns the answer and how long it took.
pub fn transmit(
    link: &mut dyn CardLink,
    apdu: &[u8],
    response_buffer: usize,
) -> Result<(Vec<u8>, Duration), ExchangeError> {
    let t = Instant::now();
    match link.transmit(apdu, response_buffer + 2) {
        Ok(r) => Ok((r, t.elapsed())),
        Err(e) => Err(ExchangeError::Reader(e)),
    }
}

/// One command sent to the card and its answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leg {
    pub sent: Vec<u8>,
    pub reply: Vec<u8>,
    pub took: Duration,
}

/// Most commands sent for one when following 61xx and 6Cxx: a card that keeps asking stops here.
pub const MAX_LEGS: usize = 32;

/// A command and the ones sent after it for 61xx and 6Cxx.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Followed {
    /// Every command sent, in order (the first is the one asked for).
    pub legs: Vec<Leg>,
    /// The whole answer (every leg's data, then the last status word) and the total time, or
    /// why the last command failed.
    pub answer: Result<(Vec<u8>, Duration), ExchangeError>,
}

/// The class of a GET RESPONSE that follows a command of class `cla` (ISO/IEC 7816-4): the same
/// logical channel, without secure messaging or chaining. A proprietary class gets 00, as GET
/// RESPONSE is an interindustry command.
pub fn get_response_class(cla: u8) -> u8 {
    match cla {
        0x00..=0x1F => cla & 0x03,
        0x40..=0x7F => 0x40 | (cla & 0x0F),
        _ => 0x00,
    }
}

/// The command `apdu` again with Le = `le` (for 6Cxx): only a short command that expects data
/// or has none (cases 1, 2 and 4); None otherwise.
pub fn with_le(apdu: &[u8], le: u8) -> Option<Vec<u8>> {
    let c = crate::iso::parse_command(apdu)?;
    if c.extended || (c.lc.is_some() && c.le.is_none()) {
        return None;
    }
    let mut out = apdu.to_vec();
    if c.le.is_some() {
        *out.last_mut()? = le;
    } else {
        out.push(le);
    }
    Some(out)
}

/// Sends `apdu`; with `follow`, then sends GET RESPONSE while the card answers 61xx, and the
/// command again with the right Le when it answers 6Cxx (ISO/IEC 7816-3 and 7816-4), up to
/// `MAX_LEGS` commands and `response_buffer` bytes of data.
pub fn transmit_following(
    link: &mut dyn CardLink,
    apdu: &[u8],
    response_buffer: usize,
    follow: bool,
) -> Followed {
    let mut legs = Vec::new();
    let mut data = Vec::new();
    let mut total = Duration::ZERO;
    let mut sent = apdu.to_vec();
    loop {
        let (reply, took) = match transmit(link, &sent, response_buffer) {
            Ok(x) => x,
            Err(e) => {
                return Followed {
                    legs,
                    answer: Err(e),
                }
            }
        };
        total = total.saturating_add(took);
        let (body, sw) = match reply.split_last_chunk::<2>() {
            Some((b, sw)) => (b.to_vec(), Some(*sw)),
            None => (reply.clone(), None),
        };
        legs.push(Leg {
            sent: sent.clone(),
            reply,
            took,
        });
        data.extend_from_slice(&body);
        let room = legs.len() < MAX_LEGS && data.len() <= response_buffer;
        let next = match sw {
            Some([0x61, n]) if follow && room => Some(vec![
                get_response_class(apdu.first().copied().unwrap_or(0)),
                0xC0,
                0x00,
                0x00,
                n,
            ]),
            Some([0x6C, n]) if follow && room && body.is_empty() => with_le(&sent, n),
            _ => None,
        };
        match next {
            Some(cmd) => sent = cmd,
            None => {
                if let Some(sw) = sw {
                    data.extend_from_slice(&sw);
                }
                return Followed {
                    legs,
                    answer: Ok((data, total)),
                };
            }
        }
    }
}

/// SCARD_CTL_CODE(n), the SCardControl code of function `n`, as one system builds it:
/// `0x00310000 | n << 2` on Windows (winscard), `0x42000000 + n` with pcsc-lite (Linux, macOS).
pub const fn ctl_code_for(windows: bool, n: u32) -> u32 {
    if windows {
        0x0031_0000 | (n << 2)
    } else {
        0x4200_0000 + n
    }
}

/// SCARD_CTL_CODE(n) on the system ApduStudio was built for.
pub const fn ctl_code(n: u32) -> u32 {
    ctl_code_for(cfg!(windows), n)
}

/// CM_IOCTL_GET_FEATURE_REQUEST, SCARD_CTL_CODE(3400) (PC/SC part 10): the reader lists the
/// features it supports, each with the control code that runs it.
pub const IOCTL_GET_FEATURE_REQUEST: u32 = ctl_code(3400);
/// PC/SC part 10 feature tag of the CCID escape command (FEATURE_CCID_ESC_COMMAND).
pub const FEATURE_CCID_ESC_COMMAND: u8 = 0x13;
/// The CCID escape code when the reader doesn't list FEATURE_CCID_ESC_COMMAND: on Windows,
/// IOCTL_CCID_ESCAPE, SCARD_CTL_CODE(3500) (0x3136B0), from Windows' CCID driver; with
/// pcsc-lite, IOCTL_SMARTCARD_VENDOR_IFD_EXCHANGE, SCARD_CTL_CODE(1), from its CCID driver.
pub const IOCTL_CCID_ESCAPE: u32 = if cfg!(windows) {
    ctl_code(3500)
} else {
    ctl_code(1)
};

/// Where the CCID escape code came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeCode {
    /// The reader listed FEATURE_CCID_ESC_COMMAND with this code.
    Reported(u32),
    /// The reader listed no such feature: the system's default, IOCTL_CCID_ESCAPE.
    Default(u32),
}

impl EscapeCode {
    pub fn code(self) -> u32 {
        match self {
            EscapeCode::Reported(c) | EscapeCode::Default(c) => c,
        }
    }

    /// One line for the exchanges: which code and why.
    pub fn describe(self) -> String {
        match self {
            EscapeCode::Reported(c) => format!(
                "CCID escape: control code 0x{c:08X}, the reader's FEATURE_CCID_ESC_COMMAND (PC/SC part 10)"
            ),
            EscapeCode::Default(c) => format!(
                "CCID escape: control code 0x{c:08X}, {} (the reader lists no FEATURE_CCID_ESC_COMMAND)",
                if cfg!(windows) {
                    "IOCTL_CCID_ESCAPE, Windows' code"
                } else {
                    "IOCTL_SMARTCARD_VENDOR_IFD_EXCHANGE, the code of pcsc-lite's CCID driver"
                }
            ),
        }
    }
}

/// The features of a GET_FEATURE_REQUEST reply: tag and control code, from its TLV list
/// (tag, length 4, code big-endian). Stops at the first malformed entry.
pub fn features(reply: &[u8]) -> Vec<(u8, u32)> {
    let mut out = Vec::new();
    let mut rest = reply;
    while let [tag, 4, a, b, c, d, tail @ ..] = rest {
        out.push((*tag, u32::from_be_bytes([*a, *b, *c, *d])));
        rest = tail;
    }
    out
}

/// The code to send a CCID escape command with: the one the reader lists for
/// FEATURE_CCID_ESC_COMMAND, or else the system's default.
pub fn escape_code(link: &mut dyn CardLink) -> EscapeCode {
    link.control(IOCTL_GET_FEATURE_REQUEST, &[], 256)
        .ok()
        .and_then(|r| {
            features(&r)
                .into_iter()
                .find(|(tag, _)| *tag == FEATURE_CCID_ESC_COMMAND)
        })
        .map(|(_, c)| EscapeCode::Reported(c))
        .unwrap_or(EscapeCode::Default(IOCTL_CCID_ESCAPE))
}

/// What to check when the reader refuses an escape command.
pub const ESCAPE_HINT: &str = if cfg!(windows) {
    "Windows' own CCID driver accepts escape commands only for readers whose EscapeCommandEnable registry value (a DWORD) is non-zero; a reader maker's driver may have its own setting."
} else {
    "pcsc-lite's CCID driver accepts escape commands only when ifdDriverOptions in its Info.plist has DRIVER_OPTION_CCID_EXCHANGE_AUTHORIZED (0x0001)."
};
/// Most readers listed. The reader states are read in batches (`reader::STATUS_BATCH`), so
/// this only bounds the list.
pub const MAX_READERS: usize = 64;

/// The PC/SC name of an error and what it means, in a few words.
fn error_name(e: Error) -> (&'static str, &'static str) {
    use Error::*;
    match e {
        Cancelled => ("SCARD_E_CANCELLED", "Cancelled before the end"),
        CantDispose => (
            "SCARD_E_CANT_DISPOSE",
            "The reader can't do that with the card (power down, reset, eject)",
        ),
        CardUnsupported => ("SCARD_E_CARD_UNSUPPORTED", "This card isn't supported"),
        DuplicateReader => ("SCARD_E_DUPLICATE_READER", "Two readers have the same name"),
        InsufficientBuffer => (
            "SCARD_E_INSUFFICIENT_BUFFER",
            "The answer is longer than the buffer kept for it (see --response-buffer)",
        ),
        InvalidAtr => ("SCARD_E_INVALID_ATR", "The ATR given isn't valid"),
        InvalidHandle => (
            "SCARD_E_INVALID_HANDLE",
            "The connection is no longer valid: connect again",
        ),
        InvalidParameter => (
            "SCARD_E_INVALID_PARAMETER",
            "The service refused a parameter of the call",
        ),
        InvalidTarget => (
            "SCARD_E_INVALID_TARGET",
            "The service's start-up settings are missing or wrong",
        ),
        InvalidValue => (
            "SCARD_E_INVALID_VALUE",
            "The service refused a value given to the call",
        ),
        NotReady => ("SCARD_E_NOT_READY", "The reader or the card isn't ready"),
        NotTransacted => ("SCARD_E_NOT_TRANSACTED", "No transaction was open"),
        NoMemory => ("SCARD_E_NO_MEMORY", "The service ran out of memory"),
        NoService => ("SCARD_E_NO_SERVICE", "The smart card service isn't running"),
        NoSmartcard => ("SCARD_E_NO_SMARTCARD", "The reader has no card"),
        PciTooSmall => (
            "SCARD_E_PCI_TOO_SMALL",
            "The protocol information buffer is too small",
        ),
        ProtoMismatch => (
            "SCARD_E_PROTO_MISMATCH",
            "The card doesn't use the protocol asked for",
        ),
        ReaderUnavailable => (
            "SCARD_E_READER_UNAVAILABLE",
            "The reader can't be used right now",
        ),
        ReaderUnsupported => (
            "SCARD_E_READER_UNSUPPORTED",
            "The service doesn't support this reader's driver",
        ),
        ServiceStopped => ("SCARD_E_SERVICE_STOPPED", "The smart card service stopped"),
        SharingViolation => (
            "SCARD_E_SHARING_VIOLATION",
            "Another program holds the card: close it, or connect in shared mode",
        ),
        SystemCancelled => (
            "SCARD_E_SYSTEM_CANCELLED",
            "The system cancelled the action (logging off or shutting down)",
        ),
        Timeout => ("SCARD_E_TIMEOUT", "The time allowed ran out"),
        UnknownCard => ("SCARD_E_UNKNOWN_CARD", "The service doesn't know this card"),
        UnknownReader => ("SCARD_E_UNKNOWN_READER", "No reader has this name"),
        CommError => (
            "SCARD_F_COMM_ERROR",
            "Talking to the service or the reader failed",
        ),
        InternalError => (
            "SCARD_F_INTERNAL_ERROR",
            "The service failed one of its own checks",
        ),
        UnknownError => (
            "SCARD_F_UNKNOWN_ERROR",
            "The service failed and doesn't say why",
        ),
        WaitedTooLong => (
            "SCARD_F_WAITED_TOO_LONG",
            "One of the service's own timers ran out",
        ),
        Shutdown => ("SCARD_P_SHUTDOWN", "The service is shutting down"),
        // pcsc has this one on Windows only.
        #[cfg(windows)]
        Unexpected => (
            "SCARD_E_UNEXPECTED",
            "The service met something it didn't expect",
        ),
        CommDataLost => (
            "SCARD_E_COMM_DATA_LOST",
            "Data was lost on the way to or from the card",
        ),
        ServerTooBusy => ("SCARD_E_SERVER_TOO_BUSY", "The service is too busy"),
        RemovedCard => ("SCARD_W_REMOVED_CARD", "The card was taken out"),
        ResetCard => (
            "SCARD_W_RESET_CARD",
            "The card was reset since the connection opened",
        ),
        UnpoweredCard => ("SCARD_W_UNPOWERED_CARD", "The card has no power"),
        UnresponsiveCard => (
            "SCARD_W_UNRESPONSIVE_CARD",
            "The card doesn't answer its reset",
        ),
        UnsupportedCard => (
            "SCARD_W_UNSUPPORTED_CARD",
            "The reader can't talk to this card (its ATR)",
        ),
        NoReadersAvailable => ("SCARD_E_NO_READERS_AVAILABLE", "No reader is connected"),
        UnsupportedFeature => (
            "SCARD_E_UNSUPPORTED_FEATURE",
            "The reader or its driver doesn't support this",
        ),
        _ => ("", "The smart card service returned an error"),
    }
}

/// An error of the smart card service as the person reads it: "The reader has no card
/// (SCARD_E_NO_SMARTCARD, 0x8010000C)".
pub fn error_text(e: Error) -> String {
    let code = e as u32;
    match error_name(e) {
        ("", text) => format!("{text} (0x{code:08X})"),
        (name, text) => format!("{text} ({name}, 0x{code:08X})"),
    }
}

/// Card State text (SCardStatus).
pub fn status_text(s: Status) -> &'static str {
    if s.contains(Status::SPECIFIC) {
        "A card is in, powered, with its protocol chosen."
    } else if s.contains(Status::NEGOTIABLE) {
        "A card is in and reset; its protocol isn't chosen yet."
    } else if s.contains(Status::POWERED) {
        "A card is in and powered; its mode isn't known."
    } else if s.contains(Status::SWALLOWED) {
        "A card is in place, without power."
    } else if s.contains(Status::PRESENT) {
        "A card is in."
    } else {
        "The reader has no card."
    }
}

/// The SCARD_* names of a card status (SCardStatus), e.g. "SCARD_PRESENT | SCARD_SPECIFIC".
pub fn status_names(s: Status) -> String {
    let names: Vec<&str> = [
        (Status::ABSENT, "SCARD_ABSENT"),
        (Status::PRESENT, "SCARD_PRESENT"),
        (Status::SWALLOWED, "SCARD_SWALLOWED"),
        (Status::POWERED, "SCARD_POWERED"),
        (Status::NEGOTIABLE, "SCARD_NEGOTIABLE"),
        (Status::SPECIFIC, "SCARD_SPECIFIC"),
    ]
    .into_iter()
    .filter(|(flag, _)| s.contains(*flag))
    .map(|(_, n)| n)
    .collect();
    if names.is_empty() {
        "SCARD_UNKNOWN".into()
    } else {
        names.join(" | ")
    }
}

/// GET DATA, UID (PC/SC part 3 supplement): the reader answers the contactless card's UID.
pub const GET_UID: [u8; 5] = [0xFF, 0xCA, 0x00, 0x00, 0x00];

/// What the answer to `GET_UID` says.
pub fn uid_text(resp: &[u8]) -> String {
    match resp {
        [uid @ .., 0x90, 0x00] if !uid.is_empty() => {
            let hex: Vec<String> = uid.iter().map(|b| format!("{b:02X}")).collect();
            format!(
                "UID ({} bytes, PC/SC part 3 GET DATA): {}",
                uid.len(),
                hex.join(" ")
            )
        }
        [.., 0x6A, 0x81] => "UID: the reader supports GET DATA, not for this card (6A81).".into(),
        [.., 0x6E, 0x00] | [.., 0x68, 0x00] | [.., 0x6D, 0x00] => {
            "UID: the command reached the card, which refused class FF: not a contactless reader implementing PC/SC part 3.".into()
        }
        _ => format!(
            "UID: no UID in the answer ({}).",
            resp.iter()
                .map(|b| format!("{b:02X}"))
                .collect::<String>()
        ),
    }
}

/// Readers' state text for one reader's state (SCardGetStatusChange): one line per flag.
pub fn reader_state_text(s: State) -> String {
    let mut out = String::new();
    let pairs: [(State, &str, &str); 11] = [
        (
            State::UNKNOWN,
            "SCARD_STATE_UNKNOWN",
            "the service doesn't know this reader",
        ),
        (
            State::UNAVAILABLE,
            "SCARD_STATE_UNAVAILABLE",
            "its state can't be read",
        ),
        (
            State::IGNORE,
            "SCARD_STATE_IGNORE",
            "the service says to leave it aside",
        ),
        (State::CHANGED, "SCARD_STATE_CHANGED", "its state changed"),
        (State::EMPTY, "SCARD_STATE_EMPTY", "no card"),
        (State::PRESENT, "SCARD_STATE_PRESENT", "a card is in"),
        (
            State::ATRMATCH,
            "SCARD_STATE_ATRMATCH",
            "the card's ATR is one asked for",
        ),
        (
            State::EXCLUSIVE,
            "SCARD_STATE_EXCLUSIVE",
            "another program holds the card alone",
        ),
        (
            State::INUSE,
            "SCARD_STATE_INUSE",
            "other programs use the card; sharing it is possible",
        ),
        (State::MUTE, "SCARD_STATE_MUTE", "the card doesn't answer"),
        (
            State::UNPOWERED,
            "SCARD_STATE_UNPOWERED",
            "the card has no power",
        ),
    ];
    for (flag, name, text) in pairs {
        if s.contains(flag) {
            out.push_str(&format!("{name}: {text}\n"));
        }
    }
    out
}

// ----------------------------------------------------------------- reader info

/// How an attribute's bytes read (PC/SC part 3, SCardGetAttrib).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrKind {
    /// Text, possibly ending with NUL bytes (UTF-16 from some Windows drivers).
    Text,
    /// A DWORD 0xMMmmbbbb: major, minor and build (SCARD_ATTR_VENDOR_IFD_VERSION).
    Version,
    /// A DWORD whose high word is the reader's connection (SCARD_ATTR_CHANNEL_ID).
    Channel,
    /// A DWORD of protocol bits: 1 is T=0, 2 is T=1.
    Protocols,
    /// A DWORD in kHz.
    KHz,
    /// A DWORD in bit/s.
    Bps,
    /// A DWORD.
    Number,
}

/// The standard attributes Reader info asks for, in the order shown. A driver gives the ones it
/// has; the others are left out.
pub const READER_ATTRIBUTES: &[(Attribute, &str, AttrKind)] = &[
    (Attribute::VendorName, "Vendor", AttrKind::Text),
    (Attribute::VendorIfdType, "Model", AttrKind::Text),
    (Attribute::VendorIfdVersion, "Version", AttrKind::Version),
    (
        Attribute::VendorIfdSerialNo,
        "Serial number",
        AttrKind::Text,
    ),
    (Attribute::DeviceFriendlyName, "Name", AttrKind::Text),
    (Attribute::ChannelId, "Connection", AttrKind::Channel),
    (
        Attribute::AsyncProtocolTypes,
        "Protocols",
        AttrKind::Protocols,
    ),
    (Attribute::DefaultClk, "Default clock", AttrKind::KHz),
    (Attribute::MaxClk, "Maximum clock", AttrKind::KHz),
    (
        Attribute::DefaultDataRate,
        "Default data rate",
        AttrKind::Bps,
    ),
    (Attribute::MaxDataRate, "Maximum data rate", AttrKind::Bps),
    (Attribute::MaxIfsd, "Maximum IFSD", AttrKind::Number),
];

/// The features PC/SC part 10 defines, by tag, for GET_FEATURE_REQUEST's list.
pub const PART10_FEATURES: &[(u8, &str)] = &[
    (0x01, "VERIFY_PIN_START"),
    (0x02, "VERIFY_PIN_FINISH"),
    (0x03, "MODIFY_PIN_START"),
    (0x04, "MODIFY_PIN_FINISH"),
    (0x05, "GET_KEY_PRESSED"),
    (0x06, "VERIFY_PIN_DIRECT"),
    (0x07, "MODIFY_PIN_DIRECT"),
    (0x08, "MCT_READER_DIRECT"),
    (0x09, "MCT_UNIVERSAL"),
    (0x0A, "IFD_PIN_PROPERTIES"),
    (0x0B, "ABORT"),
    (0x0C, "SET_SPE_MESSAGE"),
    (0x0D, "VERIFY_PIN_DIRECT_APP_ID"),
    (0x0E, "MODIFY_PIN_DIRECT_APP_ID"),
    (0x0F, "WRITE_DISPLAY"),
    (0x10, "GET_KEY"),
    (0x11, "IFD_DISPLAY_PROPERTIES"),
    (0x12, "GET_TLV_PROPERTIES"),
    (0x13, "CCID_ESC_COMMAND"),
    (0x20, "EXECUTE_PACE"),
];

/// The first four bytes as a little-endian DWORD (pcsc-lite gives 8 on 64-bit Linux).
fn dword(b: &[u8]) -> Option<u32> {
    match b {
        [a, b, c, d, ..] => Some(u32::from_le_bytes([*a, *b, *c, *d])),
        _ => None,
    }
}

/// An attribute's value as text, or None when the driver gave nothing readable.
pub fn attribute_text(kind: AttrKind, b: &[u8]) -> Option<String> {
    let text = match kind {
        AttrKind::Text => {
            let utf16 = b.len() >= 4 && b.get(1) == Some(&0) && b.get(3) == Some(&0);
            let s = if utf16 {
                let units: Vec<u16> = b
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes(*c))
                    .collect();
                String::from_utf16_lossy(&units)
            } else {
                String::from_utf8_lossy(b).into_owned()
            };
            s.trim_matches(|c: char| c == '\0' || c.is_whitespace())
                .to_string()
        }
        AttrKind::Version => {
            // 0 is what drivers that don't know the version give.
            let v = dword(b).filter(|v| *v != 0)?;
            format!(
                "{}.{}.{} (0x{v:08X})",
                v >> 24,
                (v >> 16) & 0xFF,
                v & 0xFFFF
            )
        }
        AttrKind::Channel => {
            let v = dword(b)?;
            let kind = match v >> 16 {
                0x01 => "serial port",
                0x02 => "parallel port",
                0x04 => "PS/2 keyboard port",
                0x08 => "SCSI",
                0x10 => "IDE",
                0x20 => "USB",
                _ => "other",
            };
            format!("{kind} (0x{v:08X})")
        }
        AttrKind::Protocols => {
            let v = dword(b)?;
            let mut p = Vec::new();
            if v & 1 != 0 {
                p.push("T=0");
            }
            if v & 2 != 0 {
                p.push("T=1");
            }
            if p.is_empty() {
                format!("0x{v:08X}")
            } else {
                p.join(", ")
            }
        }
        AttrKind::KHz => format!("{} kHz", dword(b)?),
        AttrKind::Bps => format!("{} bit/s", dword(b)?),
        AttrKind::Number => dword(b)?.to_string(),
    };
    (!text.is_empty()).then_some(text)
}

/// What Reader info found: the attributes the driver gave, and the part 10 features (None when
/// the reader doesn't answer GET_FEATURE_REQUEST).
#[derive(Clone, Debug, PartialEq)]
pub struct ReaderInfo {
    pub attributes: Vec<(&'static str, String)>,
    pub features: Option<Vec<(u8, u32)>>,
    /// Read through a direct connection opened for it (no connection was open).
    pub opened_for_it: bool,
}

/// Asks the reader for its standard attributes and its PC/SC part 10 features.
pub fn reader_info(link: &mut dyn CardLink) -> ReaderInfo {
    let attributes = READER_ATTRIBUTES
        .iter()
        .filter_map(|(attr, label, kind)| {
            let b = link.attribute(*attr).ok()?;
            Some((*label, attribute_text(*kind, &b)?))
        })
        .collect();
    let features = link
        .control(IOCTL_GET_FEATURE_REQUEST, &[], 256)
        .ok()
        .map(|r| features(&r));
    ReaderInfo {
        attributes,
        features,
        opened_for_it: false,
    }
}

impl ReaderInfo {
    /// The note shown in the exchanges.
    pub fn text(&self) -> String {
        let mut s = String::from(if self.opened_for_it {
            "Reader info (SCardGetAttrib, through a direct connection opened for it):"
        } else {
            "Reader info (SCardGetAttrib):"
        });
        if self.attributes.is_empty() {
            s.push_str("\n  The driver gives none of the standard attributes.");
        }
        for (label, value) in &self.attributes {
            s.push_str(&format!("\n  {label}: {value}"));
        }
        s.push_str("\nPC/SC part 10 features: ");
        match &self.features {
            None => s.push_str("the reader doesn't answer GET_FEATURE_REQUEST"),
            Some(f) if f.is_empty() => s.push_str("none"),
            Some(f) => {
                let names: Vec<String> = f
                    .iter()
                    .map(|(tag, code)| {
                        let name = PART10_FEATURES
                            .iter()
                            .find(|(t, _)| t == tag)
                            .map(|(_, n)| (*n).to_string())
                            .unwrap_or_else(|| format!("feature 0x{tag:02X}"));
                        format!("{name} (0x{code:08X})")
                    })
                    .collect();
                s.push_str(&names.join(", "));
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::fake::FakeSystem;
    use crate::reader::{Proto, Share};

    fn link(f: &FakeSystem) -> Box<dyn CardLink> {
        let mut s = (f.opener())().expect("fake service");
        s.connect("R", Share::Shared, Proto::Any)
            .expect("fake connect")
    }

    #[test]
    fn exchange_errors_read_as_the_service_s_texts() {
        assert_eq!(ExchangeError::NoCard.to_string(), NO_CONNECTION);
        assert_eq!(
            ExchangeError::Reader(Error::NoSmartcard).to_string(),
            error_text(Error::NoSmartcard)
        );
    }

    #[test]
    fn control_codes_follow_each_system() {
        // Windows (winscard).
        assert_eq!(ctl_code_for(true, 3500), 0x0031_36B0);
        assert_eq!(ctl_code_for(true, 3400), 0x0031_3520);
        // pcsc-lite (Linux, macOS).
        assert_eq!(ctl_code_for(false, 1), 0x4200_0001);
        assert_eq!(ctl_code_for(false, 3400), 0x4200_0D48);
        // What this build uses.
        if cfg!(windows) {
            assert_eq!(IOCTL_CCID_ESCAPE, 0x0031_36B0);
        } else {
            assert_eq!(IOCTL_CCID_ESCAPE, 0x4200_0001);
        }
    }

    #[test]
    fn features_are_read_from_their_tlv_list() {
        let reply = [
            0x06, 4, 0x42, 0x33, 0x00, 0x06, // FEATURE_VERIFY_PIN_DIRECT
            0x13, 4, 0x42, 0x00, 0x00, 0x01, // FEATURE_CCID_ESC_COMMAND
        ];
        assert_eq!(
            features(&reply),
            vec![(0x06, 0x4233_0006), (0x13, 0x4200_0001)]
        );
        // A wrong length or a cut entry ends the list there.
        assert_eq!(features(&[0x13, 2, 0x00, 0x01]), vec![]);
        assert_eq!(
            features(&[0x06, 4, 0, 0, 0, 6, 0x13, 4, 0x42]),
            vec![(0x06, 6)]
        );
        assert_eq!(features(&[]), vec![]);
    }

    #[test]
    fn the_escape_code_is_the_readers_when_it_lists_one() {
        let f = FakeSystem::with_card("R", &[0x3B]);
        f.answer_control(
            IOCTL_GET_FEATURE_REQUEST,
            &[],
            &[0x13, 4, 0x12, 0x34, 0x56, 0x78],
        );
        let mut l = link(&f);
        assert_eq!(escape_code(l.as_mut()), EscapeCode::Reported(0x1234_5678));
    }

    #[test]
    fn without_the_feature_the_escape_code_is_the_systems() {
        // The reader refuses GET_FEATURE_REQUEST.
        let f = FakeSystem::with_card("R", &[0x3B]);
        let mut l = link(&f);
        assert_eq!(
            escape_code(l.as_mut()),
            EscapeCode::Default(IOCTL_CCID_ESCAPE)
        );
        // It answers, without FEATURE_CCID_ESC_COMMAND.
        let f = FakeSystem::with_card("R", &[0x3B]);
        f.answer_control(IOCTL_GET_FEATURE_REQUEST, &[], &[0x06, 4, 0, 0, 0, 6]);
        let mut l = link(&f);
        assert_eq!(
            escape_code(l.as_mut()),
            EscapeCode::Default(IOCTL_CCID_ESCAPE)
        );
    }

    #[test]
    fn attributes_read_as_text_versions_and_numbers() {
        use AttrKind::*;
        assert_eq!(attribute_text(Text, b"Identiv\0\0"), Some("Identiv".into()));
        // UTF-16 from a Windows driver.
        let utf16: Vec<u8> = "uTrust\0"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(attribute_text(Text, &utf16), Some("uTrust".into()));
        assert_eq!(attribute_text(Text, b"\0\0"), None);
        assert_eq!(
            attribute_text(Version, &0x0200_0005u32.to_le_bytes()),
            Some("2.0.5 (0x02000005)".into())
        );
        assert_eq!(
            attribute_text(Channel, &0x0020_0000u32.to_le_bytes()),
            Some("USB (0x00200000)".into())
        );
        assert_eq!(
            attribute_text(Protocols, &[3, 0, 0, 0]),
            Some("T=0, T=1".into())
        );
        // pcsc-lite's 8-byte DWORD on 64-bit Linux.
        assert_eq!(
            attribute_text(KHz, &[0xA0, 0x0F, 0, 0, 0, 0, 0, 0]),
            Some("4000 kHz".into())
        );
        assert_eq!(
            attribute_text(Number, &[1, 2]),
            None,
            "too short for a DWORD"
        );
    }

    #[test]
    fn reader_info_lists_what_the_driver_gives_and_the_part_10_features() {
        let f = FakeSystem::with_card("R", &[0x3B]);
        f.attribute(Attribute::VendorName, b"ACME\0")
            .attribute(Attribute::VendorIfdVersion, &0x0102_0003u32.to_le_bytes())
            .answer_control(
                IOCTL_GET_FEATURE_REQUEST,
                &[],
                &[
                    0x06, 4, 0x42, 0x33, 0x00, 0x06, 0x13, 4, 0x42, 0x00, 0x00, 0x01,
                ],
            );
        let mut l = link(&f);
        let info = reader_info(l.as_mut());
        assert_eq!(
            info.attributes,
            vec![
                ("Vendor", "ACME".into()),
                ("Version", "1.2.3 (0x01020003)".into())
            ]
        );
        let t = info.text();
        assert!(t.contains("Vendor: ACME"), "{t}");
        assert!(
            t.contains("VERIFY_PIN_DIRECT (0x42330006), CCID_ESC_COMMAND (0x42000001)"),
            "{t}"
        );
        // A driver that gives nothing, and a reader without GET_FEATURE_REQUEST.
        let bare = FakeSystem::with_card("R", &[0x3B]);
        let mut l = link(&bare);
        let t = reader_info(l.as_mut()).text();
        assert!(t.contains("gives none of the standard attributes"), "{t}");
        assert!(t.contains("doesn't answer GET_FEATURE_REQUEST"), "{t}");
    }

    #[test]
    fn a_zero_version_is_left_out_and_uids_are_read() {
        assert_eq!(attribute_text(AttrKind::Version, &[0, 0, 0, 0]), None);
        assert_eq!(
            uid_text(&[0x04, 0xA2, 0x3B, 0x1C, 0x90, 0x00]),
            "UID (4 bytes, PC/SC part 3 GET DATA): 04 A2 3B 1C"
        );
        assert!(uid_text(&[0x6E, 0x00]).contains("refused class FF"));
        assert!(uid_text(&[0x6A, 0x81]).contains("not for this card"));
        assert_eq!(
            status_names(Status::PRESENT | Status::SPECIFIC),
            "SCARD_PRESENT | SCARD_SPECIFIC"
        );
        assert_eq!(
            reader_state_text(State::PRESENT | State::UNPOWERED),
            "SCARD_STATE_PRESENT: a card is in\nSCARD_STATE_UNPOWERED: the card has no power\n"
        );
    }

    /// Errors name their PC/SC constant and code, so they can be looked up.
    #[test]
    fn errors_name_their_pc_sc_constant_and_code() {
        assert_eq!(
            error_text(Error::NoSmartcard),
            "The reader has no card (SCARD_E_NO_SMARTCARD, 0x8010000C)"
        );
        assert_eq!(
            error_text(Error::NoService),
            "The smart card service isn't running (SCARD_E_NO_SERVICE, 0x8010001D)"
        );
        assert_eq!(
            error_text(Error::WrongChv),
            "The smart card service returned an error (0x8010006B)"
        );
        assert_eq!(
            status_text(Status::SWALLOWED),
            "A card is in place, without power."
        );
    }

    fn sent_apdus(f: &FakeSystem) -> Vec<Vec<u8>> {
        f.sent()
            .into_iter()
            .filter_map(|s| match s {
                crate::reader::fake::Sent::Transmit(a) => Some(a),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn get_response_follows_61xx_and_the_data_is_put_together() {
        let f = FakeSystem::with_card("R", &[0x3B]);
        let select = [0x01, 0xA4, 0x04, 0x00, 0x02, 0x3F, 0x00];
        f.answer(&select, &[0x61, 0x04]);
        // Logical channel 1 is kept in GET RESPONSE's class.
        f.answer(&[0x01, 0xC0, 0x00, 0x00, 0x04], &[0x6F, 0x02, 0x61, 0x02]);
        f.answer(&[0x01, 0xC0, 0x00, 0x00, 0x02], &[0x84, 0x00, 0x90, 0x00]);
        let mut l = link(&f);
        let r = transmit_following(l.as_mut(), &select, 256, true);
        assert_eq!(r.legs.len(), 3);
        let (answer, _) = r.answer.expect("answer");
        assert_eq!(answer, vec![0x6F, 0x02, 0x84, 0x00, 0x90, 0x00]);
        // Without following, only the command is sent.
        let f = FakeSystem::with_card("R", &[0x3B]);
        f.answer(&select, &[0x61, 0x04]);
        let mut l = link(&f);
        let r = transmit_following(l.as_mut(), &select, 256, false);
        assert_eq!(r.answer.expect("answer").0, vec![0x61, 0x04]);
        assert_eq!(sent_apdus(&f), vec![select.to_vec()]);
    }

    #[test]
    fn a_wrong_le_is_corrected_after_6cxx() {
        let f = FakeSystem::with_card("R", &[0x3B]);
        f.answer(&[0x00, 0xB0, 0x00, 0x00, 0x00], &[0x6C, 0x03]);
        f.answer(&[0x00, 0xB0, 0x00, 0x00, 0x03], &[1, 2, 3, 0x90, 0x00]);
        let mut l = link(&f);
        let r = transmit_following(l.as_mut(), &[0x00, 0xB0, 0x00, 0x00, 0x00], 256, true);
        assert_eq!(r.answer.expect("answer").0, vec![1, 2, 3, 0x90, 0x00]);
        assert_eq!(r.legs.len(), 2);
        // Case 3 (no Le): 6Cxx can't be followed, the answer stays.
        assert_eq!(with_le(&[0x00, 0xD6, 0x00, 0x00, 0x01, 0xAA], 2), None);
        assert_eq!(
            with_le(&[0x00, 0x84, 0x00, 0x00], 8),
            Some(vec![0, 0x84, 0, 0, 8])
        );
        assert_eq!(
            with_le(&[0x00, 0x84, 0x00, 0x00, 0x00, 0x00, 0x08], 8),
            None
        );
    }

    #[test]
    fn a_card_that_keeps_asking_stops_after_max_legs() {
        let f = FakeSystem::with_card("R", &[0x3B]);
        f.answer(&[0x00, 0xCA, 0x00, 0x66, 0x00], &[0x61, 0x10]);
        f.answer(&[0x00, 0xC0, 0x00, 0x00, 0x10], &[0xAA, 0x61, 0x10]);
        let mut l = link(&f);
        let r = transmit_following(l.as_mut(), &[0x00, 0xCA, 0x00, 0x66, 0x00], 256, true);
        assert_eq!(r.legs.len(), MAX_LEGS);
        let (answer, _) = r.answer.expect("answer");
        assert!(
            answer.ends_with(&[0x61, 0x10]),
            "the last SW stays: {answer:02X?}"
        );
        assert_eq!(answer.len(), MAX_LEGS - 1 + 2);
    }

    #[test]
    fn get_response_classes() {
        assert_eq!(get_response_class(0x00), 0x00);
        assert_eq!(get_response_class(0x0C), 0x00, "secure messaging dropped");
        assert_eq!(
            get_response_class(0x13),
            0x03,
            "chaining dropped, channel kept"
        );
        assert_eq!(
            get_response_class(0x65),
            0x45,
            "further class: SM dropped, channel kept"
        );
        assert_eq!(get_response_class(0x80), 0x00, "proprietary");
        assert_eq!(get_response_class(0x94), 0x00, "proprietary");
    }
}
