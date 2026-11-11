// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! Running a script: plays its commands, one per step, to the card (SCardTransmit) or the
//! reader (a CCID escape command), and keeps the counts and the report. The report is written
//! to the log as it grows, so a run's log reads like its report. Nothing here knows about the
//! window: it gets the card link, writes through a callback and says what happened.
//!
//! ```text
//! Run of demo.apdu, 1 iteration
//!
//! Iteration 1 of 1
//! [2] Read record 1, SFI 1
//!     card    00 B2 01 0C 00
//!     answer  6A 83  (3 ms)
//!     expect  90 00
//!     failed  byte 1 is 6A, expected 90
//!
//! Result: 6 commands, 4 passed, 1 warning, 1 failed
//! Time: 164 ms in all, 164 ms per iteration
//! ```

use std::time::{Duration, Instant};

use crate::card;
use crate::iso;
use crate::reader::CardLink;
use crate::scenario::{self, Command, HexError, Target};

/// A duration as reports show it: "12 ms", or "1.234 s" from a second on.
pub fn time_taken(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        format!("{ms} ms")
    } else {
        format!("{}.{:03} s", ms / 1000, ms % 1000)
    }
}

/// Why a run stopped before its end, when the person stopped it.
pub const STOPPED_BY_USER: &str = "Stopped: Stop or Esc was pressed.";
const STOPPED_BY_VERIFY: &str =
    "Stopped: a VERIFY failed. It isn't sent again, so the PIN can't be blocked by the run.";
/// Why a command holding a placeholder isn't sent.
pub const PLACEHOLDER: &str = "it holds a placeholder (<...>): fill it in first";

/// What the run needs from the program's settings.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub response_buffer: usize,
    /// `--no-times`: no durations in the report and the log.
    pub no_time: bool,
    /// Send GET RESPONSE after 61xx and the command again after 6Cxx (`card::transmit_following`).
    pub follow_up: bool,
}

/// Whether the card in the reader is the one the script was written for.
#[derive(Debug, PartialEq, Eq)]
pub enum AtrCheck {
    Matches,
    /// The card's ATR could not be read.
    Unreadable,
    Differs,
}

/// Compares the card's ATR (from SCardStatus) with the one the script expects (`atr:`, with
/// `?` and a final `*` as in an expected answer).
pub fn check_atr(expected: &str, atr: Option<Vec<u8>>) -> AtrCheck {
    let Some(a) = atr else {
        return AtrCheck::Unreadable;
    };
    if scenario::compare(expected.trim(), &a).is_some() {
        AtrCheck::Differs
    } else {
        AtrCheck::Matches
    }
}

/// What happened to one command.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Not sent: it holds a placeholder, or isn't hex. The text says why.
    Skipped(String),
    /// Sent to the card (SCardTransmit) or the reader (CCID escape).
    Sent {
        target: Target,
        apdu: Vec<u8>,
        /// The answer and how long it took, or why there is none.
        reply: Result<(Vec<u8>, Duration), String>,
        /// Why the answer isn't the expected one; with none expected, why it isn't a success
        /// (an error status word, `iso::sw_problem`).
        mismatch: Option<String>,
        /// Every command sent, when GET RESPONSE or a new Le followed the first (else empty):
        /// `reply` is then the whole answer put together.
        legs: Vec<card::Leg>,
    },
}

/// How a command counts in a run.
///
/// With an expected answer, the comparison decides: passed or failed. Without one, a card's
/// status word does, by its ISO/IEC 7816-4 category (`iso::sw_level`): normal processing passes,
/// an execution or checking error (64xx to 6Fxx) fails, and a warning (62xx, 63xx) or a status
/// word outside ISO/IEC 7816-4 is a warning, which doesn't fail the run. A reader's answer to an
/// escape command has no status word: with nothing expected, an answer passes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Passed,
    Warning,
    Failed,
}

/// One command run: which one, what happened, and how it counts.
#[derive(Clone, Debug, PartialEq)]
pub struct Done {
    pub index: usize,
    pub outcome: Outcome,
    pub mark: Mark,
}

