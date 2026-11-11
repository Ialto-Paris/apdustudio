// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! Access to smart card readers behind two traits, so the window can work with the real PC/SC
//! service or with a fake reader (tests, and later a simulation mode).
//!
//! Errors stay `pcsc::Error`: the texts and return codes shown to the person come from it
//! (see `card::error_text`), whichever implementation produced them.

use std::ffi::CString;
use std::time::Duration;

pub use pcsc::{Attribute, Error, Protocol, State, Status};

/// How the card is shared with other programs (Card Connect dialog).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Share {
    Shared,
    Exclusive,
    Direct,
}

/// Which protocol the card may use (Card Connect dialog).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Proto {
    T0,
    T1,
    Any,
}

/// What happens to the card on disconnection (Card Disconnect dialog).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Disp {
    PowerDown,
    Reset,
    Leave,
    Eject,
}

/// One reader as seen by Get Status.
#[derive(Clone, Debug, PartialEq)]
pub struct ReaderStatus {
    pub name: String,
    pub state: State,
    pub atr: Vec<u8>,
}

/// The connected card as seen by Card State.
#[derive(Clone, Debug, PartialEq)]
pub struct CardStatus {
    pub status: Status,
    pub protocol: Option<Protocol>,
    pub atr: Vec<u8>,
}

/// The most readers one SCardGetStatusChange call takes on Windows (MAXIMUM_SMARTCARD_READERS);
/// more fail with SCARD_E_INVALID_VALUE.
pub const STATUS_BATCH: usize = 10;

/// The smart card service: lists readers and opens connections.
pub trait CardSystem: Send {
    /// Reader names in the order the service gives them.
    fn readers(&mut self) -> Result<Vec<String>, Error>;
    /// The current state of at most `STATUS_BATCH` named readers, without waiting for a change.
    fn status_batch(&mut self, names: &[String]) -> Result<Vec<ReaderStatus>, Error>;
    /// The current state of each named reader, however many: asks `STATUS_BATCH` at a time.
    fn reader_states(&mut self, names: &[String]) -> Result<Vec<ReaderStatus>, Error> {
        let mut out = Vec::with_capacity(names.len());
        for batch in names.chunks(STATUS_BATCH) {
            out.extend(self.status_batch(batch)?);
        }
        Ok(out)
    }
    /// Connects to the card in `reader` (or to the reader itself with `Share::Direct`).
    fn connect(
        &mut self,
        reader: &str,
        share: Share,
        proto: Proto,
    ) -> Result<Box<dyn CardLink>, Error>;
}

/// An open connection to a card (or to a reader, in direct mode).
pub trait CardLink: Send {
    fn status(&self) -> Result<CardStatus, Error>;
    /// Sends an APDU (SCardTransmit); the answer is at most `max_response` bytes.
    fn transmit(&mut self, apdu: &[u8], max_response: usize) -> Result<Vec<u8>, Error>;
    /// Sends a control code to the reader (SCardControl).
    fn control(&mut self, code: u32, data: &[u8], max_response: usize) -> Result<Vec<u8>, Error>;
    /// Reads one of the reader's attributes (SCardGetAttrib), as the driver gives it.
    fn attribute(&self, attr: Attribute) -> Result<Vec<u8>, Error>;
    /// Ends the connection. On failure the connection is handed back with the error.
    fn disconnect(self: Box<Self>, disp: Disp) -> Result<(), (Box<dyn CardLink>, Error)>;
}

/// Opens the smart card service; called at start and again when the service was not running.
pub type SystemOpener = Box<dyn Fn() -> Result<Box<dyn CardSystem>, Error> + Send>;

// ------------------------------------------------------------------------------- PC/SC

/// The Windows smart card service (WinSCard) or pcsc-lite, through the `pcsc` crate.
pub struct PcscSystem {
    ctx: pcsc::Context,
}

impl PcscSystem {
    pub fn establish() -> Result<Self, Error> {
        Ok(PcscSystem {
            ctx: pcsc::Context::establish(pcsc::Scope::User)?,
        })
    }

    /// The opener the app uses outside tests.
    pub fn opener() -> SystemOpener {
        Box::new(|| Ok(Box::new(PcscSystem::establish()?) as Box<dyn CardSystem>))
    }
}

