// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! `--demo`: a simulated reader holding a card that answers a few common commands, to try the
//! window without hardware. It is the tests' fake reader; nothing here talks to a real reader.
//! `examples/demo.apdu` is a script written for it.

use crate::card::{IOCTL_CCID_ESCAPE, IOCTL_GET_FEATURE_REQUEST};
use crate::reader::fake::FakeSystem;
use crate::reader::Attribute;

pub const READER: &str = "ApduStudio Demo Reader 0";
pub const ATR: [u8; 13] = [
    0x3B, 0x88, 0x80, 0x01, 0x00, 0x00, 0x00, 0x00, 0x77, 0x81, 0x80, 0x00, 0x7F,
];

/// The simulated service: one reader, one card. Unknown commands get 6D00.
pub fn system() -> FakeSystem {
    let f = FakeSystem::with_card(READER, &ATR);
    // SELECT the master file.
    f.answer(&[0x00, 0xA4, 0x00, 0x00, 0x02, 0x3F, 0x00], &[0x90, 0x00])
        // SELECT an application by AID: its FCI.
        .answer(
            &[
                0x00, 0xA4, 0x04, 0x00, 0x07, 0xA0, 0x00, 0x00, 0x00, 0x04, 0x10, 0x10,
            ],
            &[
                0x6F, 0x0F, 0x84, 0x07, 0xA0, 0x00, 0x00, 0x00, 0x04, 0x10, 0x10, 0xA5, 0x04, 0x50,
                0x02, 0x44, 0x4D, 0x90, 0x00,
            ],
        )
        // GET CHALLENGE.
        .answer(
            &[0x00, 0x84, 0x00, 0x00, 0x08],
            &[0x3A, 0x91, 0x0C, 0x5E, 0x77, 0x02, 0xD4, 0x18, 0x90, 0x00],
        )
        // READ RECORD 1 of SFI 1: record not found.
        .answer(&[0x00, 0xB2, 0x01, 0x0C, 0x00], &[0x6A, 0x83])
        // VERIFY with a wrong PIN: two tries left.
        .answer(
            &[0x00, 0x20, 0x00, 0x80, 0x04, 0x31, 0x32, 0x33, 0x35],
            &[0x63, 0xC2],
        )
        // GET DATA: more data waiting (61xx).
        .answer(&[0x80, 0xCA, 0x9F, 0x36, 0x00], &[0x61, 0x05]);
    // Reader info: the standard attributes a CCID driver gives, and one part 10 feature.
    f.attribute(Attribute::VendorName, b"ApduStudio\0")
        .attribute(Attribute::VendorIfdType, b"Demo Reader\0")
        .attribute(Attribute::VendorIfdVersion, &0x0100_0000u32.to_le_bytes())
        .attribute(Attribute::VendorIfdSerialNo, b"0000-DEMO\0")
        .attribute(Attribute::ChannelId, &0x0020_0000u32.to_le_bytes())
        .attribute(Attribute::AsyncProtocolTypes, &3u32.to_le_bytes())
        .answer_control(
            IOCTL_GET_FEATURE_REQUEST,
            &[],
            &[&[0x13u8, 4][..], &IOCTL_CCID_ESCAPE.to_be_bytes()].concat(),
        );
    f
}
