// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! The session's exchanges: every command sent and its answer, and what happened around them
//! (connections, readings, warnings), in order. It is what the window shows; the log file is
//! written apart from it (`journal`).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Most entries kept; the oldest go first, so a long session can't fill the memory.
pub(super) const MAX_ENTRIES: usize = 5000;

/// Where a command went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Channel {
    /// SCardTransmit to the card.
    Card,
    /// A CCID escape (SCardControl) to the reader.
    Escape,
}

/// What the answer was checked against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    /// No expected answer.
    Unchecked,
    Passed,
    /// The answer differs from the expected one: why.
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Level {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Body {
    Exchange {
        channel: Channel,
        sent: Vec<u8>,
        /// The answer, or why there is none.
        reply: Result<Vec<u8>, String>,
        took: Option<Duration>,
        verdict: Verdict,
    },
    Note {
        level: Level,
        text: String,
        /// The ATRs the note shows, with whose each is (a reader's name), so the detail can
        /// decode them from their bytes rather than from the text.
        atrs: Vec<(String, Vec<u8>)>,
        /// An answer the note stands for (one put together from several commands after 61xx
        /// or 6Cxx), which the detail decodes.
        answer: Option<Vec<u8>>,
    },
}

impl Body {
    /// Whether the detail shows more than the list: every exchange, and a note with an ATR or
    /// an answer.
    pub(super) fn has_detail(&self) -> bool {
        match self {
            Body::Exchange { .. } => true,
            Body::Note { atrs, answer, .. } => !atrs.is_empty() || answer.is_some(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Entry {
    /// Unique in the session, never reused: selection follows it.
    pub(super) id: u64,
    /// Time since the session started.
    pub(super) at: Duration,
    pub(super) body: Body,
}

pub(super) struct Transcript {
    start: Instant,
    entries: VecDeque<Entry>,
    next_id: u64,
    /// The entry shown in detail.
    pub(super) selected: Option<u64>,
}

impl Transcript {
    pub(super) fn new() -> Self {
        Transcript {
            start: Instant::now(),
            entries: VecDeque::new(),
            next_id: 0,
            selected: None,
        }
    }

    fn push(&mut self, body: Body) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        if self.entries.len() >= MAX_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(Entry {
            id,
            at: self.start.elapsed(),
            body,
        });
        id
    }

    pub(super) fn note(&mut self, level: Level, text: impl Into<String>) {
        self.note_atrs(level, text, Vec::new());
    }

    /// A note that shows ATRs; the detail decodes them.
    pub(super) fn note_atrs(
        &mut self,
        level: Level,
        text: impl Into<String>,
        atrs: Vec<(String, Vec<u8>)>,
    ) {
        self.push(Body::Note {
            level,
            text: text.into(),
            atrs,
            answer: None,
        });
    }

    /// A note that stands for an answer put together from several commands; the detail
    /// decodes it.
    pub(super) fn note_answer(&mut self, level: Level, text: impl Into<String>, answer: Vec<u8>) {
        self.push(Body::Note {
            level,
            text: text.into(),
            atrs: Vec::new(),
            answer: Some(answer),
        });
    }

    pub(super) fn exchange(
        &mut self,
        channel: Channel,
        sent: Vec<u8>,
        reply: Result<Vec<u8>, String>,
        took: Option<Duration>,
        verdict: Verdict,
    ) {
        self.push(Body::Exchange {
            channel,
            sent,
            reply,
            took,
            verdict,
        });
    }

    /// Sets the verdict of the last exchange (its answer was checked after it arrived).
    pub(super) fn set_last_verdict(&mut self, v: Verdict) {
        let last = self
            .entries
            .iter_mut()
            .rev()
            .find(|e| matches!(e.body, Body::Exchange { .. }));
        if let Some(Entry {
            body: Body::Exchange { verdict, .. },
            ..
        }) = last
        {
            *verdict = v;
        }
    }

    pub(super) fn entries(&self) -> impl DoubleEndedIterator<Item = &Entry> {
        self.entries.iter()
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.selected = None;
    }

    /// The last exchange, if any.
    #[cfg(test)]
    pub(super) fn last_exchange(&self) -> Option<&Body> {
        self.entries
            .iter()
            .rev()
            .map(|e| &e.body)
            .find(|b| matches!(b, Body::Exchange { .. }))
    }

    /// The texts of the notes at `level`, oldest first.
    #[cfg(test)]
    pub(super) fn notes(&self, level: Level) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|e| match &e.body {
                Body::Note { level: l, text, .. } if *l == level => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// The whole transcript as plain text (the golden dump).
    #[cfg(test)]
    pub(super) fn to_text(&self) -> String {
        self.to_text_of(&Filter::default())
    }

    /// The entries `filter` lets through, as plain text.
    pub(super) fn to_text_of(&self, filter: &Filter) -> String {
        let mut out = String::new();
        for e in self.entries.iter().filter(|e| filter.matches(e)) {
            out.push_str(&entry_text(e, true));
            out.push('\n');
        }
        out
    }
}

/// Which entries the list shows: those holding a text (hex or words; spaces and case don't
/// count), and only problems if asked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Filter {
    pub(super) text: String,
    pub(super) problems_only: bool,
}

impl Filter {
    pub(super) fn is_active(&self) -> bool {
        self.problems_only || !squeeze(&self.text).is_empty()
    }

    pub(super) fn matches(&self, e: &Entry) -> bool {
        if self.problems_only && !is_problem(&e.body) {
            return false;
        }
        let q = squeeze(&self.text);
        q.is_empty() || squeeze(&entry_text(e, false)).contains(&q)
    }
}

/// Upper case without white space, so "6a 82" finds "6A82" and "00b2" finds "00 B2 01".
fn squeeze(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}

/// A failure or a warning: an exchange that failed, didn't give the expected answer, or ended
/// with a status word other than 9000 or 61xx; a note of a warning or an error.
pub(super) fn is_problem(b: &Body) -> bool {
    match b {
        Body::Note { level, .. } => *level != Level::Info,
        Body::Exchange { reply: Err(_), .. }
        | Body::Exchange {
            verdict: Verdict::Failed(_),
            ..
        } => true,
        Body::Exchange {
            channel: Channel::Card,
            reply: Ok(r),
            ..
        } => !matches!(r.as_slice(), [.., 0x90, 0x00] | [.., 0x61, _]),
        Body::Exchange { .. } => false,
    }
}

/// `m:ss.mmm` (or `h:mm:ss.mmm`) since the session started.
pub(super) fn clock(d: Duration) -> String {
    let ms = d.as_millis();
    let (h, m, s, ms) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{ms:03}")
    } else {
        format!("{m}:{s:02}.{ms:03}")
    }
}

/// Bytes as "00 A4 04 00".
pub(super) fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&format!("{b:02X}"));
    }
    s
}

