// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! What each control does: checks, then a job for the reader thread and what to do with its
//! answer. Answers go to the exchanges view, and their lines to the log file, laid out as in a
//! run's report (`runner`).

use std::time::{Duration, Instant};

use super::transcript::{hex, Channel, Level, Verdict};
use super::{ApduStudio, CardInfo, RowResult};
use crate::card;
use crate::card::ExchangeError;
use crate::iso;
use crate::reader::{self, CardStatus, Disp, Proto, Share, State};
use crate::runner::{time_taken, Mark};
use crate::scenario::{self, HexError};
use crate::worker::{Devices, WorkerError};

/// What the reader thread found when asked for the reader list.
struct Listing {
    /// The service could not be opened (it wasn't open before).
    open_error: Option<reader::Error>,
    /// The readers, or why they couldn't be listed (None: no service to ask).
    readers: Result<Vec<String>, Option<reader::Error>>,
}

/// On the reader thread: opens the service if needed, then lists the readers.
fn list_readers(d: &mut Devices) -> Listing {
    let open_error = if d.system.is_none() {
        d.open().err()
    } else {
        None
    };
    let readers = match d.system.as_mut() {
        Some(s) => s.readers().map_err(Some),
        None => Err(None),
    };
    Listing {
        open_error,
        readers,
    }
}

/// How a Card Connect went.
enum Connect {
    NoService,
    /// The reader is empty: wait for a card.
    Waiting,
    Connected {
        elapsed: Duration,
        status: Option<CardStatus>,
    },
    Failed(reader::Error),
}

/// On the reader thread: closes the previous connection and connects to the card in `name`.
fn connect(d: &mut Devices, name: &str, share: Share, proto: Proto) -> Connect {
    let Some(system) = d.system.as_mut() else {
        return Connect::NoService;
    };
    if share != Share::Direct {
        let empty = system
            .reader_states(&[name.to_string()])
            .map(|s| s.first().is_some_and(|r| r.state.contains(State::EMPTY)))
            .unwrap_or(false);
        if empty {
            return Connect::Waiting;
        }
    }
    if let Some(old) = d.card.take() {
        let _ = old.disconnect(Disp::Leave);
    }
    let Some(system) = d.system.as_mut() else {
        return Connect::NoService;
    };
    let t = Instant::now();
    match system.connect(name, share, proto) {
        Ok(c) => {
            let elapsed = t.elapsed();
            let status = c.status().ok();
            d.card = Some(c);
            d.escape = None;
            Connect::Connected { elapsed, status }
        }
        Err(e) => Connect::Failed(e),
    }
}

const NO_SERVICE: &str =
    "The smart card service isn't running. Start it (on Linux, the pcscd service), then Refresh.";
const NO_READER: &str = "No smart card reader found. Plug one in, then Refresh.";
/// How often the selected reader is looked at while waiting for a card, and otherwise.
pub(super) const WAIT_POLL: Duration = Duration::from_millis(250);
pub(super) const WATCH_POLL: Duration = Duration::from_millis(500);

impl ApduStudio {
    pub(super) fn start_resource_manager(&mut self) {
        self.worker.call(list_readers, |app: &mut ApduStudio, r| {
            let l = match r {
                Ok(l) => l,
                Err(e) => {
                    app.status = e.to_string();
                    return;
                }
            };
            if let Some(e) = l.open_error {
                app.status = format!(
                    "Can't reach the smart card service: {}",
                    card::error_text(e)
                );
                app.transcript.note(Level::Error, app.status.clone());
                return;
            }
            app.apply_listing(l.readers);
            if app.readers.is_empty() {
                app.status = NO_READER.to_string();
            } else {
                app.reader = Some(0);
                app.status = format!("Reader: {}", app.reader_name());
                if app.opts.demo {
                    // The simulated card is always there: connect at once.
                    app.do_connect(app.share, app.proto);
                }
            }
        });
    }

