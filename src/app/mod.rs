// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! The ApduStudio window: a connection bar, the scenario, the exchanges and a command line.
//!
//! - `actions`: what each control does, through the reader thread;
//! - `files`: scenario files and the log file;
//! - `run`: running the script, one command per frame;
//! - `transcript`: the exchanges of the session;
//! - `main_view`: the window's panels;
//! - `dialogs`: the few questions that need an answer before going on;
//! - `help`: the help window (F1), its topics as data;
//! - `widgets`: the theme and small drawing helpers.

mod actions;
mod detail;
mod dialogs;
mod editing;
mod files;
mod help;
mod main_view;
mod run;
#[cfg(test)]
mod tests;
mod transcript;
mod widgets;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::journal::Journal;
use crate::reader::{Disp, Proto, Share, SystemOpener};
use crate::runner::{time_taken, Mark, Run};
use crate::scenario::{Command, Scenario, Target};
use crate::settings::{self, Settings};
use crate::worker::Worker;
use crate::Options;
use dialogs::Dialog;
use egui::Ui;
use transcript::{Level, Transcript};

pub const APP_NAME: &str = "ApduStudio";
/// The version, from Cargo.toml: the one place it is set.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// The commit a CI build was made from (`APDU_STUDIO_COMMIT`, set by the workflows); None for a
/// build made elsewhere.
pub const BUILD_COMMIT: Option<&str> = option_env!("APDU_STUDIO_COMMIT");

/// The build, as the About box and the window title show it: "a1b2c3d" (the commit's first
/// seven digits) for a CI build, else "local build".
pub fn build_label() -> String {
    match BUILD_COMMIT.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => c.get(..7).unwrap_or(c).to_string(),
        None => "local build".to_string(),
    }
}

/// "ApduStudio 1.0.0 (a1b2c3d)", or "ApduStudio 1.0.0 (local build)".
pub fn version_line() -> String {
    format!("{} {} ({})", APP_NAME, APP_VERSION, build_label())
}
/// The window's size at start, and the smallest it may get.
pub const WINDOW_SIZE: [f32; 2] = [1180.0, 720.0];
pub const MIN_WINDOW_SIZE: [f32; 2] = [900.0, 540.0];
const LOG_NAME: &str = "ApduStudio.log";

/// Where Start log writes. On Windows, next to the program. Elsewhere the
/// program usually sits in a folder people can't write to (`/usr/bin`, an app bundle), so the
/// log goes to the folder ApduStudio was started from.
fn log_path() -> PathBuf {
    let folder = if cfg!(windows) {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(PathBuf::from))
    } else {
        std::env::current_dir().ok()
    };
    folder
        .map(|d| d.join(LOG_NAME))
        .unwrap_or_else(|| PathBuf::from(LOG_NAME))
}
/// Commands remembered by the command line.
const MAX_HISTORY: usize = 200;

/// The open connection, as the connection bar shows it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct CardInfo {
    pub(super) share: Share,
    pub(super) protocol: String,
    pub(super) atr: Vec<u8>,
}

/// How a scenario command went the last time it was sent.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RowResult {
    pub(super) mark: Mark,
    /// The status word, or why there is none ("error", "skipped").
    pub(super) short: String,
}

pub struct ApduStudio {
    opts: Options,
    /// Every reader call runs on this thread (PC/SC, or a fake reader in tests).
    worker: Worker<ApduStudio>,
    /// The smart card service was opened.
    service: bool,
    readers: Vec<String>,
    reader: Option<usize>,
    /// A card connection is open (it lives on the reader thread).
    card_open: bool,
    card_info: Option<CardInfo>,
    /// The connection bar's choices.
    share: Share,
    proto: Proto,
    disp: Disp,
    /// Waiting for a card to connect to, with these settings.
    waiting: Option<(Share, Proto)>,