fn c_name(name: &str) -> Result<CString, Error> {
    CString::new(name).map_err(|_| Error::UnknownReader)
}

impl CardSystem for PcscSystem {
    fn readers(&mut self) -> Result<Vec<String>, Error> {
        Ok(self
            .ctx
            .list_readers_owned()?
            .into_iter()
            .map(|c| c.to_string_lossy().into_owned())
            .collect())
    }

    fn status_batch(&mut self, names: &[String]) -> Result<Vec<ReaderStatus>, Error> {
        let mut states = names
            .iter()
            .map(|n| Ok(pcsc::ReaderState::new(c_name(n)?, State::UNAWARE)))
            .collect::<Result<Vec<_>, Error>>()?;
        self.ctx.get_status_change(Duration::ZERO, &mut states)?;
        Ok(states
            .iter()
            .map(|s| ReaderStatus {
                name: s.name().to_string_lossy().into_owned(),
                state: s.event_state(),
                atr: s.atr().to_vec(),
            })
            .collect())
    }

    fn connect(
        &mut self,
        reader: &str,
        share: Share,
        proto: Proto,
    ) -> Result<Box<dyn CardLink>, Error> {
        let (mode, protos) = match share {
            Share::Shared => (pcsc::ShareMode::Shared, None),
            Share::Exclusive => (pcsc::ShareMode::Exclusive, None),
            Share::Direct => (pcsc::ShareMode::Direct, Some(pcsc::Protocols::UNDEFINED)),
        };
        let protos = protos.unwrap_or(match proto {
            Proto::T0 => pcsc::Protocols::T0,
            Proto::T1 => pcsc::Protocols::T1,
            Proto::Any => pcsc::Protocols::ANY,
        });
        let card = self.ctx.connect(&c_name(reader)?, mode, protos)?;
        Ok(Box::new(PcscLink { card }))
    }
}

struct PcscLink {
    card: pcsc::Card,
}

impl CardLink for PcscLink {
    fn status(&self) -> Result<CardStatus, Error> {
        let s = self.card.status2_owned()?;
        Ok(CardStatus {
            status: s.status(),
            protocol: s.protocol2(),
            atr: s.atr().to_vec(),
        })
    }

    fn transmit(&mut self, apdu: &[u8], max_response: usize) -> Result<Vec<u8>, Error> {
        let mut buf = vec![0u8; max_response];
        Ok(self.card.transmit(apdu, &mut buf)?.to_vec())
    }

    fn control(&mut self, code: u32, data: &[u8], max_response: usize) -> Result<Vec<u8>, Error> {
        let mut buf = vec![0u8; max_response];
        // DWORD is u32 on Windows and macOS, and unsigned long (u64) on 64-bit Linux.
        #[allow(clippy::useless_conversion)]
        let code = pcsc::ffi::DWORD::from(code);
        Ok(self.card.control(code, data, &mut buf)?.to_vec())
    }

    fn attribute(&self, attr: Attribute) -> Result<Vec<u8>, Error> {
        self.card.get_attribute_owned(attr)
    }

    fn disconnect(self: Box<Self>, disp: Disp) -> Result<(), (Box<dyn CardLink>, Error)> {
        let d = match disp {
            Disp::PowerDown => pcsc::Disposition::UnpowerCard,
            Disp::Reset => pcsc::Disposition::ResetCard,
            Disp::Leave => pcsc::Disposition::LeaveCard,
            Disp::Eject => pcsc::Disposition::EjectCard,
        };
        self.card
            .disconnect(d)
            .map_err(|(card, e)| (Box::new(PcscLink { card }) as Box<dyn CardLink>, e))
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Talks to the real smart card service; run with `cargo test -- --ignored`.
    /// Without a reader the service answers "no readers" (or isn't running at all).
    #[test]
    #[ignore]
    fn the_real_service_answers() {
        match PcscSystem::establish() {
            Err(e) => assert!(
                matches!(e, Error::NoService | Error::ServiceStopped),
                "unexpected: {e:?}"
            ),
            Ok(mut s) => match s.readers() {
                Ok(names) => {
                    let states = s.reader_states(&names).expect("reader states");
                    assert_eq!(states.len(), names.len());
                    eprintln!("readers: {states:?}");
                }
                Err(e) => assert_eq!(e, Error::NoReadersAvailable),
            },
        }
    }
}

// -------------------------------------------------------------------------------- fake

/// A scripted reader for tests (and, later, a simulation mode): no hardware needed.
///
/// `FakeSystem` is a cheap handle to shared state, so a test keeps one copy to script answers
/// and read back what the app sent, while the app owns the others.
#[cfg_attr(not(test), allow(dead_code))]
pub mod fake {
    use super::*;
    use std::sync::{Arc, Mutex, PoisonError};