/// The end of a run.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub report: String,
    /// The end of the report: why it stopped (if it did), the totals and the time.
    pub closing: String,
    pub passed: u32,
    /// Commands with a warning status word and no expected answer: neither passed nor failed.
    pub warnings: u32,
    pub failed: u32,
    /// An answer differed from the expected one.
    pub compare_errors: bool,
}

/// What one step did.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// A command was run; `status` describes the progress.
    Command { done: Done, status: String },
    /// An iteration ended and another one starts.
    Iteration { status: String },
    /// The run is over (last iteration done, or a failed VERIFY: then `last` is that command).
    Finished {
        status: String,
        summary: Summary,
        last: Option<Done>,
    },
}

pub struct Run {
    iterations: u32,
    iteration: u32,
    index: usize,
    passed: u32,
    warnings: u32,
    failed: u32,
    executed: u32,
    compare_errors: bool,
    /// The reader's CCID escape code, looked up at the first reader command.
    escape: Option<card::EscapeCode>,
    iter_start: Instant,
    all_start: Instant,
    report: String,
}

/// "1 iteration", "3 iterations".
fn count(n: u32, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}

/// A line of a command's block in the report: "    answer  90 00".
fn row(label: &str, text: &str) -> String {
    format!("    {label:<7} {text}\n")
}

impl Run {
    /// Starts a run of `iterations` (at least 1) over the script in `file`. Returns the run and
    /// the report's first lines, to write in the log.
    pub fn start(file: &str, iterations: u32) -> (Run, String) {
        let now = Instant::now();
        let iterations = iterations.max(1);
        let head = format!(
            "Run of {file}, {}\n\nIteration 1 of {iterations}\n",
            count(iterations, "iteration")
        );
        let run = Run {
            iterations,
            iteration: 1,
            index: 0,
            passed: 0,
            warnings: 0,
            failed: 0,
            executed: 0,
            compare_errors: false,
            escape: None,
            iter_start: now,
            all_start: now,
            report: head.clone(),
        };
        (run, head)
    }

    /// The report so far.
    pub fn report(&self) -> &str {
        &self.report
    }

    /// The index of the command the next step will run.
    pub fn next_index(&self) -> usize {
        self.index
    }

    /// Adds `text` to the report and writes it to the log.
    fn emit(&mut self, text: &str, log: &mut dyn FnMut(&str)) {
        self.report.push_str(text);
        log(text);
    }

