// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! The help shown in the window (F1): what each part does, the script format, the options and
//! keys. The text is data: topics made of paragraphs, lists, rows and examples.

/// One piece of a topic.
pub(super) enum Block {
    /// A paragraph.
    P(&'static str),
    /// A subheading.
    H(&'static str),
    /// A bulleted list.
    List(&'static [&'static str]),
    /// Two columns: a name (shown in monospace) and what it means.
    Rows(&'static [(&'static str, &'static str)]),
    /// An example, in monospace.
    Code(&'static str),
}

pub(super) struct Topic {
    pub(super) title: &'static str,
    pub(super) blocks: &'static [Block],
}

impl Topic {
    /// The topic holds every word of `query` (case ignored); an empty query matches all.
    pub(super) fn matches(&self, query: &str) -> bool {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return true;
        }
        let text = self.text().to_lowercase();
        words.iter().all(|w| text.contains(w.as_str()))
    }

    /// All the topic's text, for searching.
    pub(super) fn text(&self) -> String {
        let mut s = String::from(self.title);
        for b in self.blocks {
            s.push('\n');
            match b {
                Block::P(t) | Block::H(t) | Block::Code(t) => s.push_str(t),
                Block::List(items) => {
                    for i in *items {
                        s.push_str(i);
                        s.push('\n');
                    }
                }
                Block::Rows(rows) => {
                    for (a, b) in *rows {
                        s.push_str(a);
                        s.push(' ');
                        s.push_str(b);
                        s.push('\n');
                    }
                }
            }
        }
        s
    }
}

use super::dialogs::Dialog;
use super::widgets::{app_icon, Palette};
use super::{build_label, ApduStudio, APP_NAME, APP_VERSION};
use egui::{Align, Layout, RichText, Sense, Stroke, Ui, Vec2};
use Block::{Code, List, Rows, H, P};

/// The help window's state: open on a topic, with the search text.
#[derive(Default)]
pub(super) struct HelpState {
    pub(super) open: bool,
    pub(super) topic: usize,
    pub(super) query: String,
}

impl ApduStudio {
    /// F1 or "?": opens the help, or closes it when it is open.
    pub(super) fn toggle_help(&mut self) {
        self.help.open = !self.help.open;
    }

    pub(super) fn help_window(&mut self, ctx: &egui::Context) {
        if !self.help.open {
            return;
        }
        let mut open = true;
        let mut about = false;
        let help = &mut self.help;
        egui::Window::new(format!("{} help", APP_NAME))
            .id(egui::Id::new("help"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([780.0, 540.0])
            .min_size([520.0, 300.0])
            .show(ctx, |ui| about = help_ui(ui, help));
        self.help.open &= open;
        if about && self.dialogs.is_empty() {
            self.dialogs.push_back(Dialog::About);
        }
    }
}

/// Draws the help; true when About was clicked.
fn help_ui(ui: &mut Ui, help: &mut HelpState) -> bool {
    let mut about = false;
    let p = Palette::of_ui(ui);
    let found: Vec<usize> = TOPICS
        .iter()
        .enumerate()
        .filter(|(_, t)| t.matches(&help.query))
        .map(|(i, _)| i)
        .collect();
    if !found.contains(&help.topic) {
        if let Some(&first) = found.first() {
            help.topic = first;
        }
    }
    egui::Panel::left("help_topics")
        .resizable(false)
        .default_size(200.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::new(36.0, 36.0), Sense::hover());
                app_icon(ui, r);
                ui.vertical(|ui| {
                    ui.label(RichText::new(format!("{} {}", APP_NAME, APP_VERSION)).strong());
                    ui.label(RichText::new(build_label()).color(p.weak));
                });
            });
            if ui
                .button("About...")
                .on_hover_text("Version, build, system, where settings and log are kept")
                .clicked()
            {
                about = true;
            }
            ui.add_space(6.0);
            ui.add(
                egui::TextEdit::singleline(&mut help.query)
                    .hint_text("Search")
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .id_salt("help_list")
                .show(ui, |ui| {
                    for &i in &found {
                        if let Some(t) = TOPICS.get(i) {
                            if ui.selectable_label(help.topic == i, t.title).clicked() {
                                help.topic = i;
                            }
                        }
                    }
                    if found.is_empty() {
                        ui.label(RichText::new("Nothing found").color(p.weak));
                    }
                });
        });
    egui::CentralPanel::default_margins().show(ui, |ui| {
        let Some(t) = TOPICS.get(help.topic).filter(|_| !found.is_empty()) else {
            return;
        };
        egui::ScrollArea::vertical()
            .id_salt(("help_topic", help.topic))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label(RichText::new(t.title).heading());
                ui.add_space(4.0);
                for b in t.blocks {
                    block_ui(ui, b, &p);
                }
            });
    });
    about
}

