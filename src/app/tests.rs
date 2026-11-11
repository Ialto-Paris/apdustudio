// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! The window's behaviour against a fake reader: no hardware needed. The last tests draw the
//! window headless, to check it lays out without panicking in both themes.

use super::transcript::{Body, Level, Verdict};
use super::*;
use crate::card;
use crate::journal::Journal;
use crate::reader::fake::{FakeCard, FakeReader, FakeSystem, Sent};
use crate::reader::{self, Disp, Error, Proto, Protocol, Share};
use crate::runner;
use crate::scenario::{self, Scenario};

const READER: &str = "Test Reader 0";
const ATR: [u8; 4] = [0x3B, 0x02, 0x14, 0x50];
const SELECT: [u8; 7] = [0x00, 0xA4, 0x00, 0x00, 0x02, 0x7F, 0x10];
const CHALLENGE: [u8; 5] = [0x00, 0x84, 0x00, 0x00, 0x08];

fn opts() -> Options {
    Options {
        expert: false,
        no_time: true,
        command_buffer: 65536,
        response_buffer: 65536,
        screenshot: None,
        demo: false,
        theme: None,
        open_file: None,
        show: None,
        // Nothing kept: tests never touch the person's settings.
        settings: None,
    }
}

fn app_with(f: &FakeSystem) -> ApduStudio {
    let mut a = ApduStudio::create(opts(), f.opener());
    a.settle();
    a
}

/// A script from its lines after the header.
fn script(body: &str) -> Scenario {
    scenario::parse(&format!(
        "{} {}\n{body}",
        scenario::HEADER,
        scenario::VERSION
    ))
    .expect("a valid script")
}

fn connected(f: &FakeSystem) -> ApduStudio {
    let mut a = app_with(f);
    a.do_connect(Share::Shared, Proto::Any);
    a.settle();
    assert!(a.card_open, "not connected: {}", a.status);
    a
}

/// What the person was told: warnings and errors in the exchanges.
fn messages(a: &ApduStudio) -> Vec<String> {
    let mut m = a.transcript.notes(Level::Warning);
    m.extend(a.transcript.notes(Level::Error));
    m
}

fn last_note(a: &ApduStudio) -> String {
    a.transcript
        .notes(Level::Info)
        .last()
        .cloned()
        .unwrap_or_default()
}

/// The last exchange's answer (or error) and verdict.
fn last_reply(a: &ApduStudio) -> Option<(Result<Vec<u8>, String>, Verdict)> {
    match a.transcript.last_exchange() {
        Some(Body::Exchange { reply, verdict, .. }) => Some((reply.clone(), verdict.clone())),
        _ => None,
    }
}

fn transmits(f: &FakeSystem) -> usize {
    f.sent()
        .iter()
        .filter(|s| matches!(s, Sent::Transmit(_)))
        .count()
}

fn run_to_end(a: &mut ApduStudio) {
    a.start_run();
    a.settle();
    for _ in 0..100 {
        if !a.running {
            break;
        }
        a.step_run(false);
        a.settle();
    }
    assert!(!a.running, "the run never finished");
}

#[test]
fn a_stopped_service_is_reported_with_its_code() {
    let f = FakeSystem::default();
    f.state.lock().unwrap().open_error = Some(Error::NoService);
    let a = app_with(&f);
    assert!(!a.service);
    assert_eq!(
        a.status,
        "Can't reach the smart card service: The smart card service isn't running (SCARD_E_NO_SERVICE, 0x8010001D)"
    );
    assert_eq!(messages(&a), vec![a.status.clone()]);
}

#[test]
fn no_reader_is_reported() {
    let a = app_with(&FakeSystem::default());
    assert!(a.readers.is_empty());
    assert_eq!(
        a.status,
        "No smart card reader found. Plug one in, then Refresh."
    );
}

#[test]
fn the_first_reader_is_selected_at_start() {
    let a = app_with(&FakeSystem::with_card(READER, &ATR));
    assert_eq!(a.reader, Some(0));
    assert!(a.status.contains(READER), "{}", a.status);
}

#[test]
fn connecting_shows_the_atr_and_protocol() {
    let f = FakeSystem::with_card(READER, &ATR);
    let a = connected(&f);
    let info = a.card_info.clone().expect("connection shown");
    assert_eq!(info.atr, ATR.to_vec());
    assert_eq!(info.protocol, "T1");
    assert_eq!(info.share, Share::Shared);
    assert!(last_note(&a).contains("protocol T1"), "{}", last_note(&a));
    assert!(
        last_note(&a).contains("ATR 3B 02 14 50"),
        "{}",
        last_note(&a)
    );
    assert_eq!(
        f.sent(),
        vec![Sent::Connect {
            reader: READER.into(),
            share: Share::Shared,
            proto: Proto::Any
        }]
    );
}

#[test]
fn an_empty_reader_waits_for_a_card_then_connects() {
    let f = FakeSystem::with_card(READER, &ATR);
    f.set_card(None);
    let mut a = app_with(&f);
    a.do_connect(Share::Shared, Proto::Any);
    a.settle();
    assert_eq!(a.waiting, Some((Share::Shared, Proto::Any)));
    assert!(!a.card_open);
    a.wait_for_card();
    a.settle();
    a.wait_for_card();
    assert!(!a.card_open, "no card yet");
    f.set_card(Some(FakeCard {
        atr: ATR.to_vec(),
        protocol: Protocol::T1,
    }));
    a.last_presence_check = None;
    a.wait_for_card();
    a.settle();
    a.wait_for_card();
    a.settle();
    assert!(a.card_open, "{}", a.status);
    assert_eq!(a.waiting, None);
}

/// The card poll runs in the background: the controls, which follow `idle`, don't grey out four
/// times a second while waiting.
#[test]
fn waiting_for_a_card_keeps_the_window_idle() {
    let f = FakeSystem::with_card(READER, &ATR);
    f.set_card(None);
    let mut a = app_with(&f);
    a.do_connect(Share::Shared, Proto::Any);
    a.settle();
    assert!(a.waiting.is_some());
    for _ in 0..5 {
        a.last_presence_check = None;
        a.wait_for_card();
        assert!(a.worker.has_pending(), "a poll went out");
        assert!(a.idle(), "a poll doesn't make the window busy");
        a.settle();
        assert!(a.idle());
    }
    assert!(a.waiting.is_some(), "still waiting: no card");
    // A poll is never stacked behind another job.
    a.last_presence_check = None;
    a.wait_for_card();
    let before = f.sent().len();
    a.last_presence_check = None;
    a.wait_for_card();
    a.settle();
    assert_eq!(
        f.sent().len(),
        before,
        "the reader states aren't 'sent' commands"
    );
}

