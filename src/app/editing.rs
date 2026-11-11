// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! Editing the script in place: the selected row's fields, adding, deleting and moving rows.
//! Every change marks the script as modified and forgets that row's last result.

use super::transcript::Level;
use super::ApduStudio;
use crate::scenario::{self, Command, Target};

impl ApduStudio {
    /// The selected row's command (its bytes or where it goes) was edited: keep its placeholder
    /// flag, and the field it is sent from, up to date.
    pub(super) fn command_edited(&mut self, i: usize) {
        if let Some(c) = self.scenario.commands.get_mut(i) {
            c.incomplete = scenario::has_placeholder(&c.text);
        }
        self.set_index(i);
        self.row_edited(i);
    }

    /// Any field of row `i` was edited.
    pub(super) fn row_edited(&mut self, i: usize) {
        self.scenario_modified = true;
        if let Some(r) = self.row_results.get_mut(i) {
            *r = None;
        }
    }

    /// Adds an empty row after the selected one (or first), and selects it.
    pub(super) fn add_row(&mut self) {
        if self.scenario.commands.len() >= scenario::MAX_COMMANDS {
            self.message(format!(
                "A script holds at most {} commands.",
                scenario::MAX_COMMANDS
            ));
            return;
        }
        let at = if self.scenario.commands.is_empty() {
            0
        } else {
            (self.index + 1).min(self.scenario.commands.len())
        };
        self.scenario
            .commands
            .insert(at, Command::new(Target::Card, ""));
        self.row_results
            .insert(at.min(self.row_results.len()), None);
        self.scenario_modified = true;
        self.set_index(at);
    }

    pub(super) fn delete_row(&mut self) {
        let i = self.index;
        if i >= self.scenario.commands.len() {
            return;
        }
        self.scenario.commands.remove(i);
        if i < self.row_results.len() {
            self.row_results.remove(i);
        }
        self.scenario_modified = true;
        let n = self.scenario.commands.len();
        if n == 0 {
            self.index = 0;
            self.command_text.clear();
        } else {
            self.set_index(i.min(n - 1));
        }
    }

    /// Moves the selected row up (`-1`) or down (`+1`); it stays selected.
    pub(super) fn move_row(&mut self, up: bool) {
        let i = self.index;
        let n = self.scenario.commands.len();
        let j = if up {
            match i.checked_sub(1) {
                Some(j) => j,
                None => return,
            }
        } else if i + 1 < n {
            i + 1
        } else {
            return;
        };
        self.scenario.commands.swap(i, j);
        if i < self.row_results.len() && j < self.row_results.len() {
            self.row_results.swap(i, j);
        }
        self.scenario_modified = true;
        self.index = j;
    }

    /// Step: sends the selected row, to the card or the reader, and selects the next.
    pub(super) fn step_selected(&mut self) {
        let i = self.index;
        let Some(target) = self.scenario.commands.get(i).map(|c| c.target) else {
            self.message("No command selected.");
            return;
        };
        self.set_index(i);
        match target {
            Target::Card => self.do_transmit(),
            Target::Reader => self.send_row_to_reader(i),
        }
        // Only a command that went out moves the selection on.
        if self.worker.busy() {
            if i + 1 < self.scenario.commands.len() {
                self.set_index(i + 1);
            } else {
                self.transcript
                    .note(Level::Info, "Step: that was the last command.");
            }
        }
    }
}
