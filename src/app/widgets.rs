// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! The theme (one palette for light, one for dark, following the system) and small drawing
//! helpers. Colour carries meaning only: state (passed, failed, warning) and the selection;
//! everything else is neutral.

use egui::{
    Color32, CornerRadius, FontId, Pos2, Rect, RichText, Stroke, TextStyle, Theme, Ui, Visuals,
};

/// The colours the views draw with. Contrast against `bg`/`panel`/`field` is at least 4.5:1 for
/// text and marks (tested below).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Palette {
    /// Window background, between panels.
    pub(super) bg: Color32,
    /// Panels and bars.
    pub(super) panel: Color32,
    /// Text fields, tables and the exchanges.
    pub(super) field: Color32,
    /// Every other row of a table.
    pub(super) stripe: Color32,
    pub(super) border: Color32,
    pub(super) text: Color32,
    /// Labels, times and other secondary text.
    pub(super) weak: Color32,
    pub(super) accent: Color32,
    pub(super) selection: Color32,
    pub(super) ok: Color32,
    pub(super) warn: Color32,
    pub(super) err: Color32,
    /// Bytes sent (bytes received use `text`).
    pub(super) sent: Color32,
}

/// Panels sit a step darker than the white fields they hold, so content stands out from the
/// frame around it; borders and secondary text are firm enough to read at a glance.
pub(super) const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xD5, 0xDA, 0xE0),
    panel: Color32::from_rgb(0xE9, 0xEC, 0xF0),
    field: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    stripe: Color32::from_rgb(0xF1, 0xF3, 0xF6),
    border: Color32::from_rgb(0xA9, 0xB1, 0xBC),
    text: Color32::from_rgb(0x14, 0x18, 0x1D),
    weak: Color32::from_rgb(0x46, 0x50, 0x5C),
    accent: Color32::from_rgb(0x07, 0x56, 0xBC),
    selection: Color32::from_rgb(0xCA, 0xDD, 0xF8),
    ok: Color32::from_rgb(0x13, 0x65, 0x2B),
    warn: Color32::from_rgb(0x6E, 0x48, 0x00),
    err: Color32::from_rgb(0xA3, 0x19, 0x2A),
    sent: Color32::from_rgb(0x0A, 0x4A, 0x9E),
};

pub(super) const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x12, 0x15, 0x19),
    panel: Color32::from_rgb(0x1B, 0x1F, 0x24),
    field: Color32::from_rgb(0x0E, 0x11, 0x14),
    stripe: Color32::from_rgb(0x16, 0x1A, 0x1F),
    border: Color32::from_rgb(0x3A, 0x41, 0x4A),
    text: Color32::from_rgb(0xE3, 0xE7, 0xEC),
    weak: Color32::from_rgb(0x9A, 0xA3, 0xAE),
    accent: Color32::from_rgb(0x6E, 0xB0, 0xFF),
    selection: Color32::from_rgb(0x1E, 0x3A, 0x5F),
    ok: Color32::from_rgb(0x56, 0xC9, 0x76),
    warn: Color32::from_rgb(0xDE, 0xAB, 0x36),
    err: Color32::from_rgb(0xFF, 0x7B, 0x7B),
    sent: Color32::from_rgb(0x8C, 0xC0, 0xFF),
};

impl Palette {
    pub(super) fn for_dark(dark: bool) -> Palette {
        if dark {
            DARK
        } else {
            LIGHT
        }
    }

    /// The palette of the theme in use.
    pub(super) fn of(ctx: &egui::Context) -> Palette {
        Palette::for_dark(ctx.global_style().visuals.dark_mode)
    }

    pub(super) fn of_ui(ui: &Ui) -> Palette {
        Palette::for_dark(ui.visuals().dark_mode)
    }
}

fn visuals(p: &Palette, dark: bool) -> Visuals {
    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.override_text_color = Some(p.text);
    v.weak_text_color = Some(p.weak);
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(6);
    v.extreme_bg_color = p.field;
    v.text_edit_bg_color = Some(p.field);
    v.faint_bg_color = p.stripe;
    v.code_bg_color = p.stripe;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.err;
    v.selection.bg_fill = p.selection;
    v.selection.stroke = Stroke::new(1.0, p.text);
    let (button, hover) = if dark {
        (
            Color32::from_rgb(0x2A, 0x30, 0x37),
            Color32::from_rgb(0x34, 0x3B, 0x44),
        )
    } else {
        (
            Color32::from_rgb(0xDD, 0xE1, 0xE7),
            Color32::from_rgb(0xCE, 0xD4, 0xDC),
        )
    };
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.bg_fill = p.panel;
    w.inactive.weak_bg_fill = button;
    w.inactive.bg_fill = button;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.weak_bg_fill = hover;
    w.hovered.bg_fill = hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.active.weak_bg_fill = p.selection;
    w.active.bg_fill = p.selection;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.open.weak_bg_fill = p.selection;
    w.open.bg_stroke = Stroke::new(1.0, p.accent);
    for s in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        s.corner_radius = CornerRadius::same(3);
    }
    v
}

/// The interface's font: Ubuntu Regular (assets/fonts, Ubuntu Font Licence). egui's own is
/// Ubuntu Light, whose thin strokes made every label look pale.
const UI_FONT: &[u8] = include_bytes!("../../assets/fonts/Ubuntu-Regular.ttf");

/// Puts Ubuntu Regular first for proportional text; egui's fonts stay as fallbacks (symbols,
/// emoji) and for monospace.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "Ubuntu-Regular".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(UI_FONT)),
    );
    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        family.insert(0, "Ubuntu-Regular".to_owned());
    }
    ctx.set_fonts(fonts);
}

