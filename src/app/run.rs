// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! Running the script: the runner's steps go to the reader thread one per frame; each command run
//! lands in the exchanges and in its row's result.

use super::actions::sw_text;
use super::transcript::{Channel, Level, Verdict};
use super::ApduStudio;
use crate::runner::{self, AtrCheck, Done, Mark, Outcome, Run, Step, Summary};
use crate::scenario::Target;

impl ApduStudio {
    pub(super) fn start_run(&mut self) {
        if self.scenario.commands.is_empty() {
            self.message("The script has no command to run.");
            return;
        }
        if !self.need_card() {
            return;
        }
        let Some(exp) = self
            .scenario
            .atr_expected
            .clone()
            .filter(|_| !self.opts.expert)
        else {
            self.begin_run();
            return;
        };
        self.worker.call(
            |d| d.card.as_ref().and_then(|c| c.status().ok()).map(|s| s.atr),
            move |app: &mut ApduStudio, r| match runner::check_atr(&exp, r.ok().flatten()) {
                AtrCheck::Matches => app.begin_run(),
                AtrCheck::Unreadable => app.alert(
                    "The card's ATR can't be read, so it can't be checked against the script's atr: line. Connect again.",
                ),
                AtrCheck::Differs => app.alert(
                    "This card's ATR isn't the one the script expects (its atr: line). Use the script written for this card, or start ApduStudio with --expert to skip the check.",
                ),
            },
        );
    }

    pub(super) fn begin_run(&mut self) {
        let iterations = self.repeat_count.trim().parse::<u32>().unwrap_or(1).max(1);
        let file = self
            .file
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let (run, log_line) = Run::start(&file, iterations);
        self.log(&log_line);
        self.reset_row_results();
        let n = self.scenario.commands.len();
        self.transcript.note(
            Level::Info,
            format!(
                "Run: {n} command{}, {iterations} iteration{}",
                if n == 1 { "" } else { "s" },
                if iterations == 1 { "" } else { "s" }
            ),
        );
        self.run = Some(run);
        self.running = true;
        self.stop_requested = false;
        self.status = "Running. Stop or Esc stops after the command on its way.".into();
    }

    pub(super) fn run_settings(&self) -> runner::Settings {
        runner::Settings {
            response_buffer: self.opts.response_buffer,
            no_time: self.opts.no_time,
            follow_up: self.settings.follow_up,
        }
    }

    /// Records one command the run sent (or skipped): its exchange and its row's result.
    fn record(&mut self, done: Done) {
        match done.outcome {
            Outcome::Skipped(why) => {
                self.transcript.note(
                    Level::Warning,
                    format!("Command {} skipped: {}", done.index + 1, why),
                );
                self.set_row_result(done.index, Mark::Failed, "skipped".into());
            }
            Outcome::Sent {
                target,
                apdu,
                reply,
                mismatch,
                legs,
            } => {
                let channel = match target {
                    Target::Card => Channel::Card,
                    Target::Reader => Channel::Escape,
                };
                let expects = self
                    .scenario
                    .commands
                    .get(done.index)
                    .is_some_and(|c| c.expected.is_some());
                let verdict = match (&reply, mismatch) {
                    (_, Some(m)) => Verdict::Failed(m),
                    (Ok(_), None) if expects => Verdict::Passed,
                    _ => Verdict::Unchecked,
                };
                let short = match &reply {
                    Ok((r, _)) => sw_text(r),
                    Err(_) => "error".into(),
                };
                if legs.is_empty() {
                    let (reply, took) = match reply {
                        Ok((r, d)) => (Ok(r), Some(d)),
                        Err(e) => (Err(e), None),
                    };
                    self.transcript
                        .exchange(channel, apdu, reply, took, verdict);
                } else {
                    let whole = reply.as_ref().ok().map(|(r, _)| r.clone());
                    self.show_legs(&legs, reply.err(), verdict, whole.as_deref(), done.index);
                }
                self.set_row_result(done.index, done.mark, short);
            }
        }
    }

    /// Shows the end of a run: the totals, and an alert after compare errors.
    pub(super) fn finish_run(&mut self, status: String, summary: Summary) {
        self.run = None;
        self.running = false;
        self.stop_requested = false;
        // The report's end: why it stopped, if it did, the result and the time.
        self.transcript
            .note(Level::Info, summary.closing.trim().to_string());
        if summary.compare_errors {
            let log = if self.log.is_some() {
                format!(" and in the log file {}", self.log_path.display())
            } else {
                String::new()
            };
            self.alert(format!(
                "Some answers weren't the expected ones: see the failed commands in the exchanges{log}."
            ));
        }
        // The counts stay in the status line.
        self.status = status;
    }

    /// Sends the scenario's next command to the reader thread; called each frame while running.
    /// `stop` (Escape, or the Stop button) ends the run, after the command on its way if any.
    pub(super) fn step_run(&mut self, stop: bool) {
        if !self.running {
            return;
        }
        if stop {
            self.stop_requested = true;
        }
        if self.worker.busy() {
            return;
        }
        let Some(mut run) = self.run.take() else {
            return;
        };
        if self.stop_requested {
            let mut lines: Vec<String> = Vec::new();
            let settings = self.run_settings();
            let summary = run.stop(runner::STOPPED_BY_USER, settings, &mut |l| {
                lines.push(l.to_string())
            });
            self.write_log_lines(&lines);
            self.finish_run(Run::finished_status(&summary), summary);
            return;
        }
        let next = run.next_index();
        if next < self.scenario.commands.len() {
            self.set_index(next);
        }
        let commands = self.scenario.commands.clone();
        let settings = self.run_settings();
        self.worker.call(
            move |d| {
                let mut lines: Vec<String> = Vec::new();
                let step = run.step(&commands, d.link(), settings, &mut |l| {
                    lines.push(l.to_string())
                });
                (run, step, lines)
            },
            |app: &mut ApduStudio, r| {
                let (run, step, lines) = match r {
                    Ok(x) => x,
                    Err(e) => {
                        app.run = None;
                        app.running = false;
                        app.stop_requested = false;
                        app.status = e.to_string();
                        return;
                    }
                };
                app.write_log_lines(&lines);
                match step {
                    Step::Command { done, status } => {
                        app.record(done);
                        app.status = status;
                        app.run = Some(run);
                    }
                    Step::Iteration { status } => {
                        app.transcript.note(Level::Info, status.clone());
                        app.status = status;
                        app.run = Some(run);
                    }
                    Step::Finished {
                        status,
                        summary,
                        last,
                    } => {
                        if let Some(done) = last {
                            app.record(done);
                        }
                        app.finish_run(status, summary);
                    }
                }
            },
        );
    }

    pub(super) fn write_log_lines(&mut self, lines: &[String]) {
        for l in lines {
            self.log(l);
        }
    }
}
