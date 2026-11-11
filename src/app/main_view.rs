// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! The window: a connection bar at the top, the script on the left, the exchanges in the
//! middle, and the command line, CCID escape, log and status at the bottom.

use egui::text::LayoutJob;
use egui::{
    Align, Align2, Color32, FontId, Frame, Key, Layout, Margin, Modifiers, Rect, RichText, Sense,
    Stroke, TextFormat, TextStyle, Ui, Vec2,
};

use super::dialogs::{Confirm, Dialog};
use super::transcript::{channel_label, clock, hex, Body, Entry, Level, Verdict};
use super::widgets::{dot, Palette};
use super::{ApduStudio, DISPS, PROTOS, SHARES};
use crate::iso;
use crate::reader::Share;
use crate::runner::Mark;
use crate::scenario::Target;

/// Bytes shown per exchange line in the list; the detail shows them all.
const LIST_BYTES: usize = 48;

fn bar_frame(p: &Palette) -> Frame {
    Frame::new()
        .fill(p.panel)
        .inner_margin(Margin::symmetric(10, 7))
        .stroke(Stroke::new(1.0, p.border))
}

fn mono(text: impl Into<String>) -> RichText {
    RichText::new(text).monospace()
}

/// A line of coloured pieces, in one font.
fn line_job(pieces: &[(&str, Color32)], font: &FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (text, color) in pieces {
        job.append(
            text,
            0.0,
            TextFormat {
                font_id: font.clone(),
                color: *color,
                ..Default::default()
            },
        );
    }
    job
}

/// Bytes for the list: all of them, or the first `LIST_BYTES` and how many there are.
fn list_hex(bytes: &[u8]) -> String {
    if bytes.len() <= LIST_BYTES {
        hex(bytes)
    } else {
        format!(
            "{} ... ({} bytes)",
            hex(bytes.get(..LIST_BYTES).unwrap_or_default()),
            bytes.len()
        )
    }
}

/// Paints `text` on one line from `pos`, at most `max_width` wide; longer text ends with "…".
fn elided(
    painter: &egui::Painter,
    pos: egui::Pos2,
    text: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
) {
    if max_width <= 0.0 {
        return;
    }
    let mut job = LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(font, color));
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width);
    let galley = painter.layout_job(job);
    painter.galley(pos, galley, color);
}

/// How many text lines an entry takes in the list.
fn entry_lines(e: &Entry) -> usize {
    match &e.body {
        Body::Exchange { .. } => 2,
        Body::Note { text, .. } => text.lines().count().max(1),
    }
}

impl ApduStudio {
    pub(super) fn main_window(&mut self, ui: &mut Ui) {
        let p = Palette::of_ui(ui);
        self.shortcuts(ui);
        egui::Panel::top("connection")
            .frame(bar_frame(&p))
            .show(ui, |ui| self.connection_bar(ui, &p));
        egui::Panel::bottom("console")
            .frame(bar_frame(&p))
            .show(ui, |ui| self.console(ui, &p));
        egui::Panel::left("scenario")
            .resizable(true)
            .default_size(480.0)
            .min_size(340.0)
            .frame(bar_frame(&p))
            .show(ui, |ui| self.scenario_panel(ui, &p));
        egui::CentralPanel::default_margins()
            .frame(Frame::new().fill(p.field).inner_margin(Margin::same(8)))
            .show(ui, |ui| self.exchanges(ui, &p));
    }