    scenario: Scenario,
    /// The scenario was edited since it was opened or saved.
    scenario_modified: bool,
    file: Option<PathBuf>,
    /// The selected scenario row.
    index: usize,
    /// How each scenario row went last time (same length as the scenario).
    row_results: Vec<Option<RowResult>>,
    /// The command line.
    command_text: String,
    history: Vec<String>,
    /// Where ↑/↓ are in the history (None: the line being typed).
    history_pos: Option<usize>,

    transcript: Transcript,
    status: String,

    /// The open log file. It is closed, and the person told, when a write fails.
    log: Option<Journal>,
    log_path: PathBuf,

    repeat_count: String,
    escape_text: String,
    escape_history: Vec<String>,

    dialogs: VecDeque<Dialog>,
    /// The scenario run; away on the reader thread while one of its steps runs.
    run: Option<Run>,
    /// A scenario run is going on.
    running: bool,
    /// Stop was asked while a step was on the reader thread: stop when it comes back.
    stop_requested: bool,
    /// Waiting for a card: the last check found one in the reader.
    card_seen: bool,
    /// The last look at the selected reader: its name, and whether a card was in it.
    seen: Option<(String, bool)>,
    /// The connected card was taken out: the connection is lost until Reconnect.
    card_removed: bool,
    /// What the exchanges list shows (Find, Problems only).
    filter: transcript::Filter,
    last_presence_check: Option<Instant>,

    /// Where the scenario panel's parts were on the last frame.
    scenario_layout: ScenarioLayout,
    /// What is kept between runs (the theme), as read at start and changed since.
    settings: Settings,
    /// The help window (F1).
    help: help::HelpState,

    frames: u32,
    shot_requested: bool,
}

/// The scenario panel's measures on the last frame: what the list must leave below it (its
/// frame, the gap and the row editor), and where things were, for the tests.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
struct ScenarioLayout {
    below: f32,
    list: egui::Rect,
    panel: egui::Rect,
    content_bottom: f32,
}

impl Default for ScenarioLayout {
    fn default() -> Self {
        Self {
            // A guess for the first frame, which is laid out again with the measure.
            below: 112.0,
            list: egui::Rect::NOTHING,
            panel: egui::Rect::NOTHING,
            content_bottom: 0.0,
        }
    }
}

impl ApduStudio {
    pub fn new(cc: &eframe::CreationContext<'_>, opts: Options, open_system: SystemOpener) -> Self {
        widgets::apply_style(&cc.egui_ctx);
        let app = Self::create(opts, open_system);
        app.apply_theme(&cc.egui_ctx);
        app
    }

    /// The theme at start: `--theme` for this run, else the one kept in the settings.
    fn apply_theme(&self, ctx: &egui::Context) {
        match self.opts.theme {
            Some(t) => ctx.set_theme(t),
            None => ctx.set_theme(self.settings.theme),
        }
    }

    /// The theme chosen in the window: used at once, and kept for the next runs.
    pub(super) fn choose_theme(&mut self, ctx: &egui::Context, theme: egui::ThemePreference) {
        ctx.set_theme(theme);
        if theme == self.settings.theme {
            return;
        }
        self.settings.theme = theme;
        self.keep_settings("The theme");
    }

    /// Whether GET RESPONSE follows 61xx and a new Le follows 6Cxx: used at once, and kept.
    pub(super) fn set_follow_up(&mut self, on: bool) {
        if on == self.settings.follow_up {
            return;
        }
        self.settings.follow_up = on;
        self.keep_settings("The 61xx / 6Cxx choice");
    }

    /// Writes the settings for the next runs; `what` names the change in the message if it
    /// can't be kept.
    fn keep_settings(&mut self, what: &str) {
        let Some(path) = self.opts.settings.clone() else {
            return;
        };
        if self.settings.is_newer() {
            self.message(format!(
                "{what} is not kept: {} was written by a newer version of {}.",
                path.display(),
                APP_NAME
            ));
        } else if let Err(e) = settings::save(&path, &self.settings) {
            self.message(format!(
                "{what} could not be kept: writing {} failed: {}",
                path.display(),
                e
            ));
        }
    }