/// Sets the fonts and their sizes and both themes; the window follows the system's light or dark
/// mode.
pub(super) fn apply_style(ctx: &egui::Context) {
    install_fonts(ctx);
    for (theme, palette, dark) in [(Theme::Light, &LIGHT, false), (Theme::Dark, &DARK, true)] {
        ctx.style_mut_of(theme, |s| {
            s.text_styles
                .insert(TextStyle::Body, FontId::proportional(13.0));
            s.text_styles
                .insert(TextStyle::Button, FontId::proportional(13.0));
            s.text_styles
                .insert(TextStyle::Monospace, FontId::monospace(13.0));
            s.text_styles
                .insert(TextStyle::Small, FontId::proportional(11.0));
            s.spacing.item_spacing = egui::vec2(6.0, 4.0);
            s.spacing.button_padding = egui::vec2(8.0, 3.0);
            s.spacing.interact_size.y = 22.0;
            s.visuals = visuals(palette, dark);
        });
    }
    ctx.set_theme(egui::ThemePreference::System);
}

/// A small filled circle, for the connection state.
pub(super) fn dot(ui: &mut Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    ui.painter().circle_filled(r.center(), 4.5, color);
}

/// A PNG as a texture, made the first time and kept in the context's memory under `name`;
/// None if it can't be read.
fn png_texture(ui: &Ui, name: &str, png: &[u8]) -> Option<egui::TextureHandle> {
    let id = egui::Id::new(name);
    let ctx = ui.ctx();
    ctx.data(|d| d.get_temp::<egui::TextureHandle>(id))
        .or_else(|| {
            let (px, w, h) = crate::icon::decode(png)?;
            let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px);
            let options =
                egui::TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear));
            let t = ctx.load_texture(name, image, options);
            ctx.data_mut(|d| d.insert_temp(id, t.clone()));
            Some(t)
        })
}

/// The application's icon (`assets/icon-256.png`), drawn in `r`; nothing if it can't be read.
pub(super) fn app_icon(ui: &Ui, r: Rect) {
    if let Some(t) = png_texture(ui, "app-icon", crate::icon::PNG) {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        ui.painter().image(t.id(), r, uv, Color32::WHITE);
    }
}

/// The logo's height in About, in points.
const IALTO_LOGO_HEIGHT: f32 = 30.0;

/// The room a line of text keeps under its letters, for descenders: about a fifth of its
/// height. Used to line letters up with a picture, which has no such room.
fn descent(line_height: f32) -> f32 {
    line_height * 0.2
}

/// Lines of text beside a picture `height` points high, their letters centred on it: lines of
/// `styles`, added by `add` in a column. The column's own spacing is used between the lines.
pub(super) fn centred_beside(
    ui: &mut Ui,
    height: f32,
    styles: &[egui::TextStyle],
    add: impl FnOnce(&mut Ui),
) {
    ui.vertical(|ui| {
        let gap = ui.spacing().item_spacing.y;
        let lines: f32 = styles.iter().map(|s| ui.text_style_height(s)).sum();
        let text = lines + gap * styles.len().saturating_sub(1) as f32;
        let last = styles.last().map_or(0.0, |s| ui.text_style_height(s));
        // The letters, not the lines' boxes, are centred: the last line's room for
        // descenders hangs below.
        let top = (height - text + descent(last)) / 2.0;
        // add_space is followed by the item spacing too.
        ui.add_space((top - gap).max(0.0));
        add(ui);
    });
}

/// Who makes ApduStudio, for About's footer: Ialto's logo, then the copyright and licence over
/// a link to Ialto's website. The logo opens the website too. Nothing is drawn for the logo if
/// it can't be read.
pub(super) fn ialto_credit(ui: &mut Ui, weak: Color32) {
    if let Some(t) = png_texture(ui, "ialto-logo", crate::icon::IALTO_PNG) {
        let [w, h] = t.size();
        let width = IALTO_LOGO_HEIGHT * w as f32 / (h.max(1) as f32);
        let logo = egui::Image::new((t.id(), egui::vec2(width, IALTO_LOGO_HEIGHT)))
            .sense(egui::Sense::click());
        let r = ui
            .add(logo)
            .on_hover_text(crate::IALTO_URL)
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if r.clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(crate::IALTO_URL));
        }
        ui.add_space(10.0);
    }
    ui.vertical(|ui| {
        // The link's letters sit on the logo's bottom: the two lines start where their height,
        // less the room a line keeps under its letters for descenders, ends at the logo's foot.
        let gap = 2.0;
        ui.spacing_mut().item_spacing.y = gap;
        let small = ui.text_style_height(&egui::TextStyle::Small);
        let body = ui.text_style_height(&egui::TextStyle::Body);
        let top = IALTO_LOGO_HEIGHT - (small + gap + body) + descent(body);
        // add_space is followed by the item spacing too.
        ui.add_space((top - gap).max(0.0));
        ui.label(RichText::new(crate::COPYRIGHT).small().color(weak));
        let site = crate::IALTO_URL.trim_start_matches("https://");
        ui.hyperlink_to(site, crate::IALTO_URL)
            .on_hover_text(crate::IALTO_URL);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance.
    fn lum(c: Color32) -> f64 {
        let ch = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_and_marks_are_readable_in_both_themes() {
        for p in [LIGHT, DARK] {
            for bg in [p.bg, p.panel, p.field, p.stripe, p.selection] {
                assert!(contrast(p.text, bg) >= 7.0, "{:?} on {bg:?}", p.text);
                for fg in [p.weak, p.ok, p.warn, p.err, p.sent, p.accent] {
                    let c = contrast(fg, bg);
                    assert!(c >= 4.5, "{fg:?} on {bg:?}: {c:.2}");
                }
            }
        }
    }
}