    /// Lists the readers again; the chosen one stays chosen if it is still there.
    pub(super) fn refresh_readers(&mut self) {
        let before = self.reader_name();
        self.worker
            .call(list_readers, move |app: &mut ApduStudio, r| match r {
                Ok(l) => {
                    if let Some(e) = l.open_error {
                        app.status = format!(
                            "Can't reach the smart card service: {}",
                            card::error_text(e)
                        );
                        app.transcript.note(Level::Error, app.status.clone());
                        return;
                    }
                    app.apply_listing(l.readers);
                    if app.readers.is_empty() {
                        app.reader = None;
                        app.message(NO_READER);
                    } else {
                        let i = app.readers.iter().position(|n| *n == before).unwrap_or(0);
                        app.reader = Some(i);
                        app.status = format!("{} reader(s) found.", app.readers.len());
                    }
                }
                Err(e) => app.status = e.to_string(),
            });
    }

    /// Uses reader `choice`. A connection to another reader is closed first.
    pub(super) fn choose_reader(&mut self, choice: usize) {
        if self.reader != Some(choice) && self.card_open {
            self.card_open = false;
            self.card_info = None;
            self.waiting = None;
            self.worker.call(
                |d| {
                    if let Some(c) = d.card.take() {
                        let _ = c.disconnect(Disp::Leave);
                    }
                },
                |_: &mut ApduStudio, _| {},
            );
        }
        self.reader = Some(choice);
        self.status = format!("Reader: {}", self.reader_name());
        let l = format!("{}\n", self.status);
        self.log(&l);
    }

    pub(super) fn apply_listing(&mut self, readers: Result<Vec<String>, Option<reader::Error>>) {
        self.readers.clear();
        self.service = !matches!(readers, Err(None));
        match readers {
            Ok(list) => {
                if list.len() > card::MAX_READERS {
                    // A message: the status is replaced by the selected reader right after.
                    self.message(format!(
                        "{} readers are connected: only the first {} are listed.",
                        list.len(),
                        card::MAX_READERS
                    ));
                }
                self.readers = list.into_iter().take(card::MAX_READERS).collect();
            }
            Err(None) | Err(Some(reader::Error::NoReadersAvailable)) => {}
            Err(Some(e)) => {
                self.status = format!("Can't list the readers: {}", card::error_text(e));
            }
        }
    }

    pub(super) fn check_hex(&mut self, text: &str) -> Option<Vec<u8>> {
        match scenario::parse_hex(text) {
            Ok(b) if b.is_empty() => {
                self.message("Nothing to send: type the bytes in hex.");
                None
            }
            Ok(b) => {
                if b.len() > self.opts.command_buffer {
                    self.message(format!(
                        "The command has {} bytes, more than the command buffer holds ({}). Start ApduStudio with --command-buffer N (up to 65536) to send longer ones.",
                        b.len(),
                        self.opts.command_buffer
                    ));
                    None
                } else {
                    Some(b)
                }
            }
            Err(HexError::NotHex) => {
                self.message("Not hexadecimal: use 0-9, A-F and spaces.");
                None
            }
            Err(HexError::Incomplete) => {
                self.message("An odd number of hex digits: a byte is two digits.");
                None
            }
        }
    }

    pub(super) fn need_card(&mut self) -> bool {
        if !self.card_open {
            self.message(card::NO_CONNECTION);
            return false;
        }
        true
    }

    // ----------------------------------------------------------------- connection

    pub(super) fn do_connect(&mut self, share: Share, proto: Proto) {
        let Some(name) = self.reader.and_then(|i| self.readers.get(i)).cloned() else {
            if self.service {
                self.message("Choose a reader first.");
            } else {
                self.message(NO_SERVICE);
            }
            return;
        };
        if !self.service {
            self.message(NO_SERVICE);
            return;
        }
        self.waiting = None;
        let reader = name.clone();
        self.worker.call(
            move |d| connect(d, &reader, share, proto),
            move |app: &mut ApduStudio, r| app.connected(r, &name, share, proto),
        );
    }