fn block_ui(ui: &mut Ui, b: &Block, p: &Palette) {
    match b {
        P(text) => {
            ui.add(egui::Label::new(*text).wrap());
            ui.add_space(4.0);
        }
        H(text) => {
            ui.add_space(6.0);
            ui.label(RichText::new(*text).strong());
        }
        List(items) => {
            for item in *items {
                ui.horizontal_top(|ui| {
                    ui.label(RichText::new("•").color(p.weak));
                    ui.add(egui::Label::new(*item).wrap());
                });
            }
            ui.add_space(4.0);
        }
        Rows(rows) => {
            for (name, what) in *rows {
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(150.0, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(150.0);
                            ui.add(egui::Label::new(RichText::new(*name).monospace()).wrap());
                        },
                    );
                    ui.add(egui::Label::new(*what).wrap());
                });
                ui.add_space(2.0);
            }
            ui.add_space(4.0);
        }
        Code(text) => {
            egui::Frame::new()
                .fill(p.field)
                .stroke(Stroke::new(1.0, p.border))
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.add(egui::Label::new(RichText::new(*text).monospace()).selectable(true));
                });
            ui.add_space(4.0);
        }
    }
}

pub(super) const TOPICS: &[Topic] = &[
    Topic {
        title: "Overview",
        blocks: &[
            P("ApduStudio tests smart cards through PC/SC: connect to a card, send APDUs, run \
               scripts that check each answer, and keep a log of everything sent and \
               received."),
            H("The window"),
            Rows(&[
                ("Connection bar", "The reader, the share mode and protocol, Connect or Disconnect; below, the connection, the card's ATR and the readings."),
                ("Script", "The commands of an .apdu script with their expected answers and last results. Run, Step, Stop; the selected row is edited below the list."),
                ("Exchanges", "Every command and answer of the session, with notes in between. Click an exchange, or a note with an ATR, to see its bytes decoded."),
                ("Command line", "An APDU to send now, the CCID escape, the log file and the status line."),
            ]),
            H("Colours"),
            P("Colour only marks state: green passed, red failed, amber a warning, blue what was \
               sent."),
            P("Try it without a reader: start ApduStudio with --demo, then open \
               examples/demo.apdu."),
            H("Version"),
            P("The window's title shows the version. About..., at the top of this help, shows \
               it with the build (the commit for a build made by CI, else \"local build\"), the \
               system, the smart card service and where the settings and the log are kept; \
               Copy puts them on the clipboard, for a bug report."),
            P("The smart card service is named with its version when the system gives it: \
               WinSCard.dll's file version on Windows, pcscd --version with pcsc-lite, the \
               macOS version for PCSC.framework (which follows it). PC/SC itself has no call \
               for it, so without one only the name is shown."),
        ],
    },
    Topic {
        title: "Connecting",
        blocks: &[
            List(&[
                "Reader: the readers PC/SC lists; Refresh lists them again.",
                "Share mode: Shared lets other programs use the card too; Exclusive keeps it to ApduStudio; Direct talks to the reader even without a card (no protocol then).",
                "Protocol: T0, T1, or T0 | T1 to let the card choose.",
                "Connect: if the reader is empty, ApduStudio waits for a card; Cancel or Esc stops waiting.",
                "Once connected, the second row shows the share mode, the protocol and the card's ATR.",
                "The selected reader is looked at twice a second: a card put in (with its ATR, decoded in the detail) or taken out is noted in the exchanges.",
                "Taking out the connected card loses the connection: Reconnect appears beside Disconnect and connects again, the same way, once a card is back.",
            ]),
            H("Disconnect"),
            P("The list beside Disconnect says what the reader does with the card:"),
            Rows(&[
                ("Power down", "Switches the card off."),
                ("Reset", "Resets the card."),
                ("Leave", "Leaves the card as it is."),
                ("Eject", "Ejects the card, on readers that can."),
            ]),
        ],
    },
    Topic {
        title: "Sending a command",
        blocks: &[
            P("Type an APDU in hex in the command line at the bottom; spaces are allowed."),
            Code("00 A4 04 00 07 A0000000041010"),
            Rows(&[
                ("Enter, Transmit", "Sends it to the card (SCardTransmit)."),
                ("Up, Down", "Goes through the commands sent before."),
            ]),
            P("ApduStudio speaks PC/SC only: commands go to the card with SCardTransmit, and the \
               reader is reached through standard means alone (Reader info, the CCID escape)."),
            P("The answer is read into a buffer of 65536 bytes (--response-buffer N changes it); \
               a command may be as long as the command buffer (--command-buffer N)."),
            H("Follow 61xx / 6Cxx"),
            P("With this box ticked (it is kept in the settings; off at first), ApduStudio sends \
               GET RESPONSE after 61xx, in the same logical channel and without secure \
               messaging (class 00 after a proprietary class), and sends the command again with \
               the right Le after 6Cxx (ISO/IEC 7816-3 and 7816-4), up to 32 commands. Every \
               command sent is shown in the exchanges and written to the log; a note then gives \
               the whole answer (each part's data, then the last status word), which the \
               detail decodes and a script compares."),
        ],
    },
    Topic {
        title: "Scripts",
        blocks: &[
            List(&[
                "Open... (Ctrl+O) reads an .apdu script; Save (Ctrl+S) writes it. A script given on the command line opens at start.",
                "Each row shows the command (\"reader\" before it when it goes to the reader), its step, the expected answer and the last result.",
                "Run (F5) sends every command, the number of times in Iterations. Stop or Esc ends the run after the command on its way.",
                "Step (F6) sends the selected command, to the card or the reader, and selects the next.",
                "Before a run, the card's ATR is compared with the script's atr: line, if it has one. --expert skips this check.",
                "A command holding a placeholder (<...>) is skipped and counts as failed.",
                "A VERIFY (ISO 7816-4: class 0x, INS 20) that fails stops the run, so a wrong PIN isn't presented again: failing means not the expected answer when the script gives one (a wrong-PIN test expects 63Cx), else not 9000.",
                "At the end: the commands passed, with a warning and failed, and the time.",
            ]),
            H("Passed, warning, failed"),
            P("A command with an expected answer passes when its answer matches, whatever the \
               status word: a test that expects 6D00 and gets it passes. A card command without \
               one should succeed, so its status word decides, by its ISO/IEC 7816-4 category:"),
            Rows(&[
                ("9000, 61xx", "Normal processing: ok (green)."),
                ("62xx, 63xx", "Warning processing: warn (amber). Counted apart; the run doesn't fail."),
                ("64xx to 6Fxx", "Execution or checking error, such as 6D00: fail (red)."),
                ("Others", "Outside ISO/IEC 7816-4, defined by the card: warn (amber)."),
            ]),
            P("A reader's answer to an escape command has no status word: with nothing expected, \
               any answer passes. The same rules apply to Step. A command typed in the command \
               line isn't judged: its status word's meaning takes the colour of its category in \
               the exchanges."),
            H("Editing"),
            P("The selected row is edited below the list: where it goes (Card or Reader), its \
               bytes, the expected answer and its step. Add puts a new row after it; Delete, Up \
               and Down act on it. A changed script shows * after its name, and ApduStudio asks \
               before losing the changes."),
            P("A script holds at most 10,000 commands."),
        ],
    },
    Topic {
        title: "Script files (.apdu)",
        blocks: &[
            P("A script is plain text. Its first line names the format and its version; every \
               other line is blank, a comment, a keyword line, or the continuation of the line \
               above:"),
            Rows(&[
                ("apdustudio-script 1", "The first line. A script written for a newer format is refused, with a word on why."),
                ("# ...", "A comment."),
                ("atr:", "The ATR the card must have before a run, as SCardStatus reads it."),
                ("step:", "What the next command does: shown in the list, the exchanges and the report."),
                ("card:", "An APDU, sent to the card with SCardTransmit."),
                ("reader:", "A command to the reader, sent as a CCID escape command (SCardControl)."),
                ("expect:", "The expected answer of the command above (see Expected answers)."),
                ("(indented)", "A line starting with a space or a tab continues the line above."),
            ]),
            H("Bytes"),
            List(&[
                "Hex digits, with spaces anywhere between bytes.",
                "A placeholder, <like this>, marks something to fill in, such as a PIN: the command is skipped until it is replaced by bytes.",
            ]),
            P("A mistake (an unknown keyword, a digit missing, an expect: with no command) stops \
               the opening and names its line."),
            H("Example"),
            Code("apdustudio-script 1\n# Read the card's master file.\natr: 3B 88 80 01 00 00 00 00 77 81 80 00 7F\n\nstep: Select the master file\ncard: 00 A4 00 00 02 3F 00\nexpect: 90 00\n\nstep: Select the application\ncard: 00 A4 04 00 07\n      A0 00 00 00 04 10 10\nexpect: 6F *\n\nstep: Verify PIN (template)\ncard: 00 20 00 80 04 <PIN, 4 bytes>\nexpect: 90 00"),
        ],
    },
    Topic {
        title: "Expected answers",
        blocks: &[
            P("The answer, data and status word together, is compared byte by byte with the hex \
               after expect:. Case and spaces don't matter."),
            Rows(&[
                ("90 00", "Exactly 90 00, with no data."),
                ("?", "Any hex digit at that place: ?? ?? 90 00 is two bytes of data, then 90 00."),
                ("*", "At the end, stops comparing: 61 * takes any answer starting with 61."),
            ]),
            P("Without *, the lengths must be equal. A difference names the byte, counted from \
               1: \"byte 1 is 6A, expected 90\", or the lengths: \"the answer has 2 bytes, \
               expected 3\". Without expect:, a card command is judged by its status word (see \
               Scripts)."),
        ],
    },
    Topic {
        title: "Exchanges",
        blocks: &[
            List(&[
                "Each exchange shows its time since the session started, what was sent, the answer, how long it took, the channel (card or escape) and the verdict.",
                "Click one to see it decoded, field by field (bytes, name, meaning; hover a name for the clause of the standard that defines it):",
                "The command by ISO/IEC 7816-4: CLA (chaining, secure messaging, logical channel, or proprietary), INS and its name, P1 and P2 for SELECT, READ/UPDATE BINARY, READ RECORD, VERIFY, GET DATA, AUTHENTICATE and MANAGE CHANNEL, the case (1 to 4, short or extended) with Lc, the data and Le. Class FF is read as a PC/SC part 3 command to the reader (GET DATA UID, LOAD KEYS, GENERAL AUTHENTICATE...).",
                "The answer: its data length and the status word, with its ISO/IEC 7816-4 category (normal, warning, execution error, checking error) and meaning; 61xx and 6Cxx say which command to send next. Hex and ASCII dumps of both follow. A CCID escape is the reader maker's format, so it is only dumped.",
                "A note that shows an ATR (Connected, Card state, Readers' state) opens the ATR decoded by ISO/IEC 7816-3: TS, T0, every interface byte for its protocol (Fi/Di, guard time, T=0 waiting time, T=1 IFSC, BWI/CWI and error code, T=15 clock stop and voltage classes), the historical bytes by ISO/IEC 7816-4 (COMPACT-TLV objects, card capabilities, status indicator) and the TCK, checked. A contactless card's ATR is also read as PC/SC part 3 builds it: a storage card's standard and name, or a type A card's ATS bytes or a type B card's ATQB.",
                "Bytes that break the standard (a wrong TCK, a missing byte) are shown in red. Other notes have nothing more to show and can't be selected.",
                "In the detail, Copy detail copies everything decoded there as text (an ATR's fields, or the command's and the answer's, one per line with their standard's clause), Copy answer copies the answer's hex, and To command line puts the command in the command line.",
                "The answer's data, when it reads wholly as BER-TLV (the FCI after SELECT, the FCP, GET DATA objects...), is shown object by object: its bytes (tag, length and value), its ISO/IEC 7816-4 name and what the value means, by the coding of the file descriptor, file identifier, short EF identifier, life cycle status, sizes and AIDs.",
            ]),
            H("Find and filter"),
            List(&[
                "Find shows the exchanges and notes holding a text: hex or words, with spaces and case not counting (6a82 finds 6A 82, 00b2 finds every READ RECORD).",
                "Problems only keeps failed exchanges, unexpected answers, status words other than 9000 and 61xx, warnings and errors.",
                "The count beside Exchanges then says how many of all are shown. Copy copies the exchanges shown, as text; Clear empties the list.",
            ]),
        ],
    },
    Topic {
        title: "Readings",
        blocks: &[
            Rows(&[
                ("Readers' state", "SCardGetStatusChange for every reader: its state flags, each with its PC/SC name (SCARD_STATE_EMPTY, SCARD_STATE_PRESENT, SCARD_STATE_INUSE...), and the ATR."),
                ("Card state", "The connected card's state, protocol and ATR (SCardStatus)."),
                ("Reader info", "The reader's vendor, model, version, serial number, connection, protocols and clock (SCardGetAttrib: those the driver gives), and the features it lists with PC/SC part 10's GET_FEATURE_REQUEST. Works without a card: when no connection is open, a direct one is opened for the reading and closed after it."),
                ("UID", "Sends GET DATA FF CA 00 00 00 (PC/SC part 3) to the card: a contactless reader answers the card's UID. On a contact reader the card refuses class FF, which the answer says."),
            ]),
            P("An error of the smart card service names its PC/SC constant and code, such as \
               \"The reader has no card (SCARD_E_NO_SMARTCARD, 0x8010000C)\"."),
        ],
    },
    Topic {
        title: "CCID escape",
        blocks: &[
            P("Sends a command to the reader itself, in hex, through its CCID driver \
               (SCardControl); the field keeps the commands sent before. It works while a card \
               is connected. A script's reader: lines are sent the same way."),
            P("The control code is the one the reader gives for FEATURE_CCID_ESC_COMMAND \
               (PC/SC part 10), asked once per connection. Readers that give none get the \
               system's code:"),
            Rows(&[
                ("Windows", "IOCTL_CCID_ESCAPE, SCARD_CTL_CODE(3500) = 0x003136B0"),
                ("Linux, macOS", "IOCTL_SMARTCARD_VENDOR_IFD_EXCHANGE, SCARD_CTL_CODE(1) = 0x42000001"),
            ]),
            P("Drivers refuse escape commands until they are allowed: with Windows' own CCID \
               driver, the reader's EscapeCommandEnable registry value (a DWORD) must be \
               non-zero; with pcsc-lite, ifdDriverOptions in the CCID driver's Info.plist must \
               include 0x0001."),
        ],
    },
    Topic {
        title: "Log file",
        blocks: &[
            List(&[
                "Start log writes ApduStudio.log: next to the program on Windows, in the folder ApduStudio was started from elsewhere. Stop log closes it.",
                "If the file exists, ApduStudio asks: Yes adds to it, No starts it again.",
                "Each command, its answer and its check are written as below; a run writes its report, laid out the same way. --no-times leaves out the durations.",
                "Every line is written at once. If a write fails, the log is closed and ApduStudio says so.",
            ]),
            Code("card    00 B2 01 0C 00\nanswer  6A 83  (3 ms)\nexpect  90 00\nfailed  byte 1 is 6A, expected 90"),
        ],
    },
    Topic {
        title: "Settings",
        blocks: &[
            P("The theme chosen in the top right (System, Light or Dark) and the Follow 61xx / \
               6Cxx box are kept for the next runs, in ApduStudio.toml:"),
            Rows(&[
                ("Windows", "%APPDATA%\\ApduStudio\\ApduStudio.toml"),
                ("Linux", "$XDG_CONFIG_HOME/apdu-studio/ApduStudio.toml, else ~/.config/apdu-studio/"),
                ("macOS", "~/Library/Application Support/ApduStudio/ApduStudio.toml"),
            ]),
            P("--theme light or --theme dark sets the theme for one run and leaves the file \
               alone."),
        ],
    },
    Topic {
        title: "Command line options",
        blocks: &[
            Code("ApduStudio [options] [script.apdu]"),
            Rows(&[
                ("--expert", "A run doesn't check the card's ATR against the script's atr: line."),
                ("--no-times", "No durations in the log."),
                ("--command-buffer N", "The longest command, 1000 to 65536 bytes (65536)."),
                ("--response-buffer N", "The longest answer, 1000 to 65536 bytes (65536)."),
                ("--demo", "A simulated reader and card instead of PC/SC."),
                ("--theme light|dark", "The theme for this run only."),
                ("script.apdu", "Opens this script at start."),
            ]),
        ],
    },
    Topic {
        title: "Running from a terminal",
        blocks: &[
            P("apdustudio-cli, beside ApduStudio, runs scripts without a window, from a terminal \
               or another program, on a test bench. It uses the same runner: a script passes or \
               fails in both alike, and the log has the same layout."),
            Code("apdustudio-cli run tests.apdu --reader \"Contactless\" --iterations 3 --log run.log\napdustudio-cli readers"),
            Rows(&[
                ("--reader R", "The reader: its name, part of it, or its number in the readers list (default: the first)."),
                ("--iterations N", "Run the script N times."),
                ("--share, --protocol", "shared or exclusive; any, t0 or t1."),
                ("--follow-up", "Follow 61xx and 6Cxx, as the box in the window does."),
                ("--no-atr-check", "Run even if the card's ATR isn't the script's."),
                ("--log FILE", "Write the log to FILE."),
                ("--disconnect D", "leave (default), reset or unpower the card at the end."),
                ("--demo", "The simulated reader and card."),
            ]),
            P("It prints the report as it goes and ends with an exit code: 0 no command failed \
               (warnings don't fail a run), 1 a command failed, 2 the run couldn't start (no \
               service, reader or card, an unreadable script, another ATR), 64 a mistake in the \
               command line."),
        ],
    },
    Topic {
        title: "Keys",
        blocks: &[
            Rows(&[
                ("Enter", "Transmit the command line."),
                ("Up, Down", "The command line's history."),
                ("F5", "Run the script."),
                ("F6", "Step: send the selected command."),
                ("Esc", "Stop the run, stop waiting for a card, or close this help."),
                ("Ctrl+O, Ctrl+S", "Open, save the script."),
                ("F1", "This help."),
            ]),
            P("On macOS, Cmd takes the place of Ctrl."),
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_topic_has_a_title_and_content() {
        assert!(!TOPICS.is_empty());
        for t in TOPICS {
            assert!(!t.title.is_empty());
            assert!(!t.blocks.is_empty(), "{}", t.title);
        }
    }

    #[test]
    fn search_needs_every_word_and_ignores_case() {
        let format = TOPICS
            .iter()
            .find(|t| t.title.starts_with("Script files"))
            .expect("the .apdu topic");
        assert!(format.matches(""));
        assert!(format.matches("  "));
        assert!(format.matches("PLACEHOLDER escape"));
        assert!(!format.matches("placeholder nonsense"));
        let found: Vec<&str> = TOPICS
            .iter()
            .filter(|t| t.matches("EscapeCommandEnable"))
            .map(|t| t.title)
            .collect();
        assert_eq!(found, vec!["CCID escape"]);
    }

    /// The help names the options and keys the program reads; a renamed one must be renamed here.
    #[test]
    fn the_options_and_keys_are_all_described() {
        let all: String = TOPICS.iter().map(Topic::text).collect();
        for word in [
            "--expert",
            "--no-times",
            "--command-buffer",
            "--response-buffer",
            "--demo",
            "--theme",
            "F1",
            "F5",
            "F6",
            "Esc",
            "Ctrl+O",
            "Ctrl+S",
        ] {
            assert!(all.contains(word), "{word} isn't described");
        }
    }
}