pub(super) fn channel_label(c: Channel) -> &'static str {
    match c {
        Channel::Card => "card",
        Channel::Escape => "escape",
    }
}

/// One entry as text: the clock (optional), then the sent and received lines.
pub(super) fn entry_text(e: &Entry, with_clock: bool) -> String {
    let t = if with_clock {
        format!("{}  ", clock(e.at))
    } else {
        String::new()
    };
    match &e.body {
        Body::Note { level, text, .. } => {
            let mark = match level {
                Level::Info => "",
                Level::Warning => "! ",
                Level::Error => "!! ",
            };
            format!("{t}{mark}{text}")
        }
        Body::Exchange {
            channel,
            sent,
            reply,
            took,
            verdict,
        } => {
            let pad = " ".repeat(t.len());
            let ms = took
                .map(|d| format!("  {} ms", d.as_millis()))
                .unwrap_or_default();
            let got = match reply {
                Ok(r) => hex(r),
                Err(why) => format!("({why})"),
            };
            let check = match verdict {
                Verdict::Unchecked => String::new(),
                Verdict::Passed => "  OK".into(),
                Verdict::Failed(why) => format!("  FAIL: {}", why.replace('\n', " ")),
            };
            format!(
                "{t}> {}  [{}]\n{pad}< {got}{ms}{check}",
                hex(sent),
                channel_label(*channel)
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_reads_minutes_seconds_and_milliseconds() {
        assert_eq!(clock(Duration::from_millis(1234)), "0:01.234");
        assert_eq!(clock(Duration::from_millis(61_005)), "1:01.005");
        assert_eq!(clock(Duration::from_millis(3_723_004)), "1:02:03.004");
    }

    #[test]
    fn the_oldest_entries_go_first_and_ids_stay_unique() {
        let mut t = Transcript::new();
        for i in 0..MAX_ENTRIES + 3 {
            t.note(Level::Info, format!("{i}"));
        }
        assert_eq!(t.len(), MAX_ENTRIES);
        let first = t.entries().next().expect("entries");
        assert_eq!(first.id, 3);
        assert_eq!(t.notes(Level::Info).first().map(String::as_str), Some("3"));
    }

    #[test]
    fn a_verdict_lands_on_the_last_exchange() {
        let mut t = Transcript::new();
        t.exchange(
            Channel::Card,
            vec![0x00, 0xA4],
            Ok(vec![0x90, 0x00]),
            None,
            Verdict::Unchecked,
        );
        t.note(Level::Warning, "between");
        t.set_last_verdict(Verdict::Failed("Compare error!".into()));
        let text = t.to_text();
        assert!(text.contains("> 00 A4  [card]"), "{text}");
        assert!(text.contains("< 90 00  FAIL: Compare error!"), "{text}");
        assert!(text.contains("! between"), "{text}");
    }

    #[test]
    fn the_filter_finds_hex_and_words_and_keeps_problems() {
        let mut t = Transcript::new();
        let ok = |t: &mut Transcript, sent: &[u8], reply: &[u8]| {
            t.exchange(
                Channel::Card,
                sent.to_vec(),
                Ok(reply.to_vec()),
                None,
                Verdict::Unchecked,
            )
        };
        ok(&mut t, &[0x00, 0xB2, 0x01, 0x0C, 0x00], &[0x01, 0x90, 0x00]);
        ok(&mut t, &[0x00, 0xB2, 0x02, 0x0C, 0x00], &[0x6A, 0x83]);
        ok(&mut t, &[0x00, 0xA4, 0x04, 0x00], &[0x61, 0x10]);
        t.exchange(
            Channel::Escape,
            vec![0x01],
            Ok(vec![0x6A, 0x83]),
            None,
            Verdict::Unchecked,
        );
        t.note(Level::Info, "Card removed from Reader 0");
        t.note(Level::Warning, "Command 3 skipped");
        let shown = |f: &Filter| t.entries().filter(|e| f.matches(e)).count();
        let find = |text: &str| Filter {
            text: text.into(),
            problems_only: false,
        };
        assert!(!Filter::default().is_active());
        assert_eq!(shown(&Filter::default()), 6);
        assert!(find("  ").matches(t.entries().next().expect("entry")));
        assert!(!find("  ").is_active());
        assert_eq!(shown(&find("6a83")), 2, "card and escape answers");
        assert_eq!(shown(&find("00 b2")), 2);
        assert_eq!(shown(&find("b2010c")), 1);
        assert_eq!(shown(&find("card REMOVED")), 1);
        let problems = Filter {
            text: String::new(),
            problems_only: true,
        };
        // 6A83 from the card and the warning; 9000, 61xx, the escape's answer and the
        // information note are not problems.
        assert_eq!(shown(&problems), 2);
        let both = Filter {
            text: "b2".into(),
            problems_only: true,
        };
        assert_eq!(shown(&both), 1);
        let copied = t.to_text_of(&both);
        assert_eq!(copied.lines().count(), 2, "{copied}");
        assert!(copied.contains("6A 83"));
    }
}