    fn connected(
        &mut self,
        r: Result<Connect, WorkerError>,
        name: &str,
        share: Share,
        proto: Proto,
    ) {
        match r {
            Err(e) => self.status = e.to_string(),
            Ok(Connect::NoService) => self.message(NO_SERVICE),
            Ok(Connect::Waiting) => {
                self.card_seen = false;
                self.last_presence_check = None;
                self.waiting = Some((share, proto));
                self.status = format!("Waiting for a card in {name}...");
            }
            Ok(Connect::Connected { elapsed, status }) => {
                self.card_removed = false;
                let atr = status.as_ref().map(|s| s.atr.clone()).unwrap_or_default();
                let protocol = status
                    .and_then(|s| s.protocol)
                    .map(|p| format!("{:?}", p))
                    .unwrap_or_else(|| "undefined".to_string());
                self.card_open = true;
                self.transcript.note_atrs(
                    Level::Info,
                    format!(
                        "Connected to {name} ({share:?}, protocol {protocol})\nATR {}",
                        hex(&atr)
                    ),
                    if atr.is_empty() {
                        Vec::new()
                    } else {
                        vec![(name.to_string(), atr.clone())]
                    },
                );
                self.card_info = Some(CardInfo {
                    share,
                    protocol,
                    atr,
                });
                self.status = format!("Connected ({})", time_taken(elapsed));
                let l = format!(
                    "Connected to {name} ({share:?}, {proto:?}), ATR {}{}\n",
                    scenario::hex_spaced(
                        &self
                            .card_info
                            .as_ref()
                            .map(|c| c.atr.clone())
                            .unwrap_or_default()
                    ),
                    self.timed(elapsed)
                );
                self.log(&l);
            }
            Ok(Connect::Failed(e)) => {
                // The previous connection, if any, was closed before connecting.
                self.card_open = false;
                self.card_info = None;
                self.status = card::error_text(e);
                self.transcript
                    .note(Level::Error, format!("Connect failed: {}", self.status));
            }
        }
    }

    /// Stops waiting for a card.
    pub(super) fn cancel_waiting(&mut self) {
        if self.waiting.take().is_some() {
            self.status = "Stopped waiting for a card.".into();
        }
    }

    pub(super) fn do_disconnect(&mut self, mode: Disp) {
        if !self.card_open {
            self.message(card::NO_CONNECTION);
            return;
        }
        self.worker.call(
            move |d| {
                let Some(c) = d.card.take() else {
                    return Err(None);
                };
                let t = Instant::now();
                match c.disconnect(mode) {
                    Ok(()) => Ok(t.elapsed()),
                    Err((c, e)) => {
                        d.card = Some(c);
                        Err(Some(e))
                    }
                }
            },
            move |app: &mut ApduStudio, r| match r {
                Ok(Ok(elapsed)) => {
                    app.card_open = false;
                    app.card_info = None;
                    app.card_removed = false;
                    app.transcript
                        .note(Level::Info, format!("Disconnected ({mode:?})"));
                    app.status = format!("Disconnected ({})", time_taken(elapsed));
                    let l = format!("Disconnected ({mode:?}){}\n", app.timed(elapsed));
                    app.log(&l);
                }
                Ok(Err(Some(e))) => {
                    app.status = card::error_text(e);
                    app.transcript
                        .note(Level::Error, format!("Disconnect failed: {}", app.status));
                }
                Ok(Err(None)) => {
                    app.card_open = false;
                    app.card_info = None;
                    app.message(card::NO_CONNECTION);
                }
                Err(e) => app.status = e.to_string(),
            },
        );
    }

    // ----------------------------------------------------------------- readings

    pub(super) fn do_get_status(&mut self) {
        if !self.service {
            self.message(NO_SERVICE);
            return;
        }
        if self.readers.is_empty() {
            self.status = NO_READER.into();
            return;
        }
        let names = self.readers.clone();
        self.worker.call(
            move |d| d.system.as_mut().map(|s| s.reader_states(&names)),
            |app: &mut ApduStudio, r| match r {
                Ok(Some(Ok(states))) => {
                    let mut s = String::from("Readers' state\n");
                    let atrs = states
                        .iter()
                        .filter(|st| !st.atr.is_empty())
                        .map(|st| (st.name.clone(), st.atr.clone()))
                        .collect();
                    for (i, st) in states.iter().enumerate() {
                        s.push_str(&format!(
                            "{}. {}: 0x{:X}\n",
                            i + 1,
                            st.name,
                            st.state.bits()
                        ));
                        s.push_str(&card::reader_state_text(st.state));
                        if !st.atr.is_empty() {
                            s.push_str(&format!("ATR {}\n", scenario::hex_spaced(&st.atr)));
                        }
                        s.push('\n');
                    }
                    app.transcript.note_atrs(Level::Info, s.trim_end(), atrs);
                    app.status = "Readers' state read.".into();
                }
                Ok(Some(Err(e))) => app.status = card::error_text(e),
                Ok(None) => app.message(NO_SERVICE),
                Err(e) => app.status = e.to_string(),
            },
        );
    }

