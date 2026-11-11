// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

#![windows_subsystem = "windows"]

use std::path::PathBuf;

use apdu_studio::{app, demo, icon, reader, settings, Options, Show};

/// A buffer size from the command line: 1000 to 65536 bytes.
fn buffer_size(v: Option<&String>) -> Option<usize> {
    let n: usize = v?.trim().parse().ok()?;
    (1000..=65536).contains(&n).then_some(n)
}

/// Reads the command line. Unknown options are left aside: the window has no console to say so.
fn parse_args() -> Options {
    let mut o = Options {
        expert: false,
        no_time: false,
        command_buffer: 65536,
        response_buffer: 65536,
        screenshot: None,
        demo: false,
        theme: None,
        open_file: None,
        show: None,
        settings: settings::default_path(),
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while let Some(a) = args.get(i) {
        let value = args.get(i + 1);
        match a.as_str() {
            "--expert" => o.expert = true,
            "--no-times" => o.no_time = true,
            "--demo" => o.demo = true,
            "--command-buffer" => {
                o.command_buffer = buffer_size(value).unwrap_or(o.command_buffer);
                i += 1;
            }
            "--response-buffer" => {
                o.response_buffer = buffer_size(value).unwrap_or(o.response_buffer);
                i += 1;
            }
            "--screenshot" => {
                o.screenshot = value.map(PathBuf::from);
                i += 1;
            }
            "--theme" => {
                o.theme = match value.map(String::as_str) {
                    Some("light") => Some(egui::Theme::Light),
                    Some("dark") => Some(egui::Theme::Dark),
                    _ => None,
                };
                i += 1;
            }
            "--show" => {
                o.show = match value.map(String::as_str) {
                    Some("help") => Some(Show::Help),
                    Some("about") => Some(Show::About),
                    _ => None,
                };
                i += 1;
            }
            file if !file.starts_with('-') => o.open_file = Some(PathBuf::from(file)),
            _ => {}
        }
        i += 1;
    }
    o
}

fn main() -> eframe::Result<()> {
    let opts = parse_args();
    // The version is in the title, so it shows in the taskbar and in screenshots.
    let name = format!("{} {}", app::APP_NAME, app::APP_VERSION);
    let title = if opts.demo {
        format!("{} (demo: simulated reader)", name)
    } else {
        name
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(app::WINDOW_SIZE)
        .with_min_inner_size(app::MIN_WINDOW_SIZE);
    // The title bar's and taskbar's icon (the Dock's on macOS).
    if let Some((rgba, width, height)) = icon::rgba() {
        viewport = viewport.with_icon(std::sync::Arc::new(egui::IconData {
            rgba,
            width,
            height,
        }));
    }
    let native = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let opener = if opts.demo {
        demo::system().opener()
    } else {
        reader::PcscSystem::opener()
    };
    eframe::run_native(
        app::APP_NAME,
        native,
        Box::new(move |cc| Ok(Box::new(app::ApduStudio::new(cc, opts, opener)))),
    )
}
