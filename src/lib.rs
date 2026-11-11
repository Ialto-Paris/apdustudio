// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! ApduStudio's engine and window, shared by its two programs: `ApduStudio` (the window,
//! `src/main.rs`) and `apdustudio-cli` (scripts run from a terminal or another program,
//! `src/bin/apdustudio-cli.rs`).

pub mod app;
pub mod card;
pub mod cli;
pub mod decode;
pub mod demo;
pub mod icon;
pub mod iso;
pub mod journal;
pub mod reader;
pub mod runner;
pub mod scenario;
pub mod settings;
pub mod stack;
pub mod tlv;
pub mod worker;

use std::path::PathBuf;

/// The copyright line shown by About.
pub const COPYRIGHT: &str = "© 2026 Ialto";
/// The licence ApduStudio is distributed under.
pub const LICENSE: &str = "Apache License 2.0";
/// Ialto's website, linked from About.
pub const IALTO_URL: &str = "https://ialto.com";

/// What `--show` opens at start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    Help,
    About,
}

/// Command line options of the window.
#[derive(Clone, Debug)]
pub struct Options {
    /// `--expert`: a run doesn't check the card's ATR against the script's `atr:` line.
    pub expert: bool,
    /// `--no-times`: no durations in the log.
    pub no_time: bool,
    /// `--command-buffer N`: the longest command, in bytes.
    pub command_buffer: usize,
    /// `--response-buffer N`: the longest answer, in bytes.
    pub response_buffer: usize,
    /// Captures the window to this file after it has settled, then closes (for checks).
    pub screenshot: Option<PathBuf>,
    /// A simulated reader and card instead of the smart card service.
    pub demo: bool,
    /// `--theme light|dark`, for this run only; else the one kept in the settings.
    pub theme: Option<egui::Theme>,
    /// A script given on the command line, opened at start.
    pub open_file: Option<PathBuf>,
    /// `--show help|about`: opens the help or About at start (with `--screenshot`, to capture
    /// them).
    pub show: Option<Show>,
    /// The settings file (`settings::default_path`); None keeps nothing, as in the tests.
    pub settings: Option<PathBuf>,
}