    pub(super) fn do_card_state(&mut self) {
        if !self.need_card() {
            return;
        }
        self.worker.call(
            |d| d.card.as_ref().map(|c| c.status()),
            |app: &mut ApduStudio, r| match r {
                Ok(Some(Ok(st))) => {
                    let s = st.status;
                    let protocol = st
                        .protocol
                        .map(|p| format!("{:?}", p))
                        .unwrap_or_else(|| "undefined".into());
                    let reader = app
                        .reader
                        .and_then(|i| app.readers.get(i))
                        .cloned()
                        .unwrap_or_default();
                    app.transcript.note_atrs(
                        Level::Info,
                        format!(
                            "Card state: {}\nStatus 0x{:X} ({})\nProtocol {}\nATR {}",
                            card::status_text(s),
                            s.bits(),
                            card::status_names(s),
                            protocol,
                            hex(&st.atr)
                        ),
                        if st.atr.is_empty() {
                            Vec::new()
                        } else {
                            vec![(reader, st.atr.clone())]
                        },
                    );
                    if let Some(info) = app.card_info.as_mut() {
                        info.protocol = protocol;
                        info.atr = st.atr;
                    }
                    app.status = "Card state read.".into();
                }
                Ok(Some(Err(e))) => {
                    if matches!(e, reader::Error::RemovedCard | reader::Error::ResetCard) {
                        app.message(
                            "The card was removed or reset since the connection: connect again.",
                        );
                    }
                    app.status = card::error_text(e);
                }
                Ok(None) => {
                    app.card_open = false;
                    app.card_info = None;
                    app.need_card();
                }
                Err(e) => app.status = e.to_string(),
            },
        );
    }

    /// Reader info: the reader's standard attributes (SCardGetAttrib) and its PC/SC part 10
    /// features, as a note in the exchanges. PC/SC reads them through a connection: the open
    /// one, or else a direct connection (no card needed) opened for the reading and closed
    /// after it, leaving the card as it is.
    pub(super) fn do_reader_info(&mut self) {
        let Some(name) = self.reader.and_then(|i| self.readers.get(i)).cloned() else {
            self.message("No reader selected.");
            return;
        };
        self.worker.call(
            move |d| -> Result<card::ReaderInfo, Option<reader::Error>> {
                if let Some(l) = d.link() {
                    return Ok(card::reader_info(l));
                }
                let system = d.system.as_mut().ok_or(None)?;
                let mut link = system
                    .connect(&name, Share::Direct, Proto::Any)
                    .map_err(Some)?;
                let mut info = card::reader_info(link.as_mut());
                info.opened_for_it = true;
                let _ = link.disconnect(Disp::Leave);
                Ok(info)
            },
            |app: &mut ApduStudio, r| match r {
                Ok(Ok(info)) => {
                    app.transcript.note(Level::Info, info.text());
                    app.status = "Reader info read.".into();
                }
                Ok(Err(Some(e))) => app.message(format!(
                    "Reader info: the reader can't be opened without a card (a direct connection): {}",
                    card::error_text(e)
                )),
                Ok(Err(None)) => app.message(NO_SERVICE),
                Err(e) => app.status = e.to_string(),
            },
        );
    }

