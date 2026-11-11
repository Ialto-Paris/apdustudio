// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! The detail of the selected entry, decoded by the standards that define the bytes: an
//! exchange's command (ISO/IEC 7816-4 header and case, or PC/SC part 3 for class FF) and its
//! answer (status word category and meaning), with hex dumps; a note's ATRs (ISO/IEC 7816-3,
//! historical bytes per ISO/IEC 7816-4, contactless cards per PC/SC part 3). See `decode`.

use egui::{Align, Color32, Layout, RichText, TextStyle, Ui};

use super::transcript::{channel_label, clock, hex, Body, Channel, Entry, Verdict};
use super::widgets::Palette;
use super::ApduStudio;
use crate::decode::{self, Field};
use crate::iso;

/// Most characters of bytes shown in the first column (8 bytes); the rest is in its tooltip.
const BYTES_SHOWN: usize = 3 * 8 - 1;

impl ApduStudio {
    /// The detail panel's content for entry `e`.
    pub(super) fn detail(&mut self, ui: &mut Ui, p: &Palette, e: &Entry) {
        let mut close = false;
        let mut use_command = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Detail").strong());
            ui.label(RichText::new(clock(e.at)).color(p.weak).monospace());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    close = true;
                }
                if ui
                    .button("Copy detail")
                    .on_hover_text("Everything decoded here, as text")
                    .clicked()
                {
                    ui.ctx().copy_text(detail_text(e));
                }
                if let Body::Exchange { sent, reply, .. } = &e.body {
                    if let Ok(r) = reply {
                        if ui
                            .button("Copy answer")
                            .on_hover_text("The answer's bytes, as hex")
                            .clicked()
                        {
                            ui.ctx().copy_text(hex(r).replace(' ', ""));
                        }
                    }
                    if ui
                        .add_enabled(!self.running, egui::Button::new("To command line"))
                        .on_hover_text("Put this command in the command line")
                        .clicked()
                    {
                        use_command = Some(hex(sent).replace(' ', ""));
                    }
                }
            });
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("detail")
            .auto_shrink([false, false])
            .show(ui, |ui| match &e.body {
                Body::Note { atrs, answer, .. } => {
                    for (k, (whose, atr)) in atrs.iter().enumerate() {
                        if k > 0 {
                            ui.add_space(8.0);
                        }
                        let title = if whose.is_empty() {
                            "ATR".to_string()
                        } else {
                            format!("ATR of {whose}")
                        };
                        ui.label(RichText::new(title).strong());
                        ui.add(
                            egui::Label::new(RichText::new(hex(atr)).monospace().color(p.text))
                                .selectable(true),
                        );
                        ui.add_space(2.0);
                        fields(ui, p, ("atr", e.id, k), &decode::atr(atr), p.text);
                    }
                    if let Some(r) = answer {
                        ui.label(RichText::new("Whole answer").strong());
                        fields(ui, p, ("whole", e.id), &decode::response(r), p.text);
                        ui.add_space(2.0);
                        dump(
                            ui,
                            r.get(..r.len().saturating_sub(2)).unwrap_or_default(),
                            p.text,
                        );
                    }
                }
                Body::Exchange {
                    channel,
                    sent,
                    reply,
                    took,
                    verdict,
                } => {
                    let took = took
                        .map(|d| format!("{} ms", d.as_millis()))
                        .unwrap_or_else(|| "-".into());
                    ui.label(
                        RichText::new(format!("{}  ·  {took}", channel_label(*channel)))
                            .color(p.weak),
                    );
                    ui.add_space(4.0);
                    // A CCID escape goes to the reader in its maker's format: nothing to decode.
                    let iso = *channel == Channel::Card;
                    ui.label(RichText::new("Command").strong());
                    if iso {
                        fields(ui, p, ("command", e.id), &decode::command(sent), p.sent);
                        ui.add_space(2.0);
                    }
                    dump(ui, sent, p.sent);
                    ui.add_space(6.0);
                    ui.label(RichText::new("Answer").strong());
                    match reply {
                        Ok(r) => {
                            if iso {
                                fields(ui, p, ("answer", e.id), &decode::response(r), p.text);
                                ui.add_space(2.0);
                            }
                            let data = if iso {
                                r.get(..r.len().saturating_sub(2)).unwrap_or_default()
                            } else {
                                r.as_slice()
                            };
                            if data.is_empty() {
                                ui.label(RichText::new("no data").color(p.weak));
                            } else {
                                dump(ui, data, p.text);
                            }
                        }
                        Err(why) => {
                            ui.label(RichText::new(why).color(p.err));
                        }
                    }
                    match verdict {
                        Verdict::Unchecked => {}
                        Verdict::Passed => {
                            ui.add_space(6.0);
                            ui.label(RichText::new("As expected").color(p.ok));
                        }
                        Verdict::Failed(why) => {
                            ui.add_space(6.0);
                            ui.label(RichText::new(why).color(p.err));
                        }
                    }
                }
            });
        if close {
            self.transcript.selected = None;
        }
        if let Some(c) = use_command {
            self.command_text = c;
            self.history_pos = None;
        }
    }
}