    /// A card sitting in a fake reader.
    #[derive(Clone, Debug)]
    pub struct FakeCard {
        pub atr: Vec<u8>,
        pub protocol: Protocol,
    }

    #[derive(Clone, Debug)]
    pub struct FakeReader {
        pub name: String,
        pub card: Option<FakeCard>,
    }

    /// Something the app sent, in order.
    #[derive(Clone, Debug, PartialEq)]
    pub enum Sent {
        Connect {
            reader: String,
            share: Share,
            proto: Proto,
        },
        Transmit(Vec<u8>),
        Control(u32, Vec<u8>),
        Attribute(Attribute),
        Disconnect(Disp),
    }

    #[derive(Default)]
    pub struct FakeState {
        pub readers: Vec<FakeReader>,
        /// Error returned when the app opens the service (e.g. `Error::NoService`).
        pub open_error: Option<Error>,
        /// Answers to APDUs, matched on the exact bytes sent.
        pub answers: Vec<(Vec<u8>, Vec<u8>)>,
        /// Answers to control codes, matched on code and data.
        pub control_answers: Vec<(u32, Vec<u8>, Vec<u8>)>,
        /// The reader's attributes; any other is "not supported", as many drivers answer.
        pub attributes: Vec<(Attribute, Vec<u8>)>,
        /// Answer to an APDU that has no script ("instruction not supported").
        pub default_answer: Vec<u8>,
        /// How long each APDU takes to answer (a slow card).
        pub delay: Duration,
        pub sent: Vec<Sent>,
    }

    #[derive(Clone, Default)]
    pub struct FakeSystem {
        pub state: Arc<Mutex<FakeState>>,
    }

    impl FakeSystem {
        /// One reader with a T=1 card in it.
        pub fn with_card(reader: &str, atr: &[u8]) -> Self {
            let f = FakeSystem::default();
            {
                let mut s = f.state.lock().unwrap_or_else(PoisonError::into_inner);
                s.default_answer = vec![0x6D, 0x00];
                s.readers.push(FakeReader {
                    name: reader.to_string(),
                    card: Some(FakeCard {
                        atr: atr.to_vec(),
                        protocol: Protocol::T1,
                    }),
                });
            }
            f
        }

        /// Answers `response` whenever `apdu` is sent.
        pub fn answer(&self, apdu: &[u8], response: &[u8]) -> &Self {
            self.state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .answers
                .push((apdu.to_vec(), response.to_vec()));
            self
        }

        pub fn answer_control(&self, code: u32, data: &[u8], response: &[u8]) -> &Self {
            self.state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .control_answers
                .push((code, data.to_vec(), response.to_vec()));
            self
        }

        /// Gives the reader the attribute `attr` with this value.
        pub fn attribute(&self, attr: Attribute, value: &[u8]) -> &Self {
            self.state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .attributes
                .push((attr, value.to_vec()));
            self
        }

        /// Takes the card out of (None) or puts one into the first reader.
        pub fn set_card(&self, card: Option<FakeCard>) {
            if let Some(r) = self
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .readers
                .first_mut()
            {
                r.card = card;
            }
        }

        pub fn sent(&self) -> Vec<Sent> {
            self.state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .sent
                .clone()
        }

        /// The opener to hand to the app.
        pub fn opener(&self) -> SystemOpener {
            let me = self.clone();
            Box::new(move || {
                if let Some(e) = me
                    .state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .open_error
                {
                    return Err(e);
                }
                Ok(Box::new(me.clone()) as Box<dyn CardSystem>)
            })
        }

        fn card_in(&self, reader: &str) -> Result<Option<FakeCard>, Error> {
            self.state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .readers
                .iter()
                .find(|r| r.name == reader)
                .map(|r| r.card.clone())
                .ok_or(Error::UnknownReader)
        }
    }