    /// UID: sends PC/SC part 3's GET DATA (FF CA 00 00 00), which a contactless reader answers
    /// with the card's UID, and says what the answer means.
    pub(super) fn do_get_uid(&mut self) {
        if !self.need_card() {
            return;
        }
        let max = self.opts.response_buffer;
        self.log(&format!(
            "card    {}\n",
            scenario::hex_spaced(&card::GET_UID)
        ));
        self.worker.call(
            move |d| match d.link() {
                Some(l) => card::transmit(l, &card::GET_UID, max),
                None => Err(ExchangeError::NoCard),
            },
            |app: &mut ApduStudio, r| match r {
                Ok(Ok((resp, d))) => {
                    app.show_response(Channel::Card, card::GET_UID.to_vec(), resp.clone(), d);
                    app.transcript.note(Level::Info, card::uid_text(&resp));
                }
                Ok(Err(e)) => {
                    app.status = e.to_string();
                    app.transcript.exchange(
                        Channel::Card,
                        card::GET_UID.to_vec(),
                        Err(app.status.replace('\n', " ")),
                        None,
                        Verdict::Unchecked,
                    );
                }
                Err(e) => app.status = e.to_string(),
            },
        );
    }

    // ----------------------------------------------------------------- sending

    /// Records an answer: the exchange, the status line and the log.
    fn show_response(&mut self, channel: Channel, sent: Vec<u8>, resp: Vec<u8>, d: Duration) {
        self.status = format!("Answer received ({})", time_taken(d));
        let l = format!("answer  {}{}\n", scenario::hex_spaced(&resp), self.timed(d));
        self.transcript
            .exchange(channel, sent, Ok(resp), Some(d), Verdict::Unchecked);
        self.log(&l);
    }

    /// The commands sent for one when 61xx or 6Cxx were followed: each as an exchange (the
    /// verdict on the last), then a note with the whole answer, which the detail decodes.
    /// `error` is why the last one failed; `index` is the scenario command (usize::MAX: none).
    pub(super) fn show_legs(
        &mut self,
        legs: &[card::Leg],
        error: Option<String>,
        verdict: Verdict,
        whole: Option<&[u8]>,
        index: usize,
    ) {
        let last = legs.len().saturating_sub(1);
        for (k, leg) in legs.iter().enumerate() {
            let v = if k == last && error.is_none() {
                verdict.clone()
            } else {
                Verdict::Unchecked
            };
            self.transcript.exchange(
                Channel::Card,
                leg.sent.clone(),
                Ok(leg.reply.clone()),
                Some(leg.took),
                v,
            );
        }
        if let Some(e) = error {
            self.transcript.note(
                Level::Error,
                format!("The command sent after it failed: {e}"),
            );
        }
        if let Some(w) = whole {
            let whose = if index == usize::MAX {
                "The answer".to_string()
            } else {
                format!("Command {index}: the answer")
            };
            self.transcript.note_answer(
                Level::Info,
                format!(
                    "{whose}, put together from {} commands: {} data bytes, SW {}",
                    legs.len(),
                    w.len().saturating_sub(2),
                    sw_text(w)
                ),
                w.to_vec(),
            );
        }
    }

    /// Checks the response against the expected one of the list command it came from (`row`).
    /// A list command with no expected answer is judged by its status word, as in a run
    /// (`runner::Mark`); a command typed in the command line isn't judged.
    fn check_expected(&mut self, row: Option<usize>, expected: Option<String>, resp: &[u8]) {
        let Some(exp) = expected else {
            let Some(i) = row else {
                return;
            };
            let mark = match (iso::level_of(resp), iso::sw_problem(resp)) {
                (Some(iso::SwLevel::Error), Some(problem)) => {
                    self.log(&format!("failed  {problem}\n"));
                    self.transcript.set_last_verdict(Verdict::Failed(problem));
                    Mark::Failed
                }
                (Some(iso::SwLevel::Warning), Some(problem)) => {
                    self.log(&format!("warning {problem}\n"));
                    Mark::Warning
                }
                _ => Mark::Passed,
            };
            self.set_row_result(i, mark, sw_text(resp));
            return;
        };
        let l = format!("expect  {}\n", exp.trim());
        self.log(&l);
        match scenario::compare(&exp, resp) {
            None => {
                self.transcript.set_last_verdict(Verdict::Passed);
                if let Some(i) = row {
                    self.set_row_result(i, Mark::Passed, sw_text(resp));
                }
            }
            Some(err) => {
                let l = format!("failed  {err}\n");
                self.log(&l);
                self.status = format!("Not the expected answer ({}): {err}", exp.trim());
                self.transcript
                    .set_last_verdict(Verdict::Failed(format!("expected {}: {err}", exp.trim())));
                if let Some(i) = row {
                    self.set_row_result(i, Mark::Failed, sw_text(resp));
                }
            }
        }
    }