    /// The app state without a window: `open_system` gives it the smart card service.
    pub fn create(opts: Options, open_system: SystemOpener) -> Self {
        let log_path = log_path();
        let (kept, unreadable) = match &opts.settings {
            Some(p) => settings::load(p),
            None => (Settings::default(), None),
        };

        let mut app = ApduStudio {
            opts,
            worker: Worker::spawn(open_system),
            service: false,
            readers: Vec::new(),
            reader: None,
            card_open: false,
            card_info: None,
            share: Share::Shared,
            proto: Proto::Any,
            disp: Disp::PowerDown,
            waiting: None,
            scenario: Scenario::default(),
            scenario_modified: false,
            file: None,
            index: 0,
            row_results: Vec::new(),
            command_text: String::new(),
            history: Vec::new(),
            history_pos: None,
            transcript: Transcript::new(),
            status: String::new(),
            log: None,
            log_path,
            repeat_count: "1".to_string(),
            // Empty at first: an escape command means something only to one reader.
            escape_text: String::new(),
            escape_history: Vec::new(),
            dialogs: VecDeque::new(),
            run: None,
            running: false,
            stop_requested: false,
            card_seen: false,
            seen: None,
            card_removed: false,
            filter: transcript::Filter::default(),
            last_presence_check: None,
            scenario_layout: ScenarioLayout::default(),
            settings: kept,
            help: help::HelpState::default(),
            frames: 0,
            shot_requested: false,
        };
        if let (Some(e), Some(p)) = (unreadable, app.opts.settings.clone()) {
            app.message(format!(
                "The settings could not be read from {}: {}. The defaults are used.",
                p.display(),
                e
            ));
        }
        app.start_resource_manager();
        if let Some(f) = app.opts.open_file.clone() {
            app.open_path(f);
        }
        match app.opts.show {
            Some(crate::Show::Help) => app.help.open = true,
            Some(crate::Show::About) => app.dialogs.push_back(Dialog::About),
            None => {}
        }
        app
    }

    // ----------------------------------------------------------------- helpers

    /// Something the person should know: a warning in the
    /// exchanges, and the status line.
    fn message(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.status = text.replace('\n', " ");
        self.transcript.note(Level::Warning, text);
    }

    /// Something that stops what was asked.
    fn alert(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.status = text.replace('\n', " ");
        self.transcript.note(Level::Error, text);
    }

    /// Writes to the log file, if one is open. The log is the record of a test, so a write that
    /// fails closes it and says so, rather than leaving a log with lines missing.
    fn log(&mut self, text: &str) {
        let Some(j) = self.log.as_mut() else {
            return;
        };
        if let Err(e) = j.write(text) {
            let path = j.path().display().to_string();
            self.log = None;
            self.alert(format!(
                "Writing to the log file {} failed: {}. The log file was closed; open it again to go on logging.",
                path, e
            ));
        }
    }

    /// "  (12 ms)", to end a log line with how long it took; nothing with `--no-times`.
    fn timed(&self, d: Duration) -> String {
        if self.opts.no_time {
            String::new()
        } else {
            format!("  ({})", time_taken(d))
        }
    }

    fn reader_name(&self) -> String {
        self.reader
            .and_then(|i| self.readers.get(i))
            .cloned()
            .unwrap_or_default()
    }

    /// Applies the answers that came back from the reader thread; never waits.
    fn pump(&mut self) {
        for (then, answer) in self.worker.ready() {
            then(self, answer);
        }
    }

    /// Waits for every answer still expected (tests: they check the result of an action).
    #[cfg(test)]
    fn settle(&mut self) {
        let end = Instant::now() + Duration::from_secs(5);
        while self.worker.has_pending() && Instant::now() < end {
            if let Some((then, answer)) = self.worker.wait(Duration::from_millis(100)) {
                then(self, answer);
            }
        }
        assert!(
            !self.worker.has_pending(),
            "the reader thread did not answer"
        );
    }

