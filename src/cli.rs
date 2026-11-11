// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! `apdustudio-cli`: runs a script without a window, for scripts and test benches. It
//! prints the run's report as it goes and ends with an exit code a script can test: 0 when
//! no command failed (a warning doesn't fail a run), 1 when one failed, 2 when the run couldn't
//! start, 64 for a mistake in the command line. It uses the same runner as the window, so a scenario passes or fails in
//! both alike.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::card;
use crate::journal::Journal;
use crate::reader::{CardLink, Disp, Proto, Share, State, SystemOpener};
use crate::runner::{self, AtrCheck, Run, Step};
use crate::scenario;

/// No command failed; some may have a warning status word (`runner::Mark::Warning`).
pub const PASSED: u8 = 0;
/// At least one command failed (an unexpected answer, an error status word with no answer
/// expected, no answer at all, a skipped command).
pub const FAILED: u8 = 1;
/// The run couldn't start: no service, no reader, no card, an unreadable file, another ATR.
pub const NOT_RUN: u8 = 2;
/// The command line is wrong.
pub const USAGE_ERROR: u8 = 64;

pub const USAGE: &str = "\
Runs ApduStudio scripts (.apdu files) from a terminal or another program.

Usage:
  apdustudio-cli run FILE [options]   Run a script on the card in a reader
  apdustudio-cli readers              List the readers, and the card in each
  apdustudio-cli --help | --version

Add --demo to any command to use a simulated reader and card instead of PC/SC.

Options of run:
  --reader R           The reader: its name, part of its name, or its number in
                       the readers list (default: the first reader)
  --iterations N       Run the script N times (default 1)
  --share MODE         shared (default) or exclusive
  --protocol P         any (default), t0 or t1
  --follow-up          Send GET RESPONSE after 61xx, and the command again with the
                       right Le after 6Cxx; the whole answer is compared
  --no-atr-check       Run even if the card's ATR isn't the script's atr: line
  --log FILE           Write the log, in the window's format, to FILE
  --disconnect D       leave (default), reset or unpower the card at the end

With no expected answer, a command fails on an error status word (64xx to 6Fxx)
and has a warning on 62xx, 63xx or a status word outside ISO/IEC 7816-4.

Exit codes: 0 no command failed (warnings don't fail a run), 1 a command failed,
2 the run couldn't start, 64 the command line is wrong.
";

#[derive(Clone, Debug, PartialEq)]
struct RunArgs {
    file: PathBuf,
    reader: Option<String>,
    iterations: u32,
    share: Share,
    proto: Proto,
    follow_up: bool,
    check_atr: bool,
    log: Option<PathBuf>,
    disconnect: Disp,
}

#[derive(Clone, Debug, PartialEq)]
enum Command {
    Run(RunArgs),
    Readers,
    Help,
    Version,
}

fn parse(args: &[String]) -> Result<Command, String> {
    let mut it = args.iter();
    let first = it.next().map(String::as_str);
    match first {
        None | Some("--help" | "-h" | "help") => return Ok(Command::Help),
        Some("--version" | "-V") => return Ok(Command::Version),
        Some("readers") => {
            return match it.next() {
                None => Ok(Command::Readers),
                Some(a) => Err(format!("readers takes no argument: {a}")),
            }
        }
        Some("run") => {}
        Some(other) => return Err(format!("unknown command: {other}")),
    }
    let mut file = None;
    let mut a = RunArgs {
        file: PathBuf::new(),
        reader: None,
        iterations: 1,
        share: Share::Shared,
        proto: Proto::Any,
        follow_up: false,
        check_atr: true,
        log: None,
        disconnect: Disp::Leave,
    };
    while let Some(arg) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match arg.as_str() {
            "--reader" => a.reader = Some(value("--reader")?),
            "--iterations" => {
                let v = value("--iterations")?;
                a.iterations = v
                    .parse()
                    .ok()
                    .filter(|n| *n >= 1)
                    .ok_or_else(|| format!("--iterations takes a number from 1: {v}"))?;
            }
            "--share" => {
                a.share = match value("--share")?.to_ascii_lowercase().as_str() {
                    "shared" => Share::Shared,
                    "exclusive" => Share::Exclusive,
                    v => return Err(format!("--share is shared or exclusive: {v}")),
                }
            }
            "--protocol" => {
                a.proto = match value("--protocol")?.to_ascii_lowercase().as_str() {
                    "any" => Proto::Any,
                    "t0" | "t=0" => Proto::T0,
                    "t1" | "t=1" => Proto::T1,
                    v => return Err(format!("--protocol is any, t0 or t1: {v}")),
                }
            }
            "--disconnect" => {
                a.disconnect = match value("--disconnect")?.to_ascii_lowercase().as_str() {
                    "leave" => Disp::Leave,
                    "reset" => Disp::Reset,
                    "unpower" => Disp::PowerDown,
                    v => return Err(format!("--disconnect is leave, reset or unpower: {v}")),
                }
            }
            "--follow-up" => a.follow_up = true,
            "--no-atr-check" => a.check_atr = false,
            "--log" => a.log = Some(PathBuf::from(value("--log")?)),
            s if s.starts_with('-') => return Err(format!("unknown option: {s}")),
            s if file.is_none() => file = Some(PathBuf::from(s)),
            s => return Err(format!("one script at a time: {s}")),
        }
    }
    a.file = file.ok_or("run needs a script (.apdu file)")?;
    Ok(Command::Run(a))
}