    pub(super) fn set_row_result(&mut self, i: usize, mark: Mark, short: String) {
        if let Some(slot) = self.row_results.get_mut(i) {
            *slot = Some(RowResult { mark, short });
        }
    }

    /// The scenario row whose command is in the command line unchanged, if any.
    fn list_row(&self) -> Option<usize> {
        self.current_list_command().map(|_| self.index)
    }

    pub(super) fn do_transmit(&mut self) {
        if !self.need_card() {
            return;
        }
        if self
            .current_list_command()
            .map(|c| c.incomplete)
            .unwrap_or(false)
        {
            self.message(format!("Not sent: {}.", crate::runner::PLACEHOLDER));
            return;
        }
        let text = self.command_text.clone();
        let Some(apdu) = self.check_hex(&text) else {
            return;
        };
        let expected = self.current_list_command().and_then(|c| c.expected.clone());
        let row = self.list_row();
        self.log(&format!("card    {}\n", text.trim()));
        let max = self.opts.response_buffer;
        let follow = self.settings.follow_up;
        let sent = apdu.clone();
        self.worker.call(
            move |d| match d.link() {
                Some(l) => card::transmit_following(l, &apdu, max, follow),
                None => card::Followed {
                    legs: Vec::new(),
                    answer: Err(ExchangeError::NoCard),
                },
            },
            move |app: &mut ApduStudio, r| {
                let e = match r {
                    Ok(f) if f.legs.len() > 1 => {
                        // Every command sent is shown; the whole answer is what's checked.
                        let whole = f.answer.as_ref().ok().map(|(a, _)| a.clone());
                        let err = f.answer.as_ref().err().map(ToString::to_string);
                        for (k, leg) in f.legs.iter().enumerate() {
                            let mut l = String::new();
                            if k > 0 {
                                l.push_str(&format!(
                                    "then    {}\n",
                                    scenario::hex_spaced(&leg.sent)
                                ));
                            }
                            l.push_str(&format!(
                                "answer  {}{}\n",
                                scenario::hex_spaced(&leg.reply),
                                app.timed(leg.took)
                            ));
                            app.log(&l);
                        }
                        app.show_legs(
                            &f.legs,
                            err.clone(),
                            Verdict::Unchecked,
                            whole.as_deref(),
                            usize::MAX,
                        );
                        match (f.answer, err) {
                            (Ok((whole, d)), _) => {
                                app.status = format!("Answer received ({})", time_taken(d));
                                let l = format!(
                                    "whole   {}{}\n",
                                    scenario::hex_spaced(&whole),
                                    app.timed(d)
                                );
                                app.log(&l);
                                app.check_expected(row, expected, &whole);
                                return;
                            }
                            (Err(_), Some(e)) => {
                                if let Some(i) = row {
                                    app.set_row_result(i, Mark::Failed, "error".into());
                                }
                                app.log(&format!("failed  {}\n", e.replace('\n', " ")));
                                app.status = e;
                                return;
                            }
                            (Err(e), None) => e.to_string(),
                        }
                    }
                    Ok(f) => match f.answer {
                        Ok((resp, d)) => {
                            app.show_response(Channel::Card, sent, resp.clone(), d);
                            app.check_expected(row, expected, &resp);
                            return;
                        }
                        Err(e) => e.to_string(),
                    },
                    Err(e) => e.to_string(),
                };
                let line = format!("failed  {}\n", e.replace('\n', " "));
                app.transcript.exchange(
                    Channel::Card,
                    sent,
                    Err(e.replace('\n', " ")),
                    None,
                    Verdict::Unchecked,
                );
                if let Some(i) = row {
                    app.set_row_result(i, Mark::Failed, "error".into());
                }
                app.status = e;
                app.log(&line);
            },
        );
    }