/// What the detail shows for `e`, as text: each section's title, its bytes and its decoded
/// fields (`decode::to_text`), in the order the panel shows them.
fn detail_text(e: &Entry) -> String {
    fn section(s: &mut String, title: &str, bytes: &[u8], fs: Option<Vec<Field>>) {
        if !s.is_empty() {
            s.push('\n');
        }
        s.push_str(&format!("{title}: {}\n", hex(bytes)));
        if let Some(fs) = fs {
            s.push_str(&decode::to_text(&fs));
        }
    }
    let mut out = String::new();
    let s = &mut out;
    match &e.body {
        Body::Note { atrs, answer, .. } => {
            for (whose, atr) in atrs {
                let title = if whose.is_empty() {
                    "ATR".to_string()
                } else {
                    format!("ATR of {whose}")
                };
                section(s, &title, atr, Some(decode::atr(atr)));
            }
            if let Some(r) = answer {
                section(s, "Whole answer", r, Some(decode::response(r)));
            }
        }
        Body::Exchange {
            channel,
            sent,
            reply,
            verdict,
            ..
        } => {
            let iso = *channel == Channel::Card;
            section(s, "Command", sent, iso.then(|| decode::command(sent)));
            match reply {
                Ok(r) => section(s, "Answer", r, iso.then(|| decode::response(r))),
                Err(why) => s.push_str(&format!("\nAnswer: {why}\n")),
            }
            if let Verdict::Failed(why) = verdict {
                s.push_str(&format!("\n{why}\n"));
            }
        }
    }
    out
}

/// Decoded fields as a table: bytes, name (its standard's clause on hover), meaning.
fn fields(ui: &mut Ui, p: &Palette, id: impl egui::AsIdSalt, fs: &[Field], bytes: Color32) {
    egui::Grid::new(id)
        .num_columns(3)
        .spacing([14.0, 3.0])
        .striped(true)
        .show(ui, |ui| {
            for f in fs {
                row(ui, p, f, 0, bytes);
            }
        });
}

fn row(ui: &mut Ui, p: &Palette, f: &Field, depth: usize, bytes: Color32) {
    let shown = shorten(&f.bytes);
    let b = ui.label(RichText::new(&shown).monospace().color(bytes));
    if shown != f.bytes {
        b.on_hover_text(&f.bytes);
    }
    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 14.0);
        ui.label(RichText::new(&f.name).strong())
            .on_hover_text(f.reference);
    });
    ui.add(
        egui::Label::new(RichText::new(&f.meaning).color(meaning_color(p, f)))
            .wrap()
            .selectable(true),
    );
    ui.end_row();
    // Fields nest at most three deep (historical bytes, an object, its parts).
    if depth < 4 {
        for c in &f.children {
            row(ui, p, c, depth + 1, bytes);
        }
    }
}

fn meaning_color(p: &Palette, f: &Field) -> Color32 {
    if f.problem {
        p.err
    } else if f.name == decode::SW && !f.meaning.starts_with("Normal") {
        p.warn
    } else {
        p.text
    }
}

/// The first bytes of a long run, with "…".
fn shorten(bytes: &str) -> String {
    match bytes.get(..BYTES_SHOWN) {
        Some(head) if bytes.len() > BYTES_SHOWN => format!("{head} …"),
        _ => bytes.to_string(),
    }
}

fn dump(ui: &mut Ui, bytes: &[u8], color: egui::Color32) {
    let text = iso::hexdump(bytes);
    ui.add(
        egui::Label::new(
            RichText::new(text.trim_end())
                .text_style(TextStyle::Monospace)
                .color(color),
        )
        .selectable(true),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_byte_runs_are_shortened_on_a_byte_boundary() {
        assert_eq!(shorten("3B 88"), "3B 88");
        let long = vec!["AA"; 20].join(" ");
        let s = shorten(&long);
        assert!(s.ends_with("AA …"), "{s}");
        assert_eq!(s.matches("AA").count(), 8);
    }

    fn entry(body: Body) -> Entry {
        Entry {
            id: 1,
            at: std::time::Duration::ZERO,
            body,
        }
    }

    #[test]
    fn the_copied_detail_of_an_atr_is_its_decoded_fields() {
        let atr = crate::demo::ATR.to_vec();
        let e = entry(Body::Note {
            level: super::super::transcript::Level::Info,
            text: "Connected".into(),
            atrs: vec![("Demo Reader 0".into(), atr.clone())],
            answer: None,
        });
        let text = detail_text(&e);
        let mut lines = text.lines();
        assert_eq!(
            lines.next(),
            Some(format!("ATR of Demo Reader 0: {}", hex(&atr)).as_str())
        );
        assert_eq!(
            &text[text.find('\n').unwrap_or(0) + 1..],
            decode::to_text(&decode::atr(&atr))
        );
        let ts = lines.next().unwrap_or_default();
        assert!(ts.starts_with("3B "), "{ts}");
        assert!(ts.contains("TS") && ts.contains("[ISO/IEC 7816-3"), "{ts}");
        // Sub-fields are indented under their parent.
        assert!(
            text.lines()
                .any(|l| l.contains("    ") && !l.starts_with(' ')),
            "{text}"
        );
    }

    #[test]
    fn the_copied_detail_of_an_exchange_has_both_sides_and_the_failure() {
        let e = entry(Body::Exchange {
            channel: Channel::Card,
            sent: vec![0x00, 0xA4, 0x00, 0x00, 0x02, 0x3F, 0x00],
            reply: Ok(vec![0x6A, 0x82]),
            took: None,
            verdict: Verdict::Failed("Compare error!".into()),
        });
        let text = detail_text(&e);
        assert!(
            text.starts_with("Command: 00 A4 00 00 02 3F 00\n"),
            "{text}"
        );
        assert!(text.contains("\nAnswer: 6A 82\n"), "{text}");
        assert!(text.contains(decode::SW), "{text}");
        assert!(text.ends_with("\nCompare error!\n"), "{text}");
        // An escape to the reader is only its bytes.
        let escape = entry(Body::Exchange {
            channel: Channel::Escape,
            sent: vec![0x01, 0x02],
            reply: Err("no answer".into()),
            took: None,
            verdict: Verdict::Unchecked,
        });
        assert_eq!(
            detail_text(&escape),
            "Command: 01 02\n\nAnswer: no answer\n"
        );
    }
}