    impl CardSystem for FakeSystem {
        fn readers(&mut self) -> Result<Vec<String>, Error> {
            let names: Vec<String> = self
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .readers
                .iter()
                .map(|r| r.name.clone())
                .collect();
            if names.is_empty() {
                Err(Error::NoReadersAvailable)
            } else {
                Ok(names)
            }
        }

        fn status_batch(&mut self, names: &[String]) -> Result<Vec<ReaderStatus>, Error> {
            // Like Windows: more than STATUS_BATCH readers in one call is refused.
            if names.len() > STATUS_BATCH {
                return Err(Error::InvalidValue);
            }
            names
                .iter()
                .map(|n| {
                    let card = self.card_in(n)?;
                    Ok(ReaderStatus {
                        name: n.clone(),
                        state: if card.is_some() {
                            State::PRESENT
                        } else {
                            State::EMPTY
                        },
                        atr: card.map(|c| c.atr).unwrap_or_default(),
                    })
                })
                .collect()
        }

        fn connect(
            &mut self,
            reader: &str,
            share: Share,
            proto: Proto,
        ) -> Result<Box<dyn CardLink>, Error> {
            let card = self.card_in(reader)?;
            self.state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .sent
                .push(Sent::Connect {
                    reader: reader.to_string(),
                    share,
                    proto,
                });
            match (card, share) {
                (Some(card), _) => Ok(Box::new(FakeLink {
                    system: self.clone(),
                    card: Some(card),
                })),
                (None, Share::Direct) => Ok(Box::new(FakeLink {
                    system: self.clone(),
                    card: None,
                })),
                (None, _) => Err(Error::NoSmartcard),
            }
        }
    }

    struct FakeLink {
        system: FakeSystem,
        card: Option<FakeCard>,
    }

    impl FakeLink {
        fn fits(resp: Vec<u8>, max_response: usize) -> Result<Vec<u8>, Error> {
            if resp.len() > max_response {
                Err(Error::InsufficientBuffer)
            } else {
                Ok(resp)
            }
        }
    }

    impl CardLink for FakeLink {
        fn status(&self) -> Result<CardStatus, Error> {
            match &self.card {
                Some(c) => Ok(CardStatus {
                    status: Status::PRESENT | Status::POWERED | Status::SPECIFIC,
                    protocol: Some(c.protocol),
                    atr: c.atr.clone(),
                }),
                None => Ok(CardStatus {
                    status: Status::ABSENT,
                    protocol: None,
                    atr: Vec::new(),
                }),
            }
        }

        fn transmit(&mut self, apdu: &[u8], max_response: usize) -> Result<Vec<u8>, Error> {
            if self.card.is_none() {
                return Err(Error::NoSmartcard);
            }
            let delay = self
                .system
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .delay;
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
            let mut s = self
                .system
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            s.sent.push(Sent::Transmit(apdu.to_vec()));
            let resp = s
                .answers
                .iter()
                .find(|(a, _)| a == apdu)
                .map(|(_, r)| r.clone())
                .unwrap_or_else(|| s.default_answer.clone());
            Self::fits(resp, max_response)
        }

        fn control(
            &mut self,
            code: u32,
            data: &[u8],
            max_response: usize,
        ) -> Result<Vec<u8>, Error> {
            let mut s = self
                .system
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            s.sent.push(Sent::Control(code, data.to_vec()));
            let resp = s
                .control_answers
                .iter()
                .find(|(c, d, _)| *c == code && d == data)
                .map(|(_, _, r)| r.clone())
                .ok_or(Error::InvalidParameter)?;
            Self::fits(resp, max_response)
        }

        fn attribute(&self, attr: Attribute) -> Result<Vec<u8>, Error> {
            let mut s = self
                .system
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            s.sent.push(Sent::Attribute(attr));
            s.attributes
                .iter()
                .find(|(a, _)| *a == attr)
                .map(|(_, v)| v.clone())
                .ok_or(Error::UnsupportedFeature)
        }

        fn disconnect(self: Box<Self>, disp: Disp) -> Result<(), (Box<dyn CardLink>, Error)> {
            self.system
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .sent
                .push(Sent::Disconnect(disp));
            Ok(())
        }
    }
}