    /// Sends the command line and remembers it.
    pub(super) fn send_command_line(&mut self) {
        let text = self.command_text.trim().to_string();
        if !text.is_empty() && self.history.last() != Some(&text) {
            self.history.push(text);
            if self.history.len() > super::MAX_HISTORY {
                self.history.remove(0);
            }
        }
        self.history_pos = None;
        self.do_transmit();
    }

    /// Execute: sends the CCID escape field to the reader, and remembers it.
    pub(super) fn do_escape(&mut self) {
        let text = self.escape_text.trim().to_string();
        if !text.is_empty() && !self.escape_history.contains(&text) {
            self.escape_history.insert(0, text.clone());
        }
        self.send_escape(text, None);
    }

    /// Step on a script row that goes to the reader: sends it as a CCID escape command and
    /// checks the answer against its expected one, if any.
    pub(super) fn send_row_to_reader(&mut self, i: usize) {
        let Some(c) = self.scenario.commands.get(i) else {
            return;
        };
        if c.incomplete {
            self.message(format!("Not sent: {}.", crate::runner::PLACEHOLDER));
            return;
        }
        let text = c.text.trim().to_string();
        self.send_escape(text, Some(i));
    }

    /// Sends `text` to the reader as a CCID escape command (SCardControl). `row` is the script
    /// row it comes from, whose expected answer is checked.
    fn send_escape(&mut self, text: String, row: Option<usize>) {
        if !self.need_card() {
            return;
        }
        let Some(bytes) = self.check_hex(&text) else {
            return;
        };
        let expected = row
            .and_then(|i| self.scenario.commands.get(i))
            .and_then(|c| c.expected.clone());
        let max = self.opts.response_buffer;
        let sent = bytes.clone();
        self.worker.call(
            move |d| {
                let known = d.escape;
                let link = d.link()?;
                // Looked up once per connection: the reader's own code, or the system's.
                let code = known.unwrap_or_else(|| card::escape_code(link));
                let t = Instant::now();
                let r = link.control(code.code(), &bytes, max);
                let elapsed = t.elapsed();
                d.escape = Some(code);
                Some((r, elapsed, known.is_none().then_some(code)))
            },
            move |app: &mut ApduStudio, r| match r {
                Ok(Some((r, d, Some(code)))) => {
                    app.transcript.note(Level::Info, code.describe());
                    app.escaped(r, d, sent, text, row, expected);
                }
                Ok(Some((r, d, None))) => app.escaped(r, d, sent, text, row, expected),
                Ok(None) => {
                    app.card_open = false;
                    app.card_info = None;
                    app.need_card();
                }
                Err(e) => app.status = e.to_string(),
            },
        );
    }

    /// Records the reader's answer to a CCID escape command, and checks it when it comes from a
    /// script row (`row`) with an expected answer. A reader's answer has no status word: with
    /// nothing expected, any answer passes.
    fn escaped(
        &mut self,
        r: Result<Vec<u8>, reader::Error>,
        d: Duration,
        sent: Vec<u8>,
        text: String,
        row: Option<usize>,
        expected: Option<String>,
    ) {
        let l = format!("reader  {}\n", text.trim());
        self.log(&l);
        match r {
            Ok(resp) => {
                let mut l = format!("answer  {}{}\n", scenario::hex_spaced(&resp), self.timed(d));
                self.status = format!("Answer received ({})", time_taken(d));
                let mismatch = expected.as_ref().and_then(|e| {
                    l.push_str(&format!("expect  {}\n", e.trim()));
                    scenario::compare(e, &resp)
                });
                let verdict = match (&expected, &mismatch) {
                    (_, Some(why)) => {
                        l.push_str(&format!("failed  {why}\n"));
                        Verdict::Failed(format!(
                            "expected {}: {why}",
                            expected.as_deref().unwrap_or_default().trim()
                        ))
                    }
                    (Some(_), None) => Verdict::Passed,
                    (None, None) => Verdict::Unchecked,
                };
                if let Some(i) = row {
                    let mark = if mismatch.is_some() {
                        Mark::Failed
                    } else {
                        Mark::Passed
                    };
                    self.set_row_result(i, mark, format!("{} bytes", resp.len()));
                }
                self.transcript
                    .exchange(Channel::Escape, sent, Ok(resp), Some(d), verdict);
                self.log(&l);
            }
            Err(e) => {
                let name = self.reader_name();
                self.status = card::error_text(e);
                self.transcript.exchange(
                    Channel::Escape,
                    sent,
                    Err(self.status.replace('\n', " ")),
                    None,
                    Verdict::Unchecked,
                );
                if let Some(i) = row {
                    self.set_row_result(i, Mark::Failed, "error".into());
                }
                let l = format!("failed  {}\n", self.status);
                self.log(&l);
                self.message(format!(
                    "{name} refused the escape command {}: {}. {}",
                    text.trim(),
                    self.status,
                    card::ESCAPE_HINT
                ));
            }
        }
    }