/// The reader `wanted` names among `readers`: its exact name, its number, or the only one
/// whose name holds it (case ignored).
fn pick_reader(readers: &[String], wanted: Option<&str>) -> Result<String, String> {
    let Some(w) = wanted else {
        return readers
            .first()
            .cloned()
            .ok_or_else(|| "Cannot find a smart card reader.".to_string());
    };
    if let Some(r) = readers.iter().find(|r| *r == w) {
        return Ok(r.clone());
    }
    if let Some(r) = w.parse::<usize>().ok().and_then(|i| readers.get(i)) {
        return Ok(r.clone());
    }
    let low = w.to_lowercase();
    let found: Vec<&String> = readers
        .iter()
        .filter(|r| r.to_lowercase().contains(&low))
        .collect();
    match found.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(format!("No reader matches \"{w}\".")),
        _ => Err(format!(
            "More than one reader matches \"{w}\": name it in full."
        )),
    }
}

/// Runs the command line `args` (without the program's name) on the smart card service that
/// `open` gives; writes to `out` and `err`, and returns the exit code.
pub fn main(args: &[String], open: SystemOpener, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let cmd = match parse(args) {
        Ok(c) => c,
        Err(e) => {
            let _ = writeln!(err, "apdustudio-cli: {e}\nSee apdustudio-cli --help.");
            return USAGE_ERROR;
        }
    };
    match cmd {
        Command::Help => {
            let _ = write!(out, "{USAGE}");
            PASSED
        }
        Command::Version => {
            let _ = writeln!(
                out,
                "apdustudio-cli {} ({})",
                crate::app::APP_VERSION,
                crate::app::build_label()
            );
            PASSED
        }
        Command::Readers => list_readers(open, out, err),
        Command::Run(a) => match run(&a, open, out) {
            Ok(code) => code,
            Err(e) => {
                let _ = writeln!(err, "apdustudio-cli: {e}");
                NOT_RUN
            }
        },
    }
}