    /// Runs the next command (or closes the iteration when all were run).
    /// `link` is None when no card is connected: the command fails like an unanswered one.
    pub fn step(
        &mut self,
        commands: &[Command],
        link: Option<&mut dyn CardLink>,
        settings: Settings,
        log: &mut dyn FnMut(&str),
    ) -> Step {
        let idx = self.index;
        let Some(cmd) = commands.get(idx) else {
            return self.end_iteration(settings, log);
        };
        let mut block = match cmd.description.trim() {
            "" => format!("[{}]\n", idx + 1),
            d => format!("[{}] {d}\n", idx + 1),
        };
        block.push_str(&row(cmd.target.keyword(), cmd.text.trim()));
        let timed = |d: Duration| {
            if settings.no_time {
                String::new()
            } else {
                format!("  ({})", time_taken(d))
            }
        };
        let mut ok = true;
        let mut warning = false;
        let mut stop = None;
        let mut compare_error = false;
        let outcome = match (cmd.incomplete, scenario::parse_hex(&cmd.text)) {
            (true, _) => {
                block.push_str(&row("skipped", PLACEHOLDER));
                ok = false;
                Outcome::Skipped(PLACEHOLDER.into())
            }
            (false, Err(e)) => {
                let why = match e {
                    HexError::NotHex => "it isn't hexadecimal",
                    HexError::Incomplete => "it has an odd number of hex digits",
                };
                block.push_str(&row("skipped", why));
                ok = false;
                Outcome::Skipped(why.into())
            }
            (false, Ok(apdu)) => {
                let (res, legs) = match (link, cmd.target) {
                    (None, _) => (Err(card::ExchangeError::NoCard), Vec::new()),
                    (Some(link), Target::Card) => {
                        let f = card::transmit_following(
                            link,
                            &apdu,
                            settings.response_buffer,
                            settings.follow_up,
                        );
                        (f.answer, f.legs)
                    }
                    (Some(link), Target::Reader) => {
                        let code = *self.escape.get_or_insert_with(|| card::escape_code(link));
                        let t = Instant::now();
                        let r = link
                            .control(code.code(), &apdu, settings.response_buffer)
                            .map(|r| (r, t.elapsed()))
                            .map_err(card::ExchangeError::Reader);
                        (r, Vec::new())
                    }
                };
                // The commands GET RESPONSE or a new Le added, each with its answer.
                let legs = if legs.len() > 1 { legs } else { Vec::new() };
                for (k, leg) in legs.iter().enumerate() {
                    if k > 0 {
                        block.push_str(&row("then", &scenario::hex_spaced(&leg.sent)));
                    }
                    block.push_str(&row(
                        "answer",
                        &format!("{}{}", scenario::hex_spaced(&leg.reply), timed(leg.took)),
                    ));
                }
                let mut mismatch = None;
                let reply = match res {
                    Ok((resp, d)) => {
                        let label = if legs.is_empty() { "answer" } else { "whole" };
                        block.push_str(&row(
                            label,
                            &format!("{}{}", scenario::hex_spaced(&resp), timed(d)),
                        ));
                        if let Some(exp) = &cmd.expected {
                            block.push_str(&row("expect", exp.trim()));
                            if let Some(err) = scenario::compare(exp, &resp) {
                                ok = false;
                                compare_error = true;
                                block.push_str(&row("failed", &err));
                                mismatch = Some(err);
                            }
                        } else if cmd.target == Target::Card {
                            // Nothing was expected, so the command should have succeeded.
                            if let Some(problem) = iso::sw_problem(&resp) {
                                if iso::level_of(&resp) == Some(iso::SwLevel::Error) {
                                    ok = false;
                                    block.push_str(&row("failed", &problem));
                                    mismatch = Some(problem);
                                } else {
                                    warning = true;
                                    block.push_str(&row("warning", &problem));
                                }
                            }
                        }
                        // A failed VERIFY (ISO 7816-4: interindustry class, INS 20) ends the
                        // run, so a wrong PIN isn't tried again and the card isn't blocked.
                        // Failed means: not the expected answer when one is given (a test of a
                        // wrong PIN expects 63Cx), else not 9000.
                        let verify = cmd.target == Target::Card
                            && apdu.first().is_some_and(|cla| cla & 0x80 == 0)
                            && apdu.get(1) == Some(&0x20);
                        let failed = if cmd.expected.is_some() {
                            mismatch.is_some()
                        } else {
                            !resp.ends_with(&[0x90, 0x00])
                        };
                        if verify && failed {
                            stop = Some(STOPPED_BY_VERIFY);
                        }
                        Ok((resp, d))
                    }
                    Err(e) => {
                        ok = false;
                        let e = e.to_string().replace('\n', " ");
                        block.push_str(&row("failed", &e));
                        Err(e)
                    }
                };
                Outcome::Sent {
                    target: cmd.target,
                    apdu,
                    reply,
                    mismatch,
                    legs,
                }
            }
        };
        self.emit(&block, log);
        self.executed += 1;
        if compare_error {
            self.compare_errors = true;
        }
        let mark = if !ok {
            self.failed += 1;
            Mark::Failed
        } else if warning {
            self.warnings += 1;
            Mark::Warning
        } else {
            self.passed += 1;
            Mark::Passed
        };
        self.index += 1;
        let done = Done {
            index: idx,
            outcome,
            mark,
        };
        if let Some(msg) = stop {
            let summary = self.summary(msg, settings, log);
            return Step::Finished {
                status: Self::finished_status(&summary),
                summary,
                last: Some(done),
            };
        }
        Step::Command {
            done,
            status: format!(
                "Run: iteration {} of {}, command {} of {}",
                self.iteration,
                self.iterations,
                idx + 1,
                commands.len()
            ),
        }
    }