#[test]
fn a_card_taken_out_and_put_back_is_noted_and_reconnect_is_offered() {
    fn look(a: &mut ApduStudio) {
        a.last_presence_check = None;
        a.watch_reader();
        assert!(a.idle(), "a look doesn't make the window busy");
        a.settle();
    }
    let f = FakeSystem::with_card(READER, &ATR);
    let mut a = connected(&f);
    look(&mut a);
    let n = a.transcript.len();
    look(&mut a);
    assert_eq!(a.transcript.len(), n, "nothing changed: nothing noted");
    f.set_card(None);
    look(&mut a);
    assert_eq!(last_note(&a), format!("Card removed from {READER}"));
    assert!(a.card_open && a.card_removed);
    assert!(a.status.contains("Reconnect"), "{}", a.status);
    look(&mut a);
    assert_eq!(a.transcript.len(), n + 1, "noted once");
    f.set_card(Some(FakeCard {
        atr: ATR.to_vec(),
        protocol: Protocol::T1,
    }));
    look(&mut a);
    assert_eq!(
        last_note(&a),
        format!("Card inserted in {READER}\nATR 3B 02 14 50")
    );
    assert!(a
        .transcript
        .entries()
        .last()
        .is_some_and(|e| e.body.has_detail()));
    a.do_reconnect();
    a.settle();
    assert!(!a.card_removed);
    assert!(
        last_note(&a).starts_with("Connected to"),
        "{}",
        last_note(&a)
    );
    let connects = f
        .sent()
        .iter()
        .filter(|s| matches!(s, Sent::Connect { .. }))
        .count();
    assert_eq!(connects, 2);
}

#[test]
fn without_a_connection_insertion_and_removal_are_still_noted() {
    let f = FakeSystem::with_card(READER, &ATR);
    let mut a = app_with(&f);
    a.last_presence_check = None;
    a.watch_reader();
    a.settle();
    f.set_card(None);
    a.last_presence_check = None;
    a.watch_reader();
    a.settle();
    assert_eq!(last_note(&a), format!("Card removed from {READER}"));
    assert!(!a.card_removed, "nothing was connected");
}

#[test]
fn waiting_can_be_cancelled() {
    let f = FakeSystem::with_card(READER, &ATR);
    f.set_card(None);
    let mut a = app_with(&f);
    a.do_connect(Share::Shared, Proto::Any);
    a.settle();
    a.cancel_waiting();
    assert_eq!(a.waiting, None);
    assert_eq!(a.status, "Stopped waiting for a card.");
}

#[test]
fn transmit_sends_the_apdu_and_shows_the_answer() {
    let f = FakeSystem::with_card(READER, &ATR);
    f.answer(&SELECT, &[0x90, 0x00]);
    let mut a = connected(&f);
    a.command_text = "00 A4 00 00 02 7F 10".into();
    a.do_transmit();
    a.settle();
    assert_eq!(f.sent().last(), Some(&Sent::Transmit(SELECT.to_vec())));
    assert_eq!(
        last_reply(&a),
        Some((Ok(vec![0x90, 0x00]), Verdict::Unchecked))
    );
    assert!(messages(&a).is_empty(), "{:?}", messages(&a));
}

#[test]
fn following_61xx_shows_every_command_and_checks_the_whole_answer() {
    let f = FakeSystem::with_card(READER, &ATR);
    f.answer(&SELECT, &[0x61, 0x02]);
    f.answer(&[0x00, 0xC0, 0x00, 0x00, 0x02], &[0x6F, 0x00, 0x90, 0x00]);
    let mut a = connected(&f);
    a.set_follow_up(true);
    assert!(a.settings.follow_up);
    a.scenario = script("step: select\ncard: 00A40000027F10\nexpect: 6F009000\n");
    a.reset_row_results();
    a.set_index(0);
    a.do_transmit();
    a.settle();
    let sent: Vec<Sent> = f.sent().into_iter().rev().take(2).collect();
    assert_eq!(
        sent,
        vec![
            Sent::Transmit(vec![0x00, 0xC0, 0x00, 0x00, 0x02]),
            Sent::Transmit(SELECT.to_vec())
        ]
    );
    // The two exchanges, the verdict on the last, then the whole answer as a note to decode.
    let bodies: Vec<Body> = a
        .transcript
        .entries()
        .rev()
        .take(3)
        .map(|e| e.body.clone())
        .collect();
    let Body::Note { answer, text, .. } = &bodies[0] else {
        panic!("{bodies:?}");
    };
    assert_eq!(answer.as_deref(), Some(&[0x6F, 0x00, 0x90, 0x00][..]));
    assert!(text.contains("put together from 2 commands"), "{text}");
    assert!(bodies[0].has_detail());
    assert!(
        matches!(
            &bodies[1],
            Body::Exchange {
                verdict: Verdict::Passed,
                ..
            }
        ),
        "{bodies:?}"
    );
    assert!(matches!(
        &bodies[2],
        Body::Exchange {
            verdict: Verdict::Unchecked,
            ..
        }
    ));
    assert!(a.row_results[0]
        .as_ref()
        .is_some_and(|r| r.mark == Mark::Passed));
    // Off again: only the command goes.
    a.set_follow_up(false);
    a.do_transmit();
    a.settle();
    assert_eq!(f.sent().last(), Some(&Sent::Transmit(SELECT.to_vec())));
}

#[test]
fn a_wrong_answer_is_marked_with_its_place() {
    let f = FakeSystem::with_card(READER, &ATR);
    f.answer(&SELECT, &[0x6A, 0x82]);
    let mut a = connected(&f);
    a.scenario = script("step: select\ncard: 00A40000027F10\nexpect: 9000\n");
    a.reset_row_results();
    a.set_index(0);
    a.do_transmit();
    a.settle();
    let Some((Ok(_), Verdict::Failed(why))) = last_reply(&a) else {
        panic!("{:?}", last_reply(&a));
    };
    assert_eq!(why, "expected 9000: byte 1 is 6A, expected 90");
    let r = a.row_results[0].clone().expect("row result");
    assert_eq!(r.mark, Mark::Failed);
    assert_eq!(r.short, "6A82");
}