    /// Watches the selected reader: asks the reader thread for its state every 250 ms while
    /// waiting for a card, else every 500 ms, and never while another job is out. A card put in
    /// or taken out is noted (`observed`). Returns true, while waiting, once a card was found.
    pub(super) fn watch_reader(&mut self) -> bool {
        if std::mem::take(&mut self.card_seen) {
            return true;
        }
        let every = if self.waiting.is_some() {
            WAIT_POLL
        } else {
            WATCH_POLL
        };
        let due = self
            .last_presence_check
            .is_none_or(|t| t.elapsed() >= every);
        let Some(name) = self.reader.and_then(|i| self.readers.get(i)).cloned() else {
            return false;
        };
        // Quiet: polling must not grey the controls each time. Only one poll is out at a time,
        // and none while an action of the person's is running.
        if due && self.service && !self.running && !self.worker.has_pending() {
            self.last_presence_check = Some(Instant::now());
            let reader = name.clone();
            self.worker.call_quiet(
                move |d| {
                    d.system
                        .as_mut()
                        .and_then(|s| s.reader_states(std::slice::from_ref(&reader)).ok())
                        .and_then(|st| st.into_iter().next())
                        .map(|r| (r.state.contains(State::PRESENT), r.atr))
                },
                move |app: &mut ApduStudio, r| {
                    if let Ok(Some((present, atr))) = r {
                        app.observed(&name, present, atr);
                    }
                },
            );
        }
        false
    }

    /// What a look at the reader found. A change since the last look in the same reader is
    /// noted: a card put in (with its ATR, decoded in the detail) or taken out; taking out the
    /// connected card loses the connection, and Reconnect is offered.
    pub(super) fn observed(&mut self, name: &str, present: bool, atr: Vec<u8>) {
        let before = self.seen.replace((name.to_string(), present));
        self.card_seen = present && self.waiting.is_some();
        let changed = before.is_some_and(|(r, p)| r == name && p != present);
        if !changed {
            return;
        }
        if present {
            let text = if atr.is_empty() {
                format!("Card inserted in {name}")
            } else {
                format!("Card inserted in {name}\nATR {}", hex(&atr))
            };
            let atrs = if atr.is_empty() {
                Vec::new()
            } else {
                vec![(name.to_string(), atr)]
            };
            self.transcript.note_atrs(Level::Info, text, atrs);
            if self.card_open && self.card_removed {
                self.status = format!("A card is in {name} again: Reconnect to use it.");
            }
        } else {
            self.transcript
                .note(Level::Info, format!("Card removed from {name}"));
            if self.card_open {
                self.card_removed = true;
                self.status =
                    "The card was removed: the connection is lost. Reconnect once a card is back."
                        .into();
            }
        }
    }

    /// Connects again, the same way, after the connected card was taken out and put back.
    pub(super) fn do_reconnect(&mut self) {
        self.card_removed = false;
        self.do_connect(self.share, self.proto);
    }

    /// Waiting for a card: connects once one is in the reader.
    pub(super) fn wait_for_card(&mut self) {
        let Some((share, proto)) = self.waiting else {
            return;
        };
        if self.watch_reader() {
            self.do_connect(share, proto);
        }
    }
}

/// The status word of an answer ("9000"), or "no SW" when it is shorter than two bytes.
pub(super) fn sw_text(resp: &[u8]) -> String {
    match resp {
        [.., a, b] => format!("{a:02X}{b:02X}"),
        _ => "no SW".into(),
    }
}