    /// Keys that work anywhere in the window (F1, F5 and Esc are read in `frame`).
    fn shortcuts(&mut self, ui: &mut Ui) {
        if !self.dialogs.is_empty() {
            return;
        }
        let (open, save, step) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::O),
                i.consume_key(Modifiers::COMMAND, Key::S),
                i.consume_key(Modifiers::NONE, Key::F6),
            )
        });
        if open && self.idle() {
            self.do_open_file();
        }
        if save && !self.running {
            self.save_current();
        }
        if step && self.idle() {
            self.step_selected();
        }
        // Closing with unsaved changes asks first.
        if ui.input(|i| i.viewport().close_requested()) && self.scenario_modified {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !self.dialogs.iter().any(|d| {
                matches!(
                    d,
                    Dialog::Confirm {
                        what: Confirm::QuitDiscarding,
                        ..
                    }
                )
            }) {
                self.dialogs.push_back(Dialog::Confirm {
                    text: "The script has unsaved changes. Close anyway?".into(),
                    what: Confirm::QuitDiscarding,
                });
            }
        }
    }

    // ----------------------------------------------------------------- connection bar

    fn connection_bar(&mut self, ui: &mut Ui, p: &Palette) {
        let idle = self.idle();
        ui.horizontal(|ui| {
            ui.label("Reader");
            let names = self.readers.clone();
            let mut choice = self.reader;
            ui.add_enabled_ui(idle && !names.is_empty(), |ui| {
                egui::ComboBox::from_id_salt("reader")
                    .width(260.0)
                    .selected_text(if names.is_empty() {
                        "(no reader)".to_string()
                    } else {
                        self.reader_name()
                    })
                    .show_ui(ui, |ui| {
                        for (i, n) in names.iter().enumerate() {
                            ui.selectable_value(&mut choice, Some(i), n);
                        }
                    });
            });
            if let Some(c) = choice.filter(|c| Some(*c) != self.reader) {
                self.choose_reader(c);
            }
            if ui
                .add_enabled(idle, egui::Button::new("Refresh"))
                .on_hover_text("List the readers again")
                .clicked()
            {
                self.refresh_readers();
            }
            ui.separator();

            let connected = self.card_open;
            ui.add_enabled_ui(idle && !connected && self.waiting.is_none(), |ui| {
                for (s, name) in SHARES {
                    ui.selectable_value(&mut self.share, s, name);
                }
                ui.label(RichText::new("·").color(p.weak));
                ui.add_enabled_ui(self.share != Share::Direct, |ui| {
                    for (pr, name) in PROTOS {
                        ui.selectable_value(&mut self.proto, pr, name);
                    }
                });
            });
            ui.separator();

            if connected {
                if self.card_removed {
                    let back = self.seen.as_ref().is_some_and(|(_, present)| *present);
                    if ui
                        .add_enabled(idle && back, egui::Button::new("Reconnect"))
                        .on_hover_text(
                            "The card was taken out: connect again, the same way, once it is back",
                        )
                        .clicked()
                    {
                        self.do_reconnect();
                    }
                }
                if ui
                    .add_enabled(idle, egui::Button::new("Disconnect"))
                    .clicked()
                {
                    self.do_disconnect(self.disp);
                }
                ui.add_enabled_ui(idle, |ui| {
                    egui::ComboBox::from_id_salt("disp")
                        .width(96.0)
                        .selected_text(
                            DISPS
                                .iter()
                                .find(|(d, _)| *d == self.disp)
                                .map(|(_, n)| *n)
                                .unwrap_or_default(),
                        )
                        .show_ui(ui, |ui| {
                            for (d, name) in DISPS {
                                ui.selectable_value(&mut self.disp, d, name);
                            }
                        })
                        .response
                        .on_hover_text("What the reader does with the card on disconnect");
                });
            } else if self.waiting.is_some() {
                if ui.button("Cancel").on_hover_text("Esc").clicked() {
                    self.cancel_waiting();
                }
            } else if ui
                .add_enabled(idle && self.reader.is_some(), egui::Button::new("Connect"))
                .clicked()
            {
                self.do_connect(self.share, self.proto);
            }
        });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            match (&self.card_info, self.waiting) {
                (Some(info), _) => {
                    dot(ui, p.ok);
                    ui.label(format!("Connected · {:?} · {}", info.share, info.protocol));
                    ui.label(RichText::new("ATR").color(p.weak));
                    ui.add(egui::Label::new(mono(hex(&info.atr))).selectable(true));
                }
                (None, Some(_)) => {
                    dot(ui, p.warn);
                    ui.label(format!("Waiting for a card in {}...", self.reader_name()));
                }
                (None, None) if !self.service => {
                    dot(ui, p.err);
                    ui.label("The smart card service isn't available");
                }
                (None, None) => {
                    dot(ui, p.weak);
                    ui.label("No card connected");
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .selectable_label(self.help.open, " ? ")
                    .on_hover_text("Help: functions, scripts, options, keys (F1)")
                    .clicked()
                {
                    self.toggle_help();
                }
                let mut pref = ui.ctx().options(|o| o.theme_preference);
                let before = pref;
                egui::ComboBox::from_id_salt("theme")
                    .width(70.0)
                    .selected_text(match pref {
                        egui::ThemePreference::System => "System",
                        egui::ThemePreference::Light => "Light",
                        egui::ThemePreference::Dark => "Dark",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut pref, egui::ThemePreference::System, "System");
                        ui.selectable_value(&mut pref, egui::ThemePreference::Light, "Light");
                        ui.selectable_value(&mut pref, egui::ThemePreference::Dark, "Dark");
                    })
                    .response
                    .on_hover_text("Theme (kept for the next runs)");
                if pref != before {
                    let ctx = ui.ctx().clone();
                    self.choose_theme(&ctx, pref);
                }
                if ui
                    .add_enabled(idle && self.card_open, egui::Button::new("UID"))
                    .on_hover_text(
                        "The contactless card's UID: GET DATA FF CA 00 00 00 (PC/SC part 3), sent \
                         to the card",
                    )
                    .clicked()
                {
                    self.do_get_uid();
                }
                if ui
                    .add_enabled(
                        idle && self.service && self.reader.is_some(),
                        egui::Button::new("Reader info"),
                    )
                    .on_hover_text(
                        "The reader's vendor, model, version and features (SCardGetAttrib, \
                         PC/SC part 10). Without a connection, a direct one is opened for it.",
                    )
                    .clicked()
                {
                    self.do_reader_info();
                }
                if ui
                    .add_enabled(idle && self.card_open, egui::Button::new("Card state"))
                    .clicked()
                {
                    self.do_card_state();
                }
                if ui
                    .add_enabled(idle && self.service, egui::Button::new("Readers' state"))
                    .on_hover_text("GetStatusChange for every reader")
                    .clicked()
                {
                    self.do_get_status();
                }
            });
        });
    }

    // ----------------------------------------------------------------- scenario

    fn scenario_panel(&mut self, ui: &mut Ui, p: &Palette) {
        let idle = self.idle();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Script").strong());
            let name = self
                .file
                .as_ref()
                .and_then(|f| f.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "(none)".into());
            let marker = if self.scenario_modified { " *" } else { "" };
            ui.label(RichText::new(format!("{name}{marker}")).color(p.weak))
                .on_hover_text(
                    self.file
                        .as_ref()
                        .map(|f| f.display().to_string())
                        .unwrap_or_default(),
                );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(!self.running, egui::Button::new("Save"))
                    .on_hover_text("Ctrl+S")
                    .clicked()
                {
                    self.save_current();
                }
                if ui
                    .add_enabled(idle, egui::Button::new("Open..."))
                    .on_hover_text("Ctrl+O")
                    .clicked()
                {
                    self.do_open_file();
                }
            });
        });
        ui.add_space(4.0);
        self.run_controls(ui, p, idle);
        ui.add_space(4.0);
        // The list takes the height the editor below leaves. The editor's height depends on
        // the selection, so it is measured: when it changed, the frame is laid out again before
        // it is shown, so the list never shows at the wrong height for a frame.
        let top = ui.cursor().top();
        let table_h = (ui.available_height() - self.scenario_layout.below).max(80.0);
        let list = Frame::new()
            .fill(p.field)
            .stroke(Stroke::new(1.0, p.border))
            .show(ui, |ui| {
                // The whole width, whatever the content: an empty list holds one short line.
                ui.set_min_width(ui.available_width());
                ui.set_min_height(table_h);
                ui.set_max_height(table_h);
                self.rows(ui, p, idle);
            })
            .response
            .rect;
        ui.add_space(6.0);
        // The editor's own rect: the panel's area always reaches the panel's bottom.
        let content_bottom = ui.scope(|ui| self.row_editor(ui, p)).response.rect.bottom();
        let below = content_bottom - top - table_h;
        if (below - self.scenario_layout.below).abs() > 0.5 {
            ui.ctx().request_discard("scenario editor height");
        }
        self.scenario_layout = super::ScenarioLayout {
            below,
            list,
            panel: ui.max_rect(),
            content_bottom,
        };
    }

    fn run_controls(&mut self, ui: &mut Ui, p: &Palette, idle: bool) {
        let has = !self.scenario.commands.is_empty();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(idle && has, egui::Button::new("Run"))
                .on_hover_text("Run the whole script (F5)")
                .clicked()
            {
                self.start_run();
            }
            if ui
                .add_enabled(idle && has, egui::Button::new("Step"))
                .on_hover_text("Send the selected command and select the next (F6)")
                .clicked()
            {
                self.step_selected();
            }
            if ui
                .add_enabled(self.running, egui::Button::new("Stop"))
                .on_hover_text("Esc")
                .clicked()
            {
                self.stop_requested = true;
            }
            ui.label(RichText::new("Iterations").color(p.weak));
            ui.add_enabled(
                idle,
                egui::TextEdit::singleline(&mut self.repeat_count)
                    .desired_width(40.0)
                    .horizontal_align(Align::RIGHT),
            );
            self.repeat_count.retain(|c| c.is_ascii_digit());
            self.repeat_count.truncate(6);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let count = |m: Mark| {
                    self.row_results
                        .iter()
                        .flatten()
                        .filter(|r| r.mark == m)
                        .count()
                };
                let (ok, warn, ko) = (
                    count(Mark::Passed),
                    count(Mark::Warning),
                    count(Mark::Failed),
                );
                if ok + warn + ko > 0 {
                    ui.label(RichText::new(format!("{ko} failed")).color(if ko > 0 {
                        p.err
                    } else {
                        p.weak
                    }));
                    if warn > 0 {
                        let s = if warn == 1 { "" } else { "s" };
                        ui.label(RichText::new(format!("{warn} warning{s}")).color(p.warn));
                    }
                    ui.label(RichText::new(format!("{ok} passed")).color(p.ok));
                }
            });
        });
    }

    /// The scenario's commands, two lines each: the command and its expected answer and last
    /// result; then the description.
    fn rows(&mut self, ui: &mut Ui, p: &Palette, idle: bool) {
        let font = FontId::monospace(12.5);
        let small = FontId::proportional(11.5);
        let line_h = ui.fonts_mut(|f| f.row_height(&font));
        let row_h = line_h * 2.0 + 6.0;
        let n = self.scenario.commands.len();
        if n == 0 {
            ui.add_space(8.0);
            ui.label(RichText::new("  Open a script (.apdu), or Add a command.").color(p.weak));
            return;
        }
        let running_row = if self.running { Some(self.index) } else { None };
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .id_salt("rows")
            .auto_shrink([false, false])
            .show_rows(ui, row_h, n, |ui, range| {
                for i in range {
                    let Some(c) = self.scenario.commands.get(i) else {
                        break;
                    };
                    let (rect, resp) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), row_h),
                        Sense::click(),
                    );
                    let bg = if i == self.index {
                        p.selection
                    } else if i % 2 == 1 {
                        p.stripe
                    } else {
                        p.field
                    };
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(rect, 0.0, bg);
                    if running_row == Some(i) {
                        painter.rect_filled(
                            Rect::from_min_size(rect.min, Vec2::new(3.0, row_h)),
                            0.0,
                            p.accent,
                        );
                    }
                    let x0 = rect.left() + 8.0;
                    let y1 = rect.top() + 3.0;
                    let y2 = y1 + line_h;
                    painter.text(
                        egui::pos2(x0, y1),
                        Align2::LEFT_TOP,
                        format!("{:3}", i + 1),
                        font.clone(),
                        p.weak,
                    );
                    // Right side first (the last result, the expected answer); the command and
                    // the description take what is left beside them, ending with "…" if longer.
                    let right = rect.right() - 8.0;
                    let mut line1_end = right;
                    let mut line2_end = right;
                    if let Some(Some(r)) = self.row_results.get(i) {
                        let (mark, color) = match r.mark {
                            Mark::Passed => ("ok", p.ok),
                            Mark::Warning => ("warn", p.warn),
                            Mark::Failed => ("fail", p.err),
                        };
                        let used = painter.text(
                            egui::pos2(right, y1),
                            Align2::RIGHT_TOP,
                            format!("{mark} {}", r.short),
                            font.clone(),
                            color,
                        );
                        line1_end = used.left() - 12.0;
                    }
                    if let Some(e) = &c.expected {
                        let used = painter.text(
                            egui::pos2(right, y2),
                            Align2::RIGHT_TOP,
                            format!("expect {e}"),
                            small.clone(),
                            p.weak,
                        );
                        line2_end = used.left() - 12.0;
                    }
                    let mut left = x0 + 36.0;
                    if c.target == Target::Reader {
                        // A CCID escape command to the reader, not an APDU to the card.
                        let used = painter.text(
                            egui::pos2(left, y1),
                            Align2::LEFT_TOP,
                            "reader",
                            font.clone(),
                            p.accent,
                        );
                        left = used.right() + 8.0;
                    }
                    let cmd_color = if c.incomplete { p.warn } else { p.text };
                    elided(
                        &painter,
                        egui::pos2(left, y1),
                        &c.text,
                        font.clone(),
                        cmd_color,
                        line1_end - left,
                    );
                    let left = x0 + 36.0;
                    elided(
                        &painter,
                        egui::pos2(left, y2),
                        c.description.trim(),
                        small.clone(),
                        p.weak,
                        line2_end - left,
                    );
                    if resp.clicked() && idle {
                        clicked = Some(i);
                    }
                    let description = c.description.trim();
                    if description.is_empty() {
                        resp.on_hover_text(c.text.as_str());
                    } else {
                        resp.on_hover_text(format!("{}\n{description}", c.text));
                    }
                }
            });
        if let Some(i) = clicked {
            self.set_index(i);
        }
    }

    /// The selected row's fields, editable, and the row operations.
    fn row_editor(&mut self, ui: &mut Ui, p: &Palette) {
        let editable = !self.running;
        let i = self.index;
        let has_row = i < self.scenario.commands.len();
        // Measured outside the grid: inside, the cell's width isn't known yet.
        let w = (ui.available_width() - 90.0).max(120.0);
        ui.add_enabled_ui(editable, |ui| {
            egui::Grid::new("row_editor")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    let mut edited = (false, false);
                    if let Some(c) = self.scenario.commands.get_mut(i) {
                        // Where it goes: the card (an APDU) or the reader (a CCID escape).
                        let before = c.target;
                        egui::ComboBox::from_id_salt("row_target")
                            .width(70.0)
                            .selected_text(match c.target {
                                Target::Card => "Card",
                                Target::Reader => "Reader",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut c.target, Target::Card, "Card")
                                    .on_hover_text("An APDU, sent to the card (SCardTransmit)");
                                ui.selectable_value(&mut c.target, Target::Reader, "Reader")
                                    .on_hover_text(
                                        "A CCID escape command, sent to the reader (SCardControl)",
                                    );
                            });
                        edited.0 |= c.target != before;
                        edited.0 |= ui
                            .add(
                                egui::TextEdit::singleline(&mut c.text)
                                    .font(TextStyle::Monospace)
                                    .desired_width(w),
                            )
                            .changed();
                        ui.end_row();
                        ui.label(RichText::new("Expected").color(p.weak));
                        let mut exp = c.expected.clone().unwrap_or_default();
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut exp)
                                    .font(TextStyle::Monospace)
                                    .hint_text("e.g. 90 00, 61 ??, 6A *")
                                    .desired_width(w),
                            )
                            .changed()
                        {
                            let t = exp.trim();
                            c.expected = (!t.is_empty()).then(|| t.to_string());
                            edited.1 = true;
                        }
                        ui.end_row();
                        ui.label(RichText::new("Step").color(p.weak));
                        edited.1 |= ui
                            .add(egui::TextEdit::singleline(&mut c.description).desired_width(w))
                            .changed();
                        ui.end_row();
                    } else {
                        ui.label(RichText::new("No command selected").color(p.weak));
                        ui.end_row();
                    }
                    if edited.0 {
                        self.command_edited(i);
                    } else if edited.1 {
                        self.row_edited(i);
                    }
                });
            ui.horizontal(|ui| {
                if ui
                    .button("Add")
                    .on_hover_text("A new row after this one")
                    .clicked()
                {
                    self.add_row();
                }
                if ui
                    .add_enabled(has_row, egui::Button::new("Delete"))
                    .clicked()
                {
                    self.delete_row();
                }
                if ui
                    .add_enabled(has_row && i > 0, egui::Button::new("Up"))
                    .clicked()
                {
                    self.move_row(true);
                }
                if ui
                    .add_enabled(
                        i + 1 < self.scenario.commands.len(),
                        egui::Button::new("Down"),
                    )
                    .clicked()
                {
                    self.move_row(false);
                }
            });
        });
    }

    // ----------------------------------------------------------------- exchanges

    fn exchanges(&mut self, ui: &mut Ui, p: &Palette) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Exchanges").strong());
            let total = self.transcript.len();
            let count = if self.filter.is_active() {
                let shown = self
                    .transcript
                    .entries()
                    .filter(|e| self.filter.matches(e))
                    .count();
                format!("{shown} of {total}")
            } else {
                format!("{total}")
            };
            ui.label(RichText::new(count).color(p.weak));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Clear").clicked() {
                    self.transcript.clear();
                }
                if ui
                    .button("Copy")
                    .on_hover_text("Copy the exchanges shown, as text")
                    .clicked()
                {
                    ui.ctx().copy_text(self.transcript.to_text_of(&self.filter));
                }
                ui.checkbox(&mut self.filter.problems_only, "Problems only")
                    .on_hover_text(
                        "Failed exchanges, unexpected answers, status words other than 9000 \
                         and 61xx, warnings and errors",
                    );
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter.text)
                        .hint_text("Find: hex or text")
                        .desired_width(150.0),
                )
                .on_hover_text(
                    "Shows the exchanges and notes holding this (spaces and case don't count)",
                );
            });
        });
        ui.add_space(2.0);
        let selected = self
            .transcript
            .selected
            .and_then(|id| self.transcript.entries().find(|e| e.id == id))
            .cloned();
        if let Some(e) = selected {
            egui::Panel::bottom("detail")
                .resizable(true)
                .default_size(250.0)
                .min_size(110.0)
                .frame(
                    Frame::new()
                        .fill(p.panel)
                        .inner_margin(Margin::same(8))
                        .stroke(Stroke::new(1.0, p.border)),
                )
                .show(ui, |ui| self.detail(ui, p, &e));
        }
        self.exchange_list(ui, p);
    }

    /// The entries, drawn only where visible (a session may hold thousands).
    fn exchange_list(&mut self, ui: &mut Ui, p: &Palette) {
        let font = FontId::monospace(12.5);
        let line_h = ui.fonts_mut(|f| f.row_height(&font));
        let gap = 6.0;
        let filter = self.filter.clone();
        let shown: Vec<&Entry> = self
            .transcript
            .entries()
            .filter(|e| filter.matches(e))
            .collect();
        let heights: Vec<f32> = shown
            .iter()
            .map(|e| entry_lines(e) as f32 * line_h + gap)
            .collect();
        let total: f32 = heights.iter().sum();
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .id_salt("exchanges")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show_viewport(ui, |ui, viewport| {
                ui.set_height(total);
                let origin = ui.min_rect().min;
                let width = ui.available_width();
                let mut y = 0.0;
                for (e, h) in shown.iter().copied().zip(heights.iter().copied()) {
                    if y + h < viewport.min.y {
                        y += h;
                        continue;
                    }
                    if y > viewport.max.y {
                        break;
                    }
                    let rect = Rect::from_min_size(origin + Vec2::new(0.0, y), Vec2::new(width, h));
                    // A note with nothing to decode would show its own text again: not selectable.
                    let sense = if e.body.has_detail() {
                        Sense::click()
                    } else {
                        Sense::hover()
                    };
                    let resp = ui.interact(rect, ui.id().with(("entry", e.id)), sense);
                    if self.transcript.selected == Some(e.id) {
                        ui.painter().rect_filled(rect, 2.0, p.selection);
                    } else if resp.hovered() && e.body.has_detail() {
                        ui.painter().rect_filled(rect, 2.0, p.stripe);
                    }
                    paint_entry(ui, e, rect, &font, line_h, p);
                    if resp.clicked() {
                        clicked = Some(e.id);
                    }
                    y += h;
                }
            });
        if let Some(id) = clicked {
            self.transcript.selected = if self.transcript.selected == Some(id) {
                None
            } else {
                Some(id)
            };
        }
    }

    // ----------------------------------------------------------------- console

    fn console(&mut self, ui: &mut Ui, p: &Palette) {
        let idle = self.idle();
        let connected = self.card_open;
        ui.horizontal(|ui| {
            ui.label(mono(">").color(p.sent));
            let buttons_w = 290.0;
            let te = ui.add_enabled(
                !self.running,
                egui::TextEdit::singleline(&mut self.command_text)
                    .font(TextStyle::Monospace)
                    .hint_text("APDU in hex, e.g. 00 A4 04 00 07 A0000000041010")
                    .desired_width((ui.available_width() - buttons_w).max(200.0)),
            );
            if te.changed() {
                self.history_pos = None;
            }
            if te.has_focus() {
                let (up, down) = ui.input_mut(|i| {
                    (
                        i.consume_key(Modifiers::NONE, Key::ArrowUp),
                        i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    )
                });
                if up {
                    self.history_back();
                }
                if down {
                    self.history_forward();
                }
            }
            let enter = te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            let send = ui
                .add_enabled(idle && connected, egui::Button::new("Transmit"))
                .on_hover_text("SCardTransmit (Enter)")
                .clicked();
            if (enter && idle) || send {
                self.send_command_line();
                te.request_focus();
            }
            let mut follow = self.settings.follow_up;
            if ui
                .add_enabled(!self.running, egui::Checkbox::new(&mut follow, "Follow 61xx / 6Cxx"))
                .on_hover_text(
                    "After 61xx, send GET RESPONSE; after 6Cxx, send the command again with that Le \
                     (ISO/IEC 7816-4). Every command sent is shown; a scenario's expected answer is \
                     compared with the whole answer.",
                )
                .changed()
            {
                self.set_follow_up(follow);
            }
        });
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("CCID escape").color(p.weak));
            ui.add_enabled(
                idle && connected,
                egui::TextEdit::singleline(&mut self.escape_text)
                    .font(TextStyle::Monospace)
                    .desired_width(220.0),
            );
            let hist = self.escape_history.clone();
            ui.add_enabled_ui(idle && connected, |ui| {
                egui::ComboBox::from_id_salt("escape_hist")
                    .width(24.0)
                    .selected_text("")
                    .show_ui(ui, |ui| {
                        for h in &hist {
                            if ui.selectable_label(false, mono(h.as_str())).clicked() {
                                self.escape_text = h.clone();
                            }
                        }
                    });
            });
            if ui
                .add_enabled(idle && connected, egui::Button::new("Execute"))
                .clicked()
            {
                self.do_escape();
            }
            ui.separator();
            let logging = self.log.is_some();
            dot(ui, if logging { p.ok } else { p.weak });
            let label = if logging { "Stop log" } else { "Start log" };
            if ui.add_enabled(idle, egui::Button::new(label)).clicked() {
                self.do_toggle_log();
            }
            ui.label(RichText::new(self.log_path.display().to_string()).color(p.weak))
                .on_hover_text("Where Start log writes");
        });
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            let busy = self.worker.busy() || self.running;
            if busy {
                dot(ui, p.accent);
            }
            ui.add(
                egui::Label::new(RichText::new(self.status.replace('\n', " ")).color(p.weak))
                    .truncate(),
            )
            .on_hover_text(self.status.as_str());
        });
    }

    pub(super) fn history_back(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let i = match self.history_pos {
            None => self.history.len() - 1,
            Some(i) => i.saturating_sub(1),
        };
        if let Some(t) = self.history.get(i) {
            self.command_text = t.clone();
            self.history_pos = Some(i);
        }
    }

    pub(super) fn history_forward(&mut self) {
        match self.history_pos {
            Some(i) if i + 1 < self.history.len() => {
                if let Some(t) = self.history.get(i + 1) {
                    self.command_text = t.clone();
                    self.history_pos = Some(i + 1);
                }
            }
            Some(_) => {
                self.command_text.clear();
                self.history_pos = None;
            }
            None => {}
        }
    }
}