fn list_readers(open: SystemOpener, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let mut system = match open() {
        Ok(s) => s,
        Err(e) => {
            let _ = writeln!(err, "apdustudio-cli: {}", card::error_text(e));
            return NOT_RUN;
        }
    };
    let names = match system.readers() {
        Ok(n) => n,
        Err(e) => {
            let _ = writeln!(err, "apdustudio-cli: {}", card::error_text(e));
            return NOT_RUN;
        }
    };
    let states = system.reader_states(&names).unwrap_or_default();
    for (i, name) in names.iter().enumerate() {
        let st = states.iter().find(|s| &s.name == name);
        let card = match st {
            Some(s) if s.state.contains(State::PRESENT) && !s.atr.is_empty() => {
                format!("card, ATR {}", hex(&s.atr))
            }
            Some(s) if s.state.contains(State::PRESENT) => "card".to_string(),
            Some(_) => "no card".to_string(),
            None => "state unknown".to_string(),
        };
        let _ = writeln!(out, "{i}  {name}  ({card})");
    }
    PASSED
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Runs the script; Err when it couldn't start.
fn run(a: &RunArgs, open: SystemOpener, out: &mut dyn Write) -> Result<u8, String> {
    let sc =
        scenario::load(&a.file).map_err(|e| format!("can't read {}: {e}", a.file.display()))?;
    if sc.commands.is_empty() {
        return Err(format!("{} has no command.", a.file.display()));
    }
    let mut journal = match &a.log {
        Some(p) => Some(open_log(p)?),
        None => None,
    };
    let mut system = open().map_err(card::error_text)?;
    let readers = system.readers().map_err(card::error_text)?;
    let name = pick_reader(&readers, a.reader.as_deref())?;
    let mut link = system
        .connect(&name, a.share, a.proto)
        .map_err(|e| format!("{name}: {}", card::error_text(e)))?;
    let atr = link.status().ok().map(|s| s.atr);
    let _ = writeln!(
        out,
        "Reader: {name}\nATR: {}\n",
        atr.as_deref()
            .map(hex)
            .unwrap_or_else(|| "(unreadable)".into())
    );
    if a.check_atr {
        if let Some(exp) = &sc.atr_expected {
            match runner::check_atr(exp, atr) {
                AtrCheck::Matches => {}
                AtrCheck::Unreadable => {
                    finish(link, a.disconnect, out);
                    return Err("the card's ATR can't be read".into());
                }
                AtrCheck::Differs => {
                    finish(link, a.disconnect, out);
                    return Err(format!(
                        "the card's ATR isn't the script's ({exp}); --no-atr-check runs anyway"
                    ));
                }
            }
        }
    }
    let settings = runner::Settings {
        response_buffer: 65536,
        no_time: false,
        follow_up: a.follow_up,
    };
    let file = a.file.display().to_string();
    let (mut run, first) = Run::start(&file, a.iterations);
    let mut log = |text: &str| {
        if let Some(j) = journal.as_mut() {
            let _ = j.write(text);
        }
    };
    log(&format!(
        "{} {} log\nReader: {name}\n\n",
        crate::app::APP_NAME,
        crate::app::APP_VERSION
    ));
    log(&first);
    let mut printed = 0usize;
    let summary = loop {
        let step = run.step(&sc.commands, Some(link.as_mut()), settings, &mut log);
        // The report grows by each command's lines: print what's new.
        let report = run.report();
        let _ = write!(out, "{}", report.get(printed..).unwrap_or_default());
        printed = report.len();
        if let Step::Finished { summary, .. } = step {
            break summary;
        }
    };
    let _ = write!(out, "{}", summary.report.get(printed..).unwrap_or_default());
    let _ = writeln!(out);
    finish(link, a.disconnect, out);
    Ok(if summary.failed == 0 { PASSED } else { FAILED })
}

fn open_log(p: &Path) -> Result<Journal, String> {
    Journal::create(p).map_err(|e| format!("can't write the log {}: {e}", p.display()))
}

/// Ends the connection; a failure is said, not fatal.
fn finish(link: Box<dyn CardLink>, disp: Disp, out: &mut dyn Write) {
    if let Err((_, e)) = link.disconnect(disp) {
        let _ = writeln!(out, "Disconnect: {}", card::error_text(e));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::fake::{FakeSystem, Sent};

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    /// A script of its own, in the system's temporary folder: `body` after the header.
    fn scenario_file(name: &str, body: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("apdu-studio-cli-{}-{name}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("t.apdu");
        let text = format!("{} {}\n{body}", scenario::HEADER, scenario::VERSION);
        std::fs::write(&p, text).expect("write the script");
        p
    }

    fn cli(f: &FakeSystem, line: &[String]) -> (u8, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = main(line, f.opener(), &mut out, &mut err);
        (
            code,
            String::from_utf8_lossy(&out).into_owned(),
            String::from_utf8_lossy(&err).into_owned(),
        )
    }

    const READER: &str = "Test Reader 0";
    const ATR: [u8; 4] = [0x3B, 0x02, 0x14, 0x50];

    #[test]
    fn the_command_line_is_read_or_refused() {
        assert_eq!(parse(&args("")), Ok(Command::Help));
        assert_eq!(parse(&args("--version")), Ok(Command::Version));
        assert_eq!(parse(&args("readers")), Ok(Command::Readers));
        let Ok(Command::Run(a)) = parse(&args(
            "run t.apdu --reader 1 --iterations 3 --share exclusive --protocol T1 --follow-up \
             --no-atr-check --log out.log --disconnect unpower",
        )) else {
            panic!("not read");
        };
        assert_eq!(a.file, PathBuf::from("t.apdu"));
        assert_eq!(a.reader.as_deref(), Some("1"));
        assert_eq!(
            (a.iterations, a.share, a.proto, a.disconnect),
            (3, Share::Exclusive, Proto::T1, Disp::PowerDown)
        );
        assert!(a.follow_up && !a.check_atr);
        for bad in [
            "run",
            "run a.apdu b.apdu",
            "run a.apdu --iterations 0",
            "run a.apdu --share direct",
            "run a.apdu --reader",
            "run a.apdu --frobnicate",
            "explode",
            "readers now",
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn readers_are_picked_by_name_number_or_part_of_the_name() {
        let r: Vec<String> = ["ACME Contact 0", "ACME Contactless 0", "Other SAM 1"]
            .map(String::from)
            .to_vec();
        assert_eq!(pick_reader(&r, None), Ok("ACME Contact 0".into()));
        assert_eq!(pick_reader(&r, Some("2")), Ok("Other SAM 1".into()));
        assert_eq!(
            pick_reader(&r, Some("contact")),
            Err("More than one reader matches \"contact\": name it in full.".into())
        );
        assert_eq!(
            pick_reader(&r, Some("ACME Contact 0")),
            Ok("ACME Contact 0".into())
        );
        assert_eq!(
            pick_reader(&r, Some("contactless")),
            Ok("ACME Contactless 0".into())
        );
        assert!(pick_reader(&r, Some("nothing")).is_err());
        assert!(pick_reader(&[], None).is_err());
    }

    #[test]
    fn a_run_that_passes_exits_0_and_one_that_fails_exits_1() {
        let f = FakeSystem::with_card(READER, &ATR);
        f.answer(&[0x00, 0xA4, 0x00, 0x00, 0x02, 0x3F, 0x00], &[0x90, 0x00]);
        f.answer(&[0x00, 0xB0, 0x00, 0x00, 0x02], &[0x6A, 0x82]);
        let good = scenario_file(
            "good",
            "atr: 3B021450\n\nstep: MF\ncard: 00A40000023F00\nexpect: 9000\n",
        );
        let (code, out, err) = cli(
            &f,
            &[
                "run".into(),
                good.display().to_string(),
                "--iterations".into(),
                "2".into(),
            ],
        );
        assert_eq!(code, PASSED, "{out}{err}");
        assert!(out.contains(&format!("Reader: {READER}")), "{out}");
        assert!(
            out.contains("Result: 2 commands, 2 passed, 0 failed"),
            "{out}"
        );
        assert!(f.sent().contains(&Sent::Disconnect(Disp::Leave)));
        let bad = scenario_file(
            "bad",
            "card: 00A40000023F00\nexpect: 9000\ncard: 00B0000002\nexpect: 9000\n",
        );
        let (code, out, _) = cli(&f, &["run".into(), bad.display().to_string()]);
        assert_eq!(code, FAILED, "{out}");
        assert!(
            out.contains("Result: 2 commands, 1 passed, 1 failed"),
            "{out}"
        );
        // With nothing expected, an error status word fails the run; a warning doesn't.
        f.answer(&[0x00, 0xB0, 0x00, 0x01, 0x02], &[0x62, 0x82]);
        let error = scenario_file("error", "card: 00B0000002\n");
        let (code, out, _) = cli(&f, &["run".into(), error.display().to_string()]);
        assert_eq!(code, FAILED, "{out}");
        assert!(out.contains("SW 6A82, an error"), "{out}");
        let warning = scenario_file("warning", "card: 00B0000102\n");
        let (code, out, _) = cli(&f, &["run".into(), warning.display().to_string()]);
        assert_eq!(code, PASSED, "{out}");
        assert!(
            out.contains("Result: 1 command, 0 passed, 1 warning, 0 failed"),
            "{out}"
        );
        for p in [good, bad, error, warning] {
            if let Some(d) = p.parent() {
                let _ = std::fs::remove_dir_all(d);
            }
        }
    }

    #[test]
    fn a_run_that_cannot_start_exits_2_and_says_why() {
        let f = FakeSystem::with_card(READER, &ATR);
        let other = scenario_file("atr", "atr: 3B00\ncard: 00A40000023F00\n");
        let (code, _, err) = cli(&f, &["run".into(), other.display().to_string()]);
        assert_eq!(code, NOT_RUN);
        assert!(err.contains("isn't the script's"), "{err}");
        assert!(
            !f.sent().iter().any(|s| matches!(s, Sent::Transmit(_))),
            "nothing was sent to the card"
        );
        // With --no-atr-check it runs.
        f.answer(&[0x00, 0xA4, 0x00, 0x00, 0x02, 0x3F, 0x00], &[0x90, 0x00]);
        let (code, _, _) = cli(
            &f,
            &[
                "run".into(),
                other.display().to_string(),
                "--no-atr-check".into(),
            ],
        );
        assert_eq!(code, PASSED);
        let (code, _, err) = cli(&f, &args("run no-such-file.apdu"));
        assert_eq!(code, NOT_RUN);
        assert!(err.contains("can't read no-such-file.apdu"), "{err}");
        // A mistake in the script names its line, and nothing is sent.
        let sent = f.sent().len();
        let broken = scenario_file("broken", "card: 00A4\ncrad: 00\n");
        let (code, _, err) = cli(&f, &["run".into(), broken.display().to_string()]);
        assert_eq!(code, NOT_RUN);
        assert!(err.contains("line 3: unknown keyword"), "{err}");
        assert_eq!(f.sent().len(), sent);
        if let Some(d) = broken.parent() {
            let _ = std::fs::remove_dir_all(d);
        }
        let (code, _, err) = cli(
            &f,
            &[
                "run".into(),
                other.display().to_string(),
                "--reader".into(),
                "nothing".into(),
            ],
        );
        assert_eq!(code, NOT_RUN);
        assert!(err.contains("No reader matches"), "{err}");
        let (code, _, err) = cli(&f, &args("run"));
        assert_eq!(code, USAGE_ERROR);
        assert!(
            err.contains(
                "run needs a script (.apdu file)
See apdustudio-cli --help."
            ),
            "{err}"
        );
        if let Some(d) = other.parent() {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn readers_are_listed_with_their_card() {
        let f = FakeSystem::with_card(READER, &ATR);
        let (code, out, _) = cli(&f, &args("readers"));
        assert_eq!(code, PASSED);
        assert_eq!(out, format!("0  {READER}  (card, ATR 3B 02 14 50)\n"));
    }
}
