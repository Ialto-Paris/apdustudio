// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! `apdustudio-cli`: ApduStudio's scenarios run from a terminal or a script (see `cli`). A
//! console program of its own, so that on Windows too the terminal waits for it and gets its
//! output and exit code. `--demo` uses the window's simulated reader and card.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let demo = args.iter().any(|a| a == "--demo");
    args.retain(|a| a != "--demo");
    let opener = if demo {
        apdu_studio::demo::system().opener()
    } else {
        apdu_studio::reader::PcscSystem::opener()
    };
    let code = apdu_studio::cli::main(
        &args,
        opener,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    );
    ExitCode::from(code)
}