/// One entry of the list: time, then what was sent and what came back, or the note.
fn paint_entry(ui: &Ui, e: &Entry, rect: Rect, font: &FontId, line_h: f32, p: &Palette) {
    let painter = ui.painter_at(rect);
    let x = rect.left() + 6.0;
    let y = rect.top() + 3.0;
    let time = clock(e.at);
    let indent = " ".repeat(time.len() + 2);
    match &e.body {
        Body::Note { level, text, .. } => {
            let color = match level {
                Level::Info => p.text,
                Level::Warning => p.warn,
                Level::Error => p.err,
            };
            for (k, l) in text.lines().enumerate() {
                let lead = if k == 0 {
                    format!("{time}  ")
                } else {
                    indent.clone()
                };
                let job = line_job(&[(&lead, p.weak), (l, color)], font);
                let g = ui.fonts_mut(|f| f.layout_job(job));
                painter.galley(egui::pos2(x, y + k as f32 * line_h), g, p.text);
            }
        }
        Body::Exchange {
            channel,
            sent,
            reply,
            took,
            verdict,
        } => {
            let sent_hex = list_hex(sent);
            let tag = format!("   {}", channel_label(*channel));
            let job = line_job(
                &[
                    (&format!("{time}  "), p.weak),
                    ("> ", p.weak),
                    (&sent_hex, p.sent),
                    (&tag, p.weak),
                ],
                font,
            );
            let g = ui.fonts_mut(|f| f.layout_job(job));
            painter.galley(egui::pos2(x, y), g, p.text);

            let (got, got_color) = match reply {
                Ok(r) => (list_hex(r), p.text),
                Err(why) => (why.clone(), p.err),
            };
            let ms = took
                .map(|d| format!("   {} ms", d.as_millis()))
                .unwrap_or_default();
            let (check, check_color) = match verdict {
                Verdict::Unchecked => (String::new(), p.weak),
                Verdict::Passed => ("   ok".to_string(), p.ok),
                Verdict::Failed(why) => (format!("   FAIL  {}", why.replace('\n', " ")), p.err),
            };
            // The status word's meaning, when the check doesn't already say why it failed. With
            // nothing to compare it with, it takes its category's colour: amber for a warning,
            // red for an error. An answer that matched the expected one stays plain.
            let meaning = match (reply, verdict) {
                (Ok(r), Verdict::Unchecked | Verdict::Passed) => iso::sw_of(r)
                    .and_then(|(_, _, m)| m)
                    .map(|m| format!("   · {m}"))
                    .unwrap_or_default(),
                _ => String::new(),
            };
            let meaning_color = match (reply, verdict) {
                (Ok(r), Verdict::Unchecked) => match iso::level_of(r) {
                    Some(iso::SwLevel::Warning) => p.warn,
                    Some(iso::SwLevel::Error) => p.err,
                    _ => p.weak,
                },
                _ => p.weak,
            };
            let job = line_job(
                &[
                    (&indent, p.weak),
                    ("< ", p.weak),
                    (&got, got_color),
                    (&ms, p.weak),
                    (&check, check_color),
                    (&meaning, meaning_color),
                ],
                font,
            );
            let g = ui.fonts_mut(|f| f.layout_job(job));
            painter.galley(egui::pos2(x, y + line_h), g, p.text);
        }
    }
}