/// With no expected answer, a run judges each command by its status word: 6283 is a warning
/// (amber, not a failure), 6D00 an error (a failure, shown as one in the exchanges).
#[test]
fn a_run_marks_status_words_by_their_category() {
    let f = FakeSystem::with_card(READER, &ATR);
    f.answer(&SELECT, &[0x90, 0x00])
        .answer(&CHALLENGE, &[0x62, 0x83]);
    let mut a = connected(&f);
    // The third command has no scripted answer: the fake replies 6D00.
    a.scenario = script("card: 00A40000027F10\ncard: 0084000008\ncard: 00B2000008\n");
    run_to_end(&mut a);
    assert_eq!(a.status, "Run finished: 1 passed, 1 warning, 1 failed.");
    let marks: Vec<Option<Mark>> = a
        .row_results
        .iter()
        .map(|r| r.as_ref().map(|r| r.mark))
        .collect();
    assert_eq!(
        marks,
        vec![Some(Mark::Passed), Some(Mark::Warning), Some(Mark::Failed)]
    );
    let verdicts: Vec<Verdict> = a
        .transcript
        .entries()
        .filter_map(|e| match &e.body {
            Body::Exchange { verdict, .. } => Some(verdict.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(verdicts.len(), 3);
    assert_eq!(verdicts[0], Verdict::Unchecked);
    assert_eq!(verdicts[1], Verdict::Unchecked, "a warning isn't a failure");
    assert!(
        matches!(&verdicts[2], Verdict::Failed(why) if why.starts_with("SW 6D00, an error")),
        "{verdicts:?}"
    );
}

/// Step sends a list command and judges it as a run would; a command typed in the command line
/// isn't judged.
#[test]
fn a_list_command_without_expected_answer_is_judged_by_its_status_word() {
    let f = FakeSystem::with_card(READER, &ATR);
    let mut a = connected(&f);
    a.scenario = script("card: 00B2000008\n");
    a.reset_row_results();
    a.set_index(0);
    a.do_transmit();
    a.settle();
    assert_eq!(
        a.row_results[0],
        Some(RowResult {
            mark: Mark::Failed,
            short: "6D00".into()
        })
    );
    assert!(
        matches!(last_reply(&a), Some((Ok(_), Verdict::Failed(_)))),
        "{:?}",
        last_reply(&a)
    );
    a.command_text = "00B2000108".into();
    a.do_transmit();
    a.settle();
    assert!(
        matches!(last_reply(&a), Some((Ok(_), Verdict::Unchecked))),
        "{:?}",
        last_reply(&a)
    );
}

#[test]
fn run_scenario_counts_passed_and_failed_commands() {
    let f = FakeSystem::with_card(READER, &ATR);
    f.answer(&SELECT, &[0x90, 0x00])
        .answer(&CHALLENGE, &[1, 2, 3, 4, 5, 6, 7, 8, 0x90, 0x00]);
    let mut a = connected(&f);
    // The third command has no scripted answer: the fake replies 6D00, not the 9000 expected.
    a.scenario = script("step: select\ncard: 00A40000027F10\nexpect: 9000\nstep: challenge\ncard: 0084000008\nexpect: ????????????????9000\nstep: read\ncard: 00B2000008\nexpect: 9000\n");
    run_to_end(&mut a);
    assert_eq!(a.status, "Run finished: 2 passed, 1 failed.");
    assert_eq!(transmits(&f), 3);
    assert!(messages(&a)
        .iter()
        .any(|t| t.starts_with("Some answers weren't the expected ones")));
    let results: Vec<(Mark, String)> = a
        .row_results
        .iter()
        .flatten()
        .map(|r| (r.mark, r.short.clone()))
        .collect();
    assert_eq!(
        results,
        vec![
            (Mark::Passed, "9000".into()),
            (Mark::Passed, "9000".into()),
            (Mark::Failed, "6D00".into())
        ]
    );
    let verdicts: Vec<Verdict> = a
        .transcript
        .entries()
        .filter_map(|e| match &e.body {
            Body::Exchange { verdict, .. } => Some(verdict.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(verdicts.len(), 3);
    assert_eq!(verdicts[0], Verdict::Passed);
    assert!(matches!(verdicts[2], Verdict::Failed(_)));
}

#[test]
fn a_failed_verify_stops_the_run_and_still_shows_its_answer() {
    let f = FakeSystem::with_card(READER, &ATR);
    let verify = [0x00, 0x20, 0x00, 0x00, 0x02, 0x12, 0x34];
    f.answer(&verify, &[0x63, 0xC2]);
    let mut a = connected(&f);
    a.scenario = script("step: pin\ncard: 00200000021234\nstep: after\ncard: 0084000008\n");
    run_to_end(&mut a);
    assert_eq!(transmits(&f), 1);
    assert_eq!(
        last_reply(&a),
        Some((Ok(vec![0x63, 0xC2]), Verdict::Unchecked))
    );
    assert!(
        last_note(&a).contains("Stopped: a VERIFY failed."),
        "{}",
        last_note(&a)
    );
}

#[test]
fn escape_stops_a_run() {
    let f = FakeSystem::with_card(READER, &ATR);
    let mut a = connected(&f);
    a.scenario = script("card: 00A40000027F10\ncard: 0084000008\n");
    a.start_run();
    a.settle();
    a.step_run(true);
    a.settle();
    assert!(!a.running);
    assert!(last_note(&a).contains(runner::STOPPED_BY_USER));
    assert_eq!(transmits(&f), 0);
}

#[test]
fn disconnecting_resets_and_forgets_the_card() {
    let f = FakeSystem::with_card(READER, &ATR);
    let mut a = connected(&f);
    a.do_disconnect(Disp::Reset);
    a.settle();
    assert!(!a.card_open);
    assert_eq!(a.card_info, None);
    assert_eq!(f.sent().last(), Some(&Sent::Disconnect(Disp::Reset)));
}

#[test]
fn choosing_another_reader_closes_the_connection() {
    let f = many_readers(2);
    let mut a = connected(&f);
    a.choose_reader(1);
    a.settle();
    assert!(!a.card_open);
    assert_eq!(a.card_info, None);
    assert_eq!(a.reader, Some(1));
    assert_eq!(f.sent().last(), Some(&Sent::Disconnect(Disp::Leave)));
}

#[test]
fn reader_info_shows_the_standard_attributes_and_features() {
    let f = FakeSystem::with_card(READER, &ATR);
    f.attribute(reader::Attribute::VendorName, b"ACME\0")
        .attribute(reader::Attribute::VendorIfdType, b"R-1\0")
        .answer_control(
            card::IOCTL_GET_FEATURE_REQUEST,
            &[],
            &[0x13, 4, 0x42, 0x00, 0x00, 0x01],
        );
    let mut a = connected(&f);
    a.do_reader_info();
    a.settle();
    let note = last_note(&a);
    assert!(note.contains("Vendor: ACME\n  Model: R-1"), "{note}");
    assert!(note.contains("CCID_ESC_COMMAND (0x42000001)"), "{note}");
    assert_eq!(a.status, "Reader info read.");
    // Only standard calls went out: attributes and GET_FEATURE_REQUEST.
    assert!(f.sent().iter().all(|s| match s {
        Sent::Control(code, _) => *code == card::IOCTL_GET_FEATURE_REQUEST,
        _ => true,
    }));
}

/// A script row that goes to the reader is selected into the CCID escape field, and Step sends
/// it with SCardControl and checks its expected answer.
#[test]
fn a_reader_row_is_sent_as_an_escape_command() {
    let f = FakeSystem::with_card(READER, &ATR);
    f.answer_control(card::IOCTL_CCID_ESCAPE, &[0x1A, 0x00], &[0x01, 0x02]);
    let mut a = connected(&f);
    a.scenario = script("step: version\nreader: 1A 00\nexpect: 01 03\ncard: 0084000008\n");
    a.reset_row_results();
    a.set_index(0);
    assert_eq!(a.escape_text, "1A 00");
    a.step_selected();
    a.settle();
    assert_eq!(transmits(&f), 0, "nothing went to the card");
    assert!(f
        .sent()
        .contains(&Sent::Control(card::IOCTL_CCID_ESCAPE, vec![0x1A, 0x00])));
    assert_eq!(
        a.row_results[0],
        Some(RowResult {
            mark: Mark::Failed,
            short: "2 bytes".into()
        })
    );
    assert!(
        matches!(last_reply(&a), Some((Ok(_), Verdict::Failed(why))) if why == "expected 01 03: byte 2 is 02, expected 03"),
        "{:?}",
        last_reply(&a)
    );
    assert_eq!(a.index, 1, "Step moved on");
}

/// The command line holds hex only: anything else is refused before it goes out.
#[test]
fn the_command_line_refuses_what_isnt_hex() {
    let f = FakeSystem::with_card(READER, &ATR);
    let mut a = connected(&f);
    let before = f.sent().len();
    a.command_text = "2,0 0084000008".into();
    a.send_command_line();
    a.settle();
    assert_eq!(f.sent().len(), before, "nothing went out");
    assert!(
        messages(&a)
            .iter()
            .any(|m| m.starts_with("Not hexadecimal")),
        "{:?}",
        messages(&a)
    );
}

#[test]
fn the_escape_field_starts_empty() {
    let f = FakeSystem::with_card(READER, &ATR);
    let a = app_with(&f);
    assert_eq!(a.escape_text, "");
    assert!(a.escape_history.is_empty());
}

fn feature_requests(f: &FakeSystem) -> usize {
    f.sent()
        .iter()
        .filter(|s| matches!(s, Sent::Control(c, _) if *c == card::IOCTL_GET_FEATURE_REQUEST))
        .count()
}

#[test]
fn escape_uses_the_code_the_reader_lists_once_per_connection() {
    const CODE: u32 = 0x4200_0001;
    let f = FakeSystem::with_card(READER, &ATR);
    f.answer_control(
        card::IOCTL_GET_FEATURE_REQUEST,
        &[],
        &[0x13, 4, 0x42, 0x00, 0x00, 0x01],
    )
    .answer_control(CODE, &[0x20, 0x10], &[0x90, 0x00]);
    let mut a = connected(&f);
    a.escape_text = "2010".into();
    a.do_escape();
    a.settle();
    assert_eq!(
        f.sent().last(),
        Some(&Sent::Control(CODE, vec![0x20, 0x10]))
    );
    assert!(
        a.transcript
            .notes(Level::Info)
            .iter()
            .any(|n| n.contains("0x42000001") && n.contains("FEATURE_CCID_ESC_COMMAND")),
        "{:?}",
        a.transcript.notes(Level::Info)
    );
    assert_eq!(
        last_reply(&a),
        Some((Ok(vec![0x90, 0x00]), Verdict::Unchecked))
    );
    assert_eq!(a.escape_history, vec!["2010".to_string()]);

    // Asked once per connection.
    a.do_escape();
    a.settle();
    assert_eq!(feature_requests(&f), 1);
    a.do_connect(Share::Shared, Proto::Any);
    a.settle();
    a.do_escape();
    a.settle();
    assert_eq!(feature_requests(&f), 2);
}

#[test]
fn a_refused_escape_says_what_the_driver_needs() {
    let f = FakeSystem::with_card(READER, &ATR);
    let mut a = connected(&f);
    a.escape_text = "2010".into();
    a.do_escape();
    a.settle();
    assert_eq!(
        f.sent().last(),
        Some(&Sent::Control(card::IOCTL_CCID_ESCAPE, vec![0x20, 0x10]))
    );
    let m = messages(&a);
    assert!(m.iter().any(|t| t.contains(card::ESCAPE_HINT)), "{m:?}");
}

#[test]
fn get_status_describes_each_reader() {
    let f = FakeSystem::with_card(READER, &ATR);
    let mut a = app_with(&f);
    a.do_get_status();
    a.settle();
    let n = last_note(&a);
    assert!(n.contains(READER), "{n}");
    assert!(n.contains("SCARD_STATE_PRESENT: a card is in"), "{n}");
    assert!(n.contains("ATR 3B 02 14 50"), "{n}");
}

#[test]
fn a_slow_card_does_not_freeze_the_window() {
    let f = FakeSystem::with_card(READER, &ATR);
    f.answer(&SELECT, &[0x90, 0x00]);
    let mut a = connected(&f);
    f.state.lock().unwrap().delay = Duration::from_millis(400);
    a.command_text = "00A40000027F10".into();
    let t = Instant::now();
    a.do_transmit();
    // The click returns at once; the answer is still on its way and the buttons wait.
    assert!(
        t.elapsed() < Duration::from_millis(200),
        "{:?}",
        t.elapsed()
    );
    assert!(a.worker.busy());
    assert!(!a.idle());
    a.pump();
    assert_eq!(last_reply(&a), None);
    a.settle();
    assert_eq!(
        last_reply(&a),
        Some((Ok(vec![0x90, 0x00]), Verdict::Unchecked))
    );
    assert!(a.idle());
}

#[test]
fn escape_during_a_slow_command_stops_after_it() {
    let f = FakeSystem::with_card(READER, &ATR);
    let mut a = connected(&f);
    f.state.lock().unwrap().delay = Duration::from_millis(200);
    a.scenario = script("card: 00A40000027F10\ncard: 0084000008\ncard: 0084000008\n");
    a.start_run();
    a.settle();
    a.step_run(false);
    assert!(a.worker.busy(), "the first command is on its way");
    a.step_run(true);
    assert!(a.running, "the run waits for the command on its way");
    a.settle();
    a.step_run(false);
    assert!(!a.running);
    let n = last_note(&a);
    assert!(n.contains(runner::STOPPED_BY_USER), "{n}");
    assert!(n.contains("Result: 1 command, 0 passed, 1 failed"), "{n}");
    assert_eq!(transmits(&f), 1);
}

#[test]
fn a_log_file_that_cannot_be_written_is_closed_and_reported() {
    let f = FakeSystem::with_card(READER, &ATR);
    f.answer(&SELECT, &[0x90, 0x00]);
    let mut a = connected(&f);
    let p = std::env::temp_dir().join(format!("apdu-studio-ro-{}.log", std::process::id()));
    std::fs::write(&p, "").expect("create");
    a.log = Some(Journal::read_only(&p).expect("open"));
    a.command_text = "00A40000027F10".into();
    a.do_transmit();
    a.settle();
    let _ = std::fs::remove_file(&p);
    assert!(a.log.is_none(), "the log should be closed");
    let m = messages(&a);
    assert_eq!(m.len(), 1, "one report, not one per line: {m:?}");
    assert!(m[0].starts_with("Writing to the log file"), "{m:?}");
    // The command itself still went to the card.
    assert_eq!(
        last_reply(&a),
        Some((Ok(vec![0x90, 0x00]), Verdict::Unchecked))
    );
}

#[test]
fn the_command_line_remembers_what_was_sent() {
    let f = FakeSystem::with_card(READER, &ATR);
    let mut a = connected(&f);
    for c in ["00A40000027F10", "0084000008", "0084000008", "00B2000008"] {
        a.command_text = c.into();
        a.send_command_line();
        a.settle();
    }
    assert_eq!(
        a.history,
        vec!["00A40000027F10", "0084000008", "00B2000008"]
    );
    a.command_text = "typing".into();
    a.history_back();
    assert_eq!(a.command_text, "00B2000008");
    a.history_back();
    a.history_back();
    a.history_back();
    assert_eq!(a.command_text, "00A40000027F10", "stops at the oldest");
    a.history_forward();
    assert_eq!(a.command_text, "0084000008");
    a.history_forward();
    a.history_forward();
    assert_eq!(a.command_text, "", "past the newest: an empty line");
}

#[test]
fn step_sends_the_selected_row_and_moves_on() {
    let f = FakeSystem::with_card(READER, &ATR);
    f.answer(&SELECT, &[0x90, 0x00]);
    let mut a = connected(&f);
    a.scenario =
        script("step: select\ncard: 00A40000027F10\nexpect: 9000\nstep: c\ncard: 0084000008\n");
    a.reset_row_results();
    a.set_index(0);
    a.step_selected();
    a.settle();
    assert_eq!(a.index, 1);
    assert_eq!(
        a.command_text, "0084000008",
        "the next row is in the command line"
    );
    assert_eq!(
        a.row_results[0],
        Some(RowResult {
            mark: Mark::Passed,
            short: "9000".into()
        })
    );
    assert_eq!(
        last_reply(&a),
        Some((Ok(vec![0x90, 0x00]), Verdict::Passed))
    );
    a.step_selected();
    a.settle();
    assert_eq!(a.index, 1, "the last row stays selected");
    assert_eq!(transmits(&f), 2);
}

#[test]
fn step_without_a_card_does_not_move_on() {
    let f = FakeSystem::with_card(READER, &ATR);
    let mut a = app_with(&f);
    a.scenario = script("card: 00A40000027F10\ncard: 0084000008\n");
    a.reset_row_results();
    a.set_index(0);
    a.step_selected();
    assert_eq!(a.index, 0);
    assert!(messages(&a)[0].starts_with("No card connection"));
}

#[test]
fn editing_rows_marks_the_scenario_modified() {
    let f = FakeSystem::with_card(READER, &ATR);
    let mut a = app_with(&f);
    a.scenario = script("step: a\ncard: 00A4\nstep: b\ncard: 0084\n");
    a.reset_row_results();
    a.set_index(0);
    a.add_row();
    assert_eq!(a.index, 1);
    assert_eq!(a.scenario.commands.len(), 3);
    assert_eq!(a.row_results.len(), 3);
    assert!(a.scenario_modified);
    a.scenario.commands[1].text = "00 20 00 00 04 <PIN>".into();
    a.command_edited(1);
    assert!(
        a.scenario.commands[1].incomplete,
        "<...> marks a placeholder"
    );
    assert_eq!(a.command_text, "00 20 00 00 04 <PIN>");
    a.move_row(true);
    assert_eq!(a.index, 0);
    assert_eq!(a.scenario.commands[0].text, "00 20 00 00 04 <PIN>");
    a.move_row(true);
    assert_eq!(a.index, 0, "the first row can't go up");
    a.delete_row();
    assert_eq!(a.scenario.commands.len(), 2);
    assert_eq!(a.scenario.commands[0].text, "00A4");
    assert_eq!(a.command_text, "00A4");
}

#[test]
fn the_demo_scenario_runs_against_the_demo_card() {
    let f = crate::demo::system();
    let mut a = ApduStudio::create(opts(), f.opener());
    a.settle();
    a.do_connect(Share::Shared, Proto::Any);
    a.settle();
    a.scenario =
        scenario::parse(include_str!("../../examples/demo.apdu")).expect("the demo script");
    a.reset_row_results();
    run_to_end(&mut a);
    let results: Vec<Option<(Mark, String)>> = a
        .row_results
        .iter()
        .map(|r| r.clone().map(|r| (r.mark, r.short)))
        .collect();
    assert_eq!(
        results,
        vec![
            Some((Mark::Passed, "9000".into())),
            Some((Mark::Passed, "9000".into())),
            Some((Mark::Passed, "9000".into())),
            Some((Mark::Failed, "6A83".into())),
            Some((Mark::Passed, "6105".into())),
            Some((Mark::Failed, "skipped".into())),
        ],
        "{}",
        a.transcript.to_text()
    );
}

// ----------------------------------------------------------------- settings

/// The theme an app shows at start, in a context of its own.
fn theme_at_start(o: &Options, f: &FakeSystem) -> egui::ThemePreference {
    let a = ApduStudio::create(o.clone(), f.opener());
    let ctx = egui::Context::default();
    a.apply_theme(&ctx);
    ctx.options(|o| o.theme_preference)
}

#[test]
fn the_theme_chosen_is_kept_for_the_next_start() {
    let f = FakeSystem::default();
    let path = crate::settings::tests::temp_file("theme-kept");
    let mut o = opts();
    o.settings = Some(path.clone());
    assert_eq!(theme_at_start(&o, &f), egui::ThemePreference::System);

    let mut a = ApduStudio::create(o.clone(), f.opener());
    let ctx = egui::Context::default();
    a.choose_theme(&ctx, egui::ThemePreference::Dark);
    assert_eq!(
        ctx.options(|o| o.theme_preference),
        egui::ThemePreference::Dark
    );
    drop(a);
    assert_eq!(theme_at_start(&o, &f), egui::ThemePreference::Dark);

    // --theme wins for its run, and leaves the kept theme alone.
    let mut once = o.clone();
    once.theme = Some(egui::Theme::Light);
    assert_eq!(theme_at_start(&once, &f), egui::ThemePreference::Light);
    assert_eq!(
        crate::settings::load(&path).0.theme,
        egui::ThemePreference::Dark
    );
    if let Some(dir) = path.parent().and_then(std::path::Path::parent) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn a_newer_settings_file_is_not_written_over() {
    let f = FakeSystem::default();
    let path = crate::settings::tests::temp_file("theme-newer");
    let newer = "version = 9\ntheme = \"dark\"\nzoom = 2\n";
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("dir");
    }
    std::fs::write(&path, newer).expect("write");
    let mut o = opts();
    o.settings = Some(path.clone());
    assert_eq!(theme_at_start(&o, &f), egui::ThemePreference::Dark);
    let mut a = ApduStudio::create(o, f.opener());
    a.choose_theme(&egui::Context::default(), egui::ThemePreference::Light);
    assert!(a.status.contains("newer version"), "{}", a.status);
    assert_eq!(std::fs::read_to_string(&path).expect("read"), newer);
    if let Some(dir) = path.parent().and_then(std::path::Path::parent) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn a_theme_that_cannot_be_written_says_so() {
    let f = FakeSystem::default();
    let path = crate::settings::tests::temp_file("theme-blocked");
    let Some(root) = path.parent().and_then(std::path::Path::parent) else {
        panic!("no parent")
    };
    std::fs::create_dir_all(root).expect("dir");
    // A file where the settings folder should be.
    std::fs::write(root.join("settings"), b"x").expect("file");
    let mut o = opts();
    o.settings = Some(path.clone());
    let mut a = ApduStudio::create(o, f.opener());
    let ctx = egui::Context::default();
    a.choose_theme(&ctx, egui::ThemePreference::Dark);
    assert_eq!(
        ctx.options(|o| o.theme_preference),
        egui::ThemePreference::Dark
    );
    assert!(a.status.contains("could not be kept"), "{}", a.status);
    let _ = std::fs::remove_dir_all(root);
}

/// Without a settings file (the system names no folder for it), the theme still changes, for
/// this run only, and nothing is said.
#[test]
fn without_a_settings_file_the_theme_lasts_the_run() {
    let f = FakeSystem::default();
    let mut a = ApduStudio::create(opts(), f.opener());
    let ctx = egui::Context::default();
    a.choose_theme(&ctx, egui::ThemePreference::Dark);
    assert_eq!(
        ctx.options(|o| o.theme_preference),
        egui::ThemePreference::Dark
    );
    assert!(!a.status.contains("theme"), "{}", a.status);
}

// ----------------------------------------------------------------- drawing

/// Draws `frames` frames of the window headless, at the default size.
fn draw(a: &mut ApduStudio, dark: bool, frames: usize) {
    draw_at(a, dark, frames, WINDOW_SIZE);
}

fn draw_at(a: &mut ApduStudio, dark: bool, frames: usize, size: [f32; 2]) {
    draw_each(a, dark, frames, size, |_| {});
}

/// Draws `frames` frames and calls `each` after every one.
fn draw_each(
    a: &mut ApduStudio,
    dark: bool,
    frames: usize,
    size: [f32; 2],
    mut each: impl FnMut(&ApduStudio),
) {
    let ctx = egui::Context::default();
    widgets::apply_style(&ctx);
    ctx.set_theme(if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    for _ in 0..frames {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size[0], size[1]),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| a.frame(ui));
        // No renderer here: drop the textures egui would have uploaded.
        out.textures_delta.clear();
        each(a);
    }
}

/// The scenario list fills its panel's width, and the row editor below it ends at the panel's
/// bottom, from the first frame on and without moving afterwards: with no command (a short
/// line in the list, a one-line editor) and with a selected command (a three-field editor).
#[test]
fn the_scenario_list_fills_its_panel_from_the_first_frame() {
    let f = crate::demo::system();
    let mut empty = ApduStudio::create(opts(), f.opener());
    empty.settle();
    let mut loaded = ApduStudio::create(opts(), f.opener());
    loaded.settle();
    loaded.scenario =
        scenario::parse(include_str!("../../examples/demo.apdu")).expect("the demo script");
    loaded.reset_row_results();
    loaded.index = 0;
    for (name, a) in [("empty", &mut empty), ("loaded", &mut loaded)] {
        for size in [WINDOW_SIZE, MIN_WINDOW_SIZE] {
            let mut seen: Vec<ScenarioLayout> = Vec::new();
            draw_each(a, false, 4, size, |a| seen.push(a.scenario_layout));
            let Some(first) = seen.first().copied() else {
                panic!("{name}: nothing drawn");
            };
            let at = format!("{name} at {size:?}: {first:?}");
            assert!(first.panel.is_positive(), "{at}");
            assert!(
                (first.list.width() - first.panel.width()).abs() < 1.0,
                "the list is as wide as its panel, {at}"
            );
            assert!(
                (first.panel.bottom() - first.content_bottom).abs() < 1.0,
                "the editor ends at the panel's bottom, {at}"
            );
            for l in &seen {
                assert_eq!(l.list, first.list, "the list holds still, {at}");
            }
        }
    }
    // A selection changes the editor's height, and the list's with it.
    let editor =
        |a: &ApduStudio| a.scenario_layout.content_bottom - a.scenario_layout.list.bottom();
    assert!(
        editor(&loaded) > editor(&empty) + 20.0,
        "{:?} {:?}",
        loaded.scenario_layout,
        empty.scenario_layout
    );
}

#[test]
fn the_window_draws_in_both_themes_with_a_full_session() {
    let f = crate::demo::system();
    let mut a = ApduStudio::create(opts(), f.opener());
    a.settle();
    draw(&mut a, false, 2);
    a.do_connect(Share::Shared, Proto::Any);
    a.settle();
    a.scenario =
        scenario::parse(include_str!("../../examples/demo.apdu")).expect("the demo script");
    a.reset_row_results();
    run_to_end(&mut a);
    for i in 0..300 {
        a.transcript
            .note(Level::Info, format!("note {i}\nsecond line"));
    }
    let third = a.transcript.entries().nth(3).map(|e| e.id);
    a.transcript.selected = third;
    draw(&mut a, false, 3);
    draw(&mut a, true, 3);
    draw_at(&mut a, true, 3, MIN_WINDOW_SIZE);
    assert!(a.transcript.len() > 300);
    // The connection's note: its ATR decoded in the detail.
    let with_atr = a
        .transcript
        .entries()
        .find(|e| matches!(&e.body, Body::Note { atrs, .. } if !atrs.is_empty()))
        .map(|e| e.id);
    assert!(with_atr.is_some());
    a.transcript.selected = with_atr;
    draw(&mut a, false, 3);
    draw_at(&mut a, true, 3, MIN_WINDOW_SIZE);
    // Filtered: only the problems, then a text no entry holds.
    a.filter.problems_only = true;
    draw(&mut a, false, 3);
    a.filter.text = "nothing holds this".into();
    draw_at(&mut a, true, 3, MIN_WINDOW_SIZE);
}

#[test]
fn notes_with_an_atr_open_its_decoding_and_other_notes_open_nothing() {
    fn last_atrs(a: &ApduStudio) -> Vec<(String, Vec<u8>)> {
        match a.transcript.entries().last().map(|e| &e.body) {
            Some(Body::Note { atrs, .. }) => atrs.clone(),
            other => panic!("not a note: {other:?}"),
        }
    }
    let f = FakeSystem::with_card(READER, &ATR);
    let mut a = connected(&f);
    let ours = vec![(READER.to_string(), ATR.to_vec())];
    assert_eq!(last_atrs(&a), ours, "Connected");
    a.do_card_state();
    a.settle();
    assert_eq!(last_atrs(&a), ours, "Card state");
    a.do_get_status();
    a.settle();
    assert_eq!(last_atrs(&a), ours, "Readers' state");
    assert!(a
        .transcript
        .entries()
        .last()
        .is_some_and(|e| e.body.has_detail()));
    a.transcript.note(Level::Info, "Disconnected");
    assert!(last_atrs(&a).is_empty());
    assert!(!a
        .transcript
        .entries()
        .last()
        .is_some_and(|e| e.body.has_detail()));
}

// ----------------------------------------------------------------- live

/// The real smart card service through the reader thread; run with `cargo test -- --ignored`.
/// Only lists the readers and reads their state: it never connects to a card.
#[test]
#[ignore]
fn the_real_service_through_the_reader_thread() {
    let mut a = ApduStudio::create(opts(), reader::PcscSystem::opener());
    a.settle();
    eprintln!("start: {}", a.status);
    if a.readers.is_empty() {
        assert!(
            a.status.contains("Cannot find a smart card reader")
                || a.status.contains("SCardEstablishContext"),
            "{}",
            a.status
        );
        return;
    }
    assert!(a.status.starts_with("Reader: "), "{}", a.status);
    a.do_get_status();
    a.settle();
    eprintln!("{}", last_note(&a));
    assert_eq!(a.status, "Readers' state read.");
    assert!(last_note(&a).contains(&a.readers[0]));
    assert!(!a.card_open, "nothing was connected");
}

/// A real card, read only; run with `cargo test real_card -- --ignored --nocapture`.
/// Connects in shared mode, reads the card's state and the reader's info, and disconnects
/// with Leave Card (no reset). It sends no APDU to the card, so any card may be in the reader.
#[test]
#[ignore]
fn a_real_card_read_only() {
    let mut a = ApduStudio::create(opts(), reader::PcscSystem::opener());
    a.settle();
    let Some(name) = a.readers.first().cloned() else {
        eprintln!("no reader: {}", a.status);
        return;
    };
    a.do_connect(Share::Shared, Proto::Any);
    a.settle();
    if !a.card_open {
        // An empty reader waits for a card instead.
        eprintln!("{name}: no card connected ({})", a.status);
        return;
    }
    eprintln!("{}\n{}", a.status, last_note(&a));
    assert!(a.status.starts_with("Card is connected"), "{}", a.status);
    assert!(a.card_info.as_ref().is_some_and(|i| !i.atr.is_empty()));

    a.do_card_state();
    a.settle();
    eprintln!("{}\n{}", a.status, last_note(&a));
    assert_eq!(a.status, "Card state read.");

    // Reader info: standard attributes (SCardGetAttrib) and part 10 features, nothing to the card.
    a.do_reader_info();
    a.settle();
    eprintln!("{}\n{}", a.status, last_note(&a));
    assert_eq!(a.status, "Reader info read.");

    // GET_FEATURE_REQUEST (PC/SC part 10) asks the reader which features it has: nothing goes
    // to the card. Readers without part 10 support refuse it, which is fine.
    a.worker.call(
        |d| {
            let l = d.link()?;
            let reply = l.control(card::IOCTL_GET_FEATURE_REQUEST, &[], 256);
            Some((reply, card::escape_code(l)))
        },
        |_: &mut ApduStudio, r| match r {
            Ok(Some((Ok(reply), code))) => eprintln!(
                "features: {:02X?}\n{}",
                card::features(&reply),
                code.describe()
            ),
            Ok(Some((Err(e), code))) => {
                eprintln!("GET_FEATURE_REQUEST refused: {e:?}\n{}", code.describe())
            }
            other => eprintln!("no answer: {other:?}"),
        },
    );
    a.settle();

    a.do_disconnect(Disp::Leave);
    a.settle();
    eprintln!("{}", a.status);
    assert!(!a.card_open, "{}", a.status);
}

/// Not a real test: dumps the statuses, what was sent, the log and the exchanges for a few
/// script runs, so two versions can be compared (GOLDEN_OUT=<file> cargo test golden --
/// --ignored).
#[test]
#[ignore]
fn golden_dump() {
    let out = std::env::var("GOLDEN_OUT").expect("GOLDEN_OUT");
    let verify: [u8; 7] = [0x00, 0x20, 0x00, 0x00, 0x02, 0x12, 0x34];
    let cases: [(&str, u32, Option<u32>); 4] = [
        (
            "step: select\ncard: 00A40000027F10\nexpect: 9000\nstep: challenge\ncard: 0084000008\n\
             expect: ?? ?? ?? ?? ?? ?? ?? ?? 90 00\nstep: read\ncard: 00B2000008\nexpect: 9000\n\
             step: incomplete\ncard: 00A4 <P1>\nstep: short\ncard: 0084000008\nexpect: 0102 *\n",
            2,
            None,
        ),
        (
            "step: pin\ncard: 00200000021234\nstep: after\ncard: 0084000008\n",
            3,
            None,
        ),
        (
            "step: a\ncard: 00A40000027F10\nstep: b\ncard: 0084000008\n",
            3,
            Some(4),
        ),
        ("step: reader\nreader: 0084000008\nexpect: 9000\n", 1, None),
    ];
    let mut dump = String::new();
    for (i, (text, iters, esc_at)) in cases.iter().enumerate() {
        let f = FakeSystem::with_card(READER, &ATR);
        f.answer(&SELECT, &[0x90, 0x00])
            .answer(&CHALLENGE, &[1, 2, 3, 4, 5, 6, 7, 8, 0x90, 0x00])
            .answer(&verify, &[0x63, 0xC2]);
        let mut a = connected(&f);
        let logp = std::env::temp_dir().join(format!("apdu-studio-golden-{i}.log"));
        a.log = Some(Journal::create(&logp).expect("log"));
        a.scenario = script(text);
        a.repeat_count = iters.to_string();
        a.start_run();
        a.settle();
        dump.push_str(&format!("== case {i}\n[status] {}\n", a.status));
        let mut n = 0;
        while a.running && n < 200 {
            a.step_run(*esc_at == Some(n));
            a.settle();
            dump.push_str(&format!(
                "[step {n}] index={} command={:?} status={}\n",
                a.index, a.command_text, a.status
            ));
            n += 1;
        }
        a.log = None;
        let exchanges: Vec<String> = a
            .transcript
            .entries()
            .map(|e| match &e.body {
                Body::Exchange {
                    channel,
                    sent,
                    reply,
                    verdict,
                    ..
                } => format!("{channel:?} {sent:02X?} -> {reply:02X?} {verdict:?}"),
                Body::Note { level, text, .. } => format!("{level:?}: {text}"),
            })
            .collect();
        dump.push_str(&format!(
            "[exchanges]\n{}\n[sent] {:?}\n[log]\n{}\n",
            exchanges.join("\n"),
            f.sent(),
            std::fs::read_to_string(&logp).expect("read log")
        ));
    }
    std::fs::write(out, dump).expect("write dump");
}

/// A fake service with `n` readers, a card in each.
fn many_readers(n: usize) -> FakeSystem {
    let f = FakeSystem::with_card(READER, &ATR);
    {
        let mut s = f.state.lock().unwrap();
        for i in 1..n {
            s.readers.push(FakeReader {
                name: format!("Test Reader {i}"),
                card: Some(FakeCard {
                    atr: ATR.to_vec(),
                    protocol: Protocol::T1,
                }),
            });
        }
    }
    f
}

#[test]
fn more_than_four_readers_are_listed_and_described() {
    let f = many_readers(12);
    let mut a = app_with(&f);
    assert_eq!(a.readers.len(), 12);
    assert!(a.status.starts_with("Reader: "), "{}", a.status);
    // 12 readers need two status calls on Windows (10 at most each).
    a.do_get_status();
    a.settle();
    assert_eq!(a.status, "Readers' state read.");
    let n = last_note(&a);
    for i in 0..12 {
        assert!(
            n.contains(&format!("{}. Test Reader {i}: ", i + 1)),
            "reader {i} missing:\n{n}"
        );
    }
}

#[test]
fn the_reader_list_is_capped() {
    let a = app_with(&many_readers(card::MAX_READERS + 3));
    assert_eq!(a.readers.len(), card::MAX_READERS);
    assert!(messages(&a)
        .iter()
        .any(|m| m.ends_with("only the first 64 are listed.")));
}

// ----------------------------------------------------------------- help

/// Runs one frame per entry, pressing that key (if any) in it.
fn frames_with_keys(a: &mut ApduStudio, keys: &[Option<egui::Key>]) {
    let ctx = egui::Context::default();
    widgets::apply_style(&ctx);
    for key in keys {
        let events = key
            .map(|k| {
                vec![egui::Event::Key {
                    key: k,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }]
            })
            .unwrap_or_default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(WINDOW_SIZE[0], WINDOW_SIZE[1]),
            )),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| a.frame(ui));
        out.textures_delta.clear();
    }
}

#[test]
fn f1_opens_the_help_and_f1_or_esc_closes_it() {
    let f = FakeSystem::default();
    let mut a = app_with(&f);
    frames_with_keys(&mut a, &[None, Some(egui::Key::F1), None]);
    assert!(a.help.open, "F1 opens");
    frames_with_keys(&mut a, &[Some(egui::Key::F1)]);
    assert!(!a.help.open, "F1 again closes");
    frames_with_keys(
        &mut a,
        &[Some(egui::Key::F1), None, Some(egui::Key::Escape)],
    );
    assert!(!a.help.open, "Esc closes");
}

#[test]
fn every_help_topic_draws_in_both_themes() {
    let f = crate::demo::system();
    let mut a = ApduStudio::create(opts(), f.opener());
    a.settle();
    a.help.open = true;
    for topic in 0..help::TOPICS.len() {
        a.help.topic = topic;
        draw(&mut a, false, 2);
        draw_at(&mut a, true, 2, MIN_WINDOW_SIZE);
        assert!(a.help.open);
        assert_eq!(a.help.topic, topic, "the topic stays chosen");
    }
}

#[test]
fn the_help_search_picks_a_matching_topic() {
    let f = FakeSystem::default();
    let mut a = app_with(&f);
    a.help.open = true;
    a.help.topic = 0;
    a.help.query = "EscapeCommandEnable".into();
    draw(&mut a, false, 2);
    assert_eq!(
        help::TOPICS.get(a.help.topic).map(|t| t.title),
        Some("CCID escape")
    );
    // Nothing found: the topic list is empty and drawing still works.
    a.help.query = "no such words anywhere".into();
    draw(&mut a, true, 2);
    assert!(a.help.open);
}

// ----------------------------------------------------------------- version

#[test]
fn the_version_comes_from_cargo_and_names_the_build() {
    assert_eq!(APP_VERSION, env!("CARGO_PKG_VERSION"));
    let line = version_line();
    assert!(
        line.starts_with(&format!("ApduStudio {} (", APP_VERSION)),
        "{line}"
    );
    match BUILD_COMMIT.map(str::trim).filter(|c| !c.is_empty()) {
        // CI sets the commit: its first seven digits.
        Some(c) => assert_eq!(build_label(), c.get(..7).unwrap_or(c)),
        None => assert_eq!(build_label(), "local build"),
    }
}

#[test]
fn about_shows_the_version_build_and_where_things_are_kept() {
    let f = FakeSystem::default();
    let mut o = opts();
    o.settings = Some(std::path::PathBuf::from("settings-here.toml"));
    let mut a = ApduStudio::create(o, f.opener());
    a.settle();
    let rows = dialogs::about_rows(&a);
    let get = |name: &str| {
        rows.iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    assert_eq!(get("Version"), APP_VERSION);
    assert_eq!(get("Build"), build_label());
    assert_eq!(get("Licence"), crate::LICENSE);
    assert!(get("System").contains(std::env::consts::OS));
    assert_eq!(get("Settings"), "settings-here.toml");
    assert!(get("Log file").ends_with("ApduStudio.log"));

    // Opened from the help, drawn, and closed with Enter.
    a.dialogs.push_back(dialogs::Dialog::About);
    draw(&mut a, false, 2);
    assert_eq!(a.dialogs.len(), 1, "it stays until answered");
    frames_with_keys(&mut a, &[Some(egui::Key::Enter)]);
    assert!(a.dialogs.is_empty(), "Enter closes it");
}

// ----------------------------------------------------------------- reader info, UID

#[test]
fn reader_info_without_a_connection_opens_a_direct_one_for_it() {
    let f = FakeSystem::with_card(READER, &ATR);
    f.attribute(reader::Attribute::VendorName, b"ACME\0");
    let mut a = app_with(&f);
    assert!(!a.card_open);
    a.do_reader_info();
    a.settle();
    let note = last_note(&a);
    assert!(
        note.contains("through a direct connection opened for it"),
        "{note}"
    );
    assert!(note.contains("Vendor: ACME"), "{note}");
    let sent = f.sent();
    assert!(sent.contains(&Sent::Connect {
        reader: READER.into(),
        share: Share::Direct,
        proto: Proto::Any
    }));
    assert_eq!(
        sent.last(),
        Some(&Sent::Disconnect(Disp::Leave)),
        "closed, leaving the card as it is"
    );
    assert!(!a.card_open, "the window's connection state is unchanged");
}

#[test]
fn uid_sends_pc_sc_get_data_and_reads_the_answer() {
    let f = FakeSystem::with_card(READER, &ATR);
    f.answer(&card::GET_UID, &[0x04, 0xA2, 0x3B, 0x1C, 0x90, 0x00]);
    let mut a = connected(&f);
    a.do_get_uid();
    a.settle();
    assert_eq!(
        f.sent().last(),
        Some(&Sent::Transmit(card::GET_UID.to_vec()))
    );
    assert!(last_note(&a).contains("04 A2 3B 1C"), "{}", last_note(&a));
}