    /// Ends the run now, saying why in the report (`STOPPED_BY_USER`).
    pub fn stop(&mut self, why: &str, settings: Settings, log: &mut dyn FnMut(&str)) -> Summary {
        self.summary(why, settings, log)
    }

    fn end_iteration(&mut self, settings: Settings, log: &mut dyn FnMut(&str)) -> Step {
        let took = time_taken(self.iter_start.elapsed());
        if !settings.no_time {
            let line = format!("Iteration {}: {took}\n", self.iteration);
            self.emit(&line, log);
        }
        let status = format!(
            "Iteration {} of {} done in {took}",
            self.iteration, self.iterations
        );
        if self.iteration >= self.iterations {
            let summary = self.summary("", settings, log);
            return Step::Finished {
                status: Self::finished_status(&summary),
                summary,
                last: None,
            };
        }
        self.iteration += 1;
        self.index = 0;
        self.iter_start = Instant::now();
        let head = format!("\nIteration {} of {}\n", self.iteration, self.iterations);
        self.emit(&head, log);
        Step::Iteration { status }
    }

    /// "4 passed, 1 warning, 1 failed" (the warnings only when there are some).
    fn counts(passed: u32, warnings: u32, failed: u32) -> String {
        let warnings = match warnings {
            0 => String::new(),
            n => format!(", {}", count(n, "warning")),
        };
        format!("{passed} passed{warnings}, {failed} failed")
    }

    pub fn finished_status(s: &Summary) -> String {
        format!(
            "Run finished: {}.",
            Self::counts(s.passed, s.warnings, s.failed)
        )
    }

