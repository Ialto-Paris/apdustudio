// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! Script files (open, save) and the log file.

use crate::journal::Journal;
use std::path::PathBuf;

use super::dialogs::{Confirm, Dialog};
use super::transcript::Level;
use super::{ApduStudio, APP_NAME, APP_VERSION};
use crate::scenario::{self, Scenario};

impl ApduStudio {
    /// Open: asks first when the scenario has unsaved changes.
    pub(super) fn do_open_file(&mut self) {
        if self.scenario_modified {
            self.dialogs.push_back(Dialog::Confirm {
                text: "The script has unsaved changes. Open another one anyway?".into(),
                what: Confirm::OpenDiscarding,
            });
        } else {
            self.pick_and_open();
        }
    }

    /// Save: the scenario as it is now, under a name the person picks.
    pub(super) fn save_current(&mut self) {
        let sc = self.scenario.clone();
        if self.save_scenario(&sc) {
            self.scenario_modified = false;
        }
    }

    pub(super) fn pick_and_open(&mut self) {
        let mut dlg = rfd::FileDialog::new()
            .set_title("Open a script")
            .add_filter("ApduStudio scripts (*.apdu)", &[scenario::EXTENSION])
            .add_filter("All files", &["*"]);
        if let Some(dir) = self.file.as_ref().and_then(|f| f.parent()) {
            dlg = dlg.set_directory(dir);
        }
        let Some(path) = dlg.pick_file() else { return };
        self.open_path(path);
    }

    pub(super) fn open_path(&mut self, path: PathBuf) {
        match scenario::load(&path) {
            Ok(sc) => {
                let too_many = sc.too_many;
                self.scenario = sc;
                self.file = Some(path.clone());
                self.index = 0;
                self.command_text.clear();
                self.reset_row_results();
                self.scenario_modified = false;
                if !self.scenario.commands.is_empty() {
                    self.set_index(0);
                }
                let name = path.display().to_string();
                self.status = format!("Opened {name}");
                self.log(&format!("Opened {name}\n"));
                self.transcript.note(
                    Level::Info,
                    format!(
                        "Opened {} ({} commands)",
                        name,
                        self.scenario.commands.len()
                    ),
                );
                if too_many {
                    self.message(format!(
                        "Only the first {} commands of the file were read.",
                        scenario::MAX_COMMANDS
                    ));
                }
                if self.scenario.commands.is_empty() {
                    self.message("The script has no command.");
                }
            }
            Err(e) => self.message(format!("Can't open {}: {e}", path.display())),
        }
    }

    pub(super) fn do_toggle_log(&mut self) {
        if self.log.is_some() {
            self.log("\n");
            self.log = None;
            self.status = format!("Stopped logging to {}", self.log_path.display());
            return;
        }
        if self.log_path.exists() {
            self.dialogs.push_back(Dialog::Confirm {
                text: format!(
                    "{} already exists. Add to it? No starts it again.",
                    self.log_path.display()
                ),
                what: Confirm::ContinueLog,
            });
        } else {
            self.open_log(false);
        }
    }

    pub(super) fn open_log(&mut self, append: bool) {
        let opened = if append {
            Journal::append(&self.log_path)
        } else {
            Journal::create(&self.log_path)
        };
        match opened {
            Ok(j) => {
                self.log = Some(j);
                let mut header = format!("\n{APP_NAME} {APP_VERSION} log\n");
                if !self.reader_name().is_empty() {
                    header.push_str(&format!("Reader: {}\n", self.reader_name()));
                }
                header.push('\n');
                self.log(&header);
                // A failed write has closed it again and said why.
                if self.log.is_some() {
                    self.status = format!("Logging to {}", self.log_path.display());
                }
            }
            Err(e) => self.message(format!(
                "Can't open the log file {}: {e}",
                self.log_path.display()
            )),
        }
    }

    pub(super) fn save_scenario(&mut self, sc: &Scenario) -> bool {
        let mut dlg = rfd::FileDialog::new()
            .set_title("Save the script")
            .add_filter("ApduStudio scripts (*.apdu)", &[scenario::EXTENSION])
            .add_filter("All files", &["*"]);
        if let Some(f) = &self.file {
            if let Some(d) = f.parent() {
                dlg = dlg.set_directory(d);
            }
            if let Some(n) = f.file_name() {
                dlg = dlg.set_file_name(n.to_string_lossy());
            }
        } else {
            dlg = dlg.set_file_name(format!("script.{}", scenario::EXTENSION));
        }
        let Some(path) = dlg.save_file() else {
            return false;
        };
        match scenario::save(&path, sc) {
            Ok(()) => {
                self.file = Some(path.clone());
                self.status = format!("Saved {}", path.display());
                true
            }
            Err(e) => {
                self.message(format!("Can't save {}: {e}", path.display()));
                false
            }
        }
    }
}