    /// Selects script row `i` and puts its command where it is sent from: the command line for
    /// the card, the CCID escape field for the reader.
    pub(super) fn set_index(&mut self, i: usize) {
        self.index = i;
        if let Some(c) = self.scenario.commands.get(i) {
            match c.target {
                Target::Card => {
                    self.command_text = c.text.clone();
                    self.history_pos = None;
                }
                Target::Reader => self.escape_text = c.text.clone(),
            }
        }
    }

    /// The selected row, when it is a card command and the command line holds it unchanged.
    pub(super) fn current_list_command(&self) -> Option<&Command> {
        self.scenario
            .commands
            .get(self.index)
            .filter(|c| c.target == Target::Card && c.text == self.command_text)
    }

    /// Forgets how the scenario's rows went (a new file, or a new run).
    pub(super) fn reset_row_results(&mut self) {
        self.row_results = vec![None; self.scenario.commands.len()];
    }

    /// No run, no question and no answer awaited: the actions may start.
    fn idle(&self) -> bool {
        !self.running && self.dialogs.is_empty() && !self.worker.busy()
    }

    pub(super) fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.opts.screenshot.clone() else {
            return;
        };
        self.frames += 1;
        // A demo capture shows a run of the scenario given on the command line.
        if self.opts.demo
            && self.frames == 30
            && self.card_open
            && self.idle()
            && !self.scenario.commands.is_empty()
        {
            self.start_run();
        }
        // ...with the first answer that carries data shown in detail.
        if self.opts.demo && self.frames == 90 {
            let first = self
                .transcript
                .entries()
                .find(|e| {
                    matches!(&e.body, transcript::Body::Exchange { reply: Ok(r), .. } if r.len() > 2)
                })
                .map(|e| e.id);
            self.transcript.selected = first;
        }
        ctx.request_repaint();
        if self.frames >= 120 && !self.shot_requested {
            self.shot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            let [w, h] = img.size;
            let mut bytes = Vec::with_capacity(w * h * 4);
            for p in &img.pixels {
                bytes.extend_from_slice(&p.to_srgba_unmultiplied());
            }
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, bytes) {
                let _ = buf.save(&path);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// One frame: answers from the reader thread, the run, the keys, then the window.
    pub(super) fn frame(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.pump();
        let (esc, f1, f5) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::F1),
                i.key_pressed(egui::Key::F5),
            )
        });
        if self.running {
            self.step_run(esc);
            ctx.request_repaint();
        } else if esc && self.waiting.is_some() {
            self.cancel_waiting();
        } else if esc && self.help.open && self.dialogs.is_empty() {
            self.help.open = false;
        }
        if self.waiting.is_some() {
            self.wait_for_card();
            ctx.request_repaint_after(actions::WAIT_POLL);
        } else if self.service && self.reader.is_some() && !self.running {
            self.watch_reader();
            ctx.request_repaint_after(actions::WATCH_POLL);
        }
        if self.worker.has_pending() {
            // Look again soon for the reader thread's answer.
            ctx.request_repaint_after(Duration::from_millis(15));
        }
        if f1 && self.dialogs.is_empty() {
            self.toggle_help();
        }
        if f5 && self.idle() && !self.scenario.commands.is_empty() {
            self.start_run();
        }

        self.main_window(ui);
        self.help_window(&ctx);
        self.show_dialog(&ctx);
        self.handle_screenshot(&ctx);
    }
}

impl eframe::App for ApduStudio {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }
}

// The reader module's choices, by name, for the views.
pub(super) const SHARES: [(Share, &str); 3] = [
    (Share::Shared, "Shared"),
    (Share::Exclusive, "Exclusive"),
    (Share::Direct, "Direct"),
];
pub(super) const PROTOS: [(Proto, &str); 3] = [
    (Proto::T0, "T0"),
    (Proto::T1, "T1"),
    (Proto::Any, "T0 | T1"),
];
pub(super) const DISPS: [(Disp, &str); 4] = [
    (Disp::PowerDown, "Power down"),
    (Disp::Reset, "Reset"),
    (Disp::Leave, "Leave"),
    (Disp::Eject, "Eject"),
];