    fn summary(&mut self, why: &str, settings: Settings, log: &mut dyn FnMut(&str)) -> Summary {
        let total = self.all_start.elapsed();
        let done_iters = self.iteration.min(self.iterations).max(1);
        let mut closing = String::from("\n");
        if !why.is_empty() {
            closing.push_str(why);
            closing.push('\n');
        }
        closing.push_str(&format!(
            "Result: {}, {}\n",
            count(self.executed, "command"),
            Self::counts(self.passed, self.warnings, self.failed)
        ));
        if !settings.no_time {
            closing.push_str(&format!(
                "Time: {} in all, {} per iteration\n",
                time_taken(total),
                time_taken(total / done_iters)
            ));
        }
        self.emit(&closing, log);
        Summary {
            report: self.report.clone(),
            closing,
            passed: self.passed,
            warnings: self.warnings,
            failed: self.failed,
            compare_errors: self.compare_errors,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::fake::{FakeSystem, Sent};
    use crate::reader::{CardSystem, Proto, Share};

    const READER: &str = "Test Reader 0";
    const ATR: [u8; 4] = [0x3B, 0x02, 0x14, 0x50];
    const SETTINGS: Settings = Settings {
        response_buffer: 65536,
        no_time: true,
        follow_up: false,
    };
    const CHALLENGE: [u8; 5] = [0x00, 0x84, 0x00, 0x00, 0x08];
    const RANDOM: [u8; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 0x90, 0x00];

    fn link(f: &FakeSystem) -> Box<dyn CardLink> {
        f.clone()
            .connect(READER, Share::Shared, Proto::Any)
            .expect("fake connect")
    }

    fn script(body: &str) -> scenario::Scenario {
        scenario::parse(&format!("{} 1\n{body}", scenario::HEADER)).expect("valid script")
    }

    /// Runs the whole script; returns the steps, the log and the summary.
    fn run_all(
        f: &FakeSystem,
        body: &str,
        iterations: u32,
        settings: Settings,
    ) -> (Vec<Step>, String, Summary) {
        let sc = script(body);
        let mut l = link(f);
        let (mut run, first) = Run::start("t.apdu", iterations);
        let mut log = first;
        let mut steps = Vec::new();
        for _ in 0..1000 {
            let step = run.step(&sc.commands, Some(l.as_mut()), settings, &mut |s| {
                log.push_str(s)
            });
            if let Step::Finished { summary, .. } = &step {
                let summary = summary.clone();
                steps.push(step);
                return (steps, log, summary);
            }
            steps.push(step);
        }
        panic!("the run never finished");
    }

    fn transmits(f: &FakeSystem) -> Vec<Vec<u8>> {
        f.sent()
            .into_iter()
            .filter_map(|s| match s {
                Sent::Transmit(a) => Some(a),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn each_iteration_runs_every_command() {
        let f = FakeSystem::with_card(READER, &ATR);
        f.answer(&[0x00, 0xA4, 0x00, 0x00, 0x02, 0x7F, 0x10], &[0x90, 0x00]);
        let (steps, _, s) = run_all(
            &f,
            "step: select\ncard: 00A40000027F10\nexpect: 9000\nstep: other\ncard: 00A40000027F10\n",
            3,
            SETTINGS,
        );
        assert_eq!((s.passed, s.failed), (6, 0));
        assert_eq!(transmits(&f).len(), 6);
        let iterations = steps
            .iter()
            .filter(|s| matches!(s, Step::Iteration { .. }))
            .count();
        assert_eq!(iterations, 2);
        assert!(s.report.starts_with("Run of t.apdu, 3 iterations\n"));
        assert!(s.report.contains("\nIteration 3 of 3\n"));
        assert!(s
            .report
            .contains("Result: 6 commands, 6 passed, 0 failed\n"));
        assert!(!s.compare_errors);
    }

    #[test]
    fn the_status_counts_commands_and_iterations() {
        let f = FakeSystem::with_card(READER, &ATR);
        f.answer(&CHALLENGE, &RANDOM);
        let (steps, _, _) = run_all(&f, "card: 0084000008\ncard: 0084000008\n", 2, SETTINGS);
        match steps.first() {
            Some(Step::Command { done, status }) => {
                assert_eq!(done.index, 0);
                assert_eq!(done.mark, Mark::Passed, "{done:?}");
                assert_eq!(status, "Run: iteration 1 of 2, command 1 of 2");
            }
            other => panic!("{other:?}"),
        }
        match steps.get(3) {
            Some(Step::Command { done, status }) if done.index == 0 => {
                assert!(
                    status.ends_with("iteration 2 of 2, command 1 of 2"),
                    "{status}"
                )
            }
            other => panic!("unexpected step: {other:?}"),
        }
    }

    #[test]
    fn a_failed_verify_ends_the_run() {
        let f = FakeSystem::with_card(READER, &ATR);
        let verify = [0x00, 0x20, 0x00, 0x00, 0x02, 0x12, 0x34];
        f.answer(&verify, &[0x63, 0xC2]);
        let (steps, _, s) = run_all(
            &f,
            "step: pin\ncard: 00200000021234\nstep: after\ncard: 0084000008\n",
            5,
            SETTINGS,
        );
        assert_eq!(transmits(&f), vec![verify.to_vec()]);
        assert_eq!(steps.len(), 1);
        assert!(s.closing.contains(STOPPED_BY_VERIFY), "{}", s.report);
        assert_eq!(
            (s.passed, s.warnings, s.failed),
            (0, 1, 0),
            "no answer was expected, and 63C2 is a warning: the run stops all the same"
        );
    }

    /// With no expected answer, the status word decides, by its ISO/IEC 7816-4 category.
    #[test]
    fn without_an_expected_answer_the_status_word_decides() {
        let f = FakeSystem::with_card(READER, &ATR);
        let select = |p2: u8| [0x00, 0xA4, 0x00, p2, 0x02, 0x3F, 0x00];
        f.answer(&select(0), &[0x90, 0x00])
            .answer(&select(1), &[0x62, 0x83])
            .answer(&select(2), &[0x91, 0x00])
            .answer(&select(3), &[0x6A, 0x82]);
        // The fifth command has no scripted answer: the fake replies 6D00.
        let (steps, log, s) = run_all(
            &f,
            "card: 00A40000023F00\ncard: 00A40001023F00\ncard: 00A40002023F00\n\
             card: 00A40003023F00\ncard: 00B2000008\n",
            1,
            SETTINGS,
        );
        let marks: Vec<Mark> = steps
            .iter()
            .filter_map(|s| match s {
                Step::Command { done, .. } => Some(done.mark),
                Step::Finished { last: Some(d), .. } => Some(d.mark),
                _ => None,
            })
            .collect();
        assert_eq!(
            marks,
            vec![
                Mark::Passed,
                Mark::Warning,
                Mark::Warning,
                Mark::Failed,
                Mark::Failed
            ]
        );
        assert_eq!((s.passed, s.warnings, s.failed), (1, 2, 2));
        assert!(!s.compare_errors, "no comparison was made");
        assert_eq!(
            Run::finished_status(&s),
            "Run finished: 1 passed, 2 warnings, 2 failed."
        );
        assert!(s
            .closing
            .contains("Result: 5 commands, 1 passed, 2 warnings, 2 failed\n"));
        let why = "SW 6D00, an error: Instruction code not supported or invalid";
        assert!(
            s.report.contains(&format!("    failed  {why}\n")),
            "{}",
            s.report
        );
        assert!(log.contains(&format!("    failed  {why}\n")), "{log}");
        assert!(
            s.report.contains("    warning SW 6283: Warning"),
            "{}",
            s.report
        );
        match steps.get(3) {
            Some(Step::Command {
                done:
                    Done {
                        outcome: Outcome::Sent { mismatch, .. },
                        ..
                    },
                ..
            }) => assert!(
                mismatch
                    .as_deref()
                    .is_some_and(|m| m.starts_with("SW 6A82, an error")),
                "{mismatch:?}"
            ),
            other => panic!("{other:?}"),
        }
    }

    /// A test of an error expects its status word: getting it passes.
    #[test]
    fn an_expected_error_status_word_passes() {
        let f = FakeSystem::with_card(READER, &ATR);
        let (_, _, s) = run_all(&f, "card: 00B2000008\nexpect: 6D00\n", 1, SETTINGS);
        assert_eq!((s.passed, s.warnings, s.failed), (1, 0, 0));
        assert_eq!(
            Run::finished_status(&s),
            "Run finished: 1 passed, 0 failed."
        );
        assert!(!s.closing.contains("warning"));
    }

    #[test]
    fn a_compare_error_is_counted_and_reported() {
        let f = FakeSystem::with_card(READER, &ATR);
        let (_, log, s) = run_all(
            &f,
            "step: read\ncard: 00B2000008\nexpect: 90 00\n",
            1,
            SETTINGS,
        );
        assert_eq!((s.passed, s.failed), (0, 1));
        assert!(s.compare_errors);
        let block = "[1] read\n    card    00B2000008\n    answer  6D 00\n    expect  90 00\n    \
                     failed  byte 1 is 6D, expected 90\n";
        assert!(s.report.contains(block), "{}", s.report);
        assert!(log.contains(block), "{log}");
    }

    #[test]
    fn placeholders_and_bad_hex_fail_without_being_sent() {
        let f = FakeSystem::with_card(READER, &ATR);
        let mut sc = script("card: 00A4 <P1>\n");
        sc.commands.push(Command::new(Target::Card, "00ZZ"));
        let mut l = link(&f);
        let (mut run, _) = Run::start("t.apdu", 1);
        for _ in 0..2 {
            run.step(&sc.commands, Some(l.as_mut()), SETTINGS, &mut |_| {});
        }
        assert!(transmits(&f).is_empty());
        assert!(run
            .report()
            .contains(&format!("    skipped {PLACEHOLDER}\n")));
        assert!(run.report().contains("    skipped it isn't hexadecimal\n"));
        let s = run.stop("", SETTINGS, &mut |_| {});
        assert_eq!((s.passed, s.failed), (0, 2));
    }

    #[test]
    fn no_card_fails_every_command() {
        let sc = script("card: 0084000008\n");
        let (mut run, _) = Run::start("t.apdu", 1);
        let step = run.step(&sc.commands, None, SETTINGS, &mut |_| {});
        match step {
            Step::Command { done, .. } => {
                assert_eq!(done.index, 0);
                assert_eq!(done.mark, Mark::Failed);
                assert!(matches!(
                    done.outcome,
                    Outcome::Sent { reply: Err(ref e), .. } if e == card::NO_CONNECTION
                ));
            }
            other => panic!("{other:?}"),
        }
        assert!(run
            .report()
            .contains(&format!("    failed  {}\n", card::NO_CONNECTION)));
    }

    #[test]
    fn durations_follow_the_no_time_setting() {
        let f = FakeSystem::with_card(READER, &ATR);
        let (_, quiet, s) = run_all(&f, "card: 0084000008\n", 1, SETTINGS);
        assert!(
            !quiet.contains(" ms") && !s.closing.contains("Time:"),
            "{quiet}"
        );
        let timed = Settings {
            no_time: false,
            ..SETTINGS
        };
        let (_, log, s) = run_all(&f, "card: 0084000008\n", 1, timed);
        assert!(log.contains("    answer  6D 00  ("), "{log}");
        assert!(log.contains("Iteration 1: "), "{log}");
        assert!(s.closing.contains("Time: "), "{}", s.closing);
    }

    #[test]
    fn the_log_reads_like_the_report() {
        let f = FakeSystem::with_card(READER, &ATR);
        f.answer(&CHALLENGE, &RANDOM);
        let (_, log, s) = run_all(&f, "step: challenge\ncard: 00 84 00 00 08\n", 1, SETTINGS);
        assert_eq!(log, s.report);
        assert_eq!(
            s.report,
            "Run of t.apdu, 1 iteration\n\nIteration 1 of 1\n[1] challenge\n    \
             card    00 84 00 00 08\n    answer  01 02 03 04 05 06 07 08 90 00\n\n\
             Result: 1 command, 1 passed, 0 failed\n"
        );
    }

    #[test]
    fn stopping_reports_what_was_done() {
        let f = FakeSystem::with_card(READER, &ATR);
        let sc = script("card: 0084000008\ncard: 0084000008\n");
        let mut l = link(&f);
        let (mut run, _) = Run::start("t.apdu", 1);
        run.step(&sc.commands, Some(l.as_mut()), SETTINGS, &mut |_| {});
        let s = run.stop(STOPPED_BY_USER, SETTINGS, &mut |_| {});
        assert!(s.closing.contains(STOPPED_BY_USER));
        assert!(s.report.contains("Result: 1 command, 0 passed, 1 failed"));
        assert_eq!(transmits(&f).len(), 1);
    }

    #[test]
    fn the_atr_is_compared_as_the_card_gives_it() {
        let atr = || Some(ATR.to_vec());
        assert_eq!(check_atr("3B021450", atr()), AtrCheck::Matches);
        assert_eq!(check_atr("3B 02 14 ??", atr()), AtrCheck::Matches);
        assert_eq!(check_atr("3B 02 *", atr()), AtrCheck::Matches);
        assert_eq!(check_atr("3B0214509001", atr()), AtrCheck::Differs);
        assert_eq!(check_atr("3B021451", atr()), AtrCheck::Differs);
        assert_eq!(check_atr("3B021450", None), AtrCheck::Unreadable);
    }

    /// `reader:` sends a CCID escape command (SCardControl), with the code the reader lists or
    /// the system's; its answer has no status word to judge.
    #[test]
    fn reader_commands_go_to_the_reader_as_escape_commands() {
        let f = FakeSystem::with_card(READER, &ATR);
        f.answer_control(card::IOCTL_CCID_ESCAPE, &[0x1A, 0x00], &[0x01, 0x02]);
        let (steps, _, s) = run_all(
            &f,
            "step: version\nreader: 1A 00\nreader: 1A 00\nexpect: 01 03\n",
            1,
            SETTINGS,
        );
        assert!(transmits(&f).is_empty());
        let escapes = f
            .sent()
            .into_iter()
            .filter(|x| matches!(x, Sent::Control(c, d) if *c == card::IOCTL_CCID_ESCAPE && d == &[0x1A, 0x00]))
            .count();
        assert_eq!(escapes, 2);
        assert_eq!((s.passed, s.failed), (1, 1), "{}", s.report);
        assert!(s.report.contains("    reader  1A 00\n    answer  01 02\n"));
        assert!(s.report.contains("    failed  byte 2 is 02, expected 03\n"));
        assert!(matches!(
            steps.first(),
            Some(Step::Command {
                done: Done {
                    outcome: Outcome::Sent {
                        target: Target::Reader,
                        ..
                    },
                    ..
                },
                ..
            })
        ));
    }

    #[test]
    fn a_wrong_pin_test_that_gets_its_expected_answer_goes_on() {
        let f = FakeSystem::with_card(READER, &ATR);
        let verify = [0x00, 0x20, 0x00, 0x00, 0x02, 0x12, 0x34];
        f.answer(&verify, &[0x63, 0xC2]);
        let (_, _, s) = run_all(
            &f,
            "step: wrong pin\ncard: 00200000021234\nexpect: 63C2\nstep: after\ncard: 0084000008\n",
            1,
            SETTINGS,
        );
        assert_eq!(transmits(&f).len(), 2, "{}", s.report);
        assert!(!s.report.contains(STOPPED_BY_VERIFY));
        // Expecting 9000 and getting 63C2 stops the run.
        let (_, _, s) = run_all(
            &f,
            "step: pin\ncard: 00200000021234\nexpect: 9000\nstep: after\ncard: 0084000008\n",
            1,
            SETTINGS,
        );
        assert!(s.report.contains(STOPPED_BY_VERIFY), "{}", s.report);
        // A proprietary class (80) with INS 20 is not VERIFY.
        f.answer(&[0x80, 0x20, 0x00, 0x00, 0x00], &[0x6D, 0x00]);
        let (steps, _, _) = run_all(&f, "card: 8020000000\ncard: 0084000008\n", 1, SETTINGS);
        assert_eq!(steps.len(), 3, "two commands and the end of the iteration");
    }

    #[test]
    fn with_follow_up_the_whole_answer_is_compared() {
        let f = FakeSystem::with_card(READER, &ATR);
        f.answer(&[0x00, 0xA4, 0x04, 0x00, 0x02, 0xA0, 0x01], &[0x61, 0x02]);
        f.answer(&[0x00, 0xC0, 0x00, 0x00, 0x02], &[0x6F, 0x00, 0x90, 0x00]);
        let text = "card: 00A4040002A001\nexpect: 6F009000\n";
        let follow = Settings {
            follow_up: true,
            ..SETTINGS
        };
        let (steps, log, s) = run_all(&f, text, 1, follow);
        assert_eq!((s.passed, s.failed), (1, 0), "{}", s.report);
        let block = "    answer  61 02\n    then    00 C0 00 00 02\n    answer  6F 00 90 00\n    \
                     whole   6F 00 90 00\n";
        assert!(s.report.contains(block), "{}", s.report);
        assert!(log.contains(block), "{log}");
        let Some(Step::Command { done, .. }) = steps.first() else {
            panic!("{steps:?}");
        };
        let Outcome::Sent { legs, reply, .. } = &done.outcome else {
            panic!("{done:?}");
        };
        assert_eq!(legs.len(), 2);
        assert_eq!(
            reply.as_ref().map(|r| r.0.clone()),
            Ok(vec![0x6F, 0x00, 0x90, 0x00])
        );
        // Off: the same script fails on 6102, and nothing else is sent.
        let f2 = FakeSystem::with_card(READER, &ATR);
        f2.answer(&[0x00, 0xA4, 0x04, 0x00, 0x02, 0xA0, 0x01], &[0x61, 0x02]);
        let (_, _, s) = run_all(&f2, text, 1, SETTINGS);
        assert_eq!((s.passed, s.failed), (0, 1));
        assert_eq!(transmits(&f2).len(), 1);
    }

    #[test]
    fn durations_read_in_ms_then_seconds() {
        assert_eq!(time_taken(Duration::from_millis(12)), "12 ms");
        assert_eq!(time_taken(Duration::from_millis(1234)), "1.234 s");
        assert_eq!(time_taken(Duration::from_millis(60_005)), "60.005 s");
    }
}
