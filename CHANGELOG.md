# Changelog

What changes from one version of ApduStudio to the next. Each version's section is also its
release's notes on GitHub. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the versions follow
[Semantic Versioning](https://semver.org/): a new feature raises the minor number, fixes alone
the patch number, and a change that breaks scripts, the settings file or the command line the
major number.

## [1.1.0] - 2026-11-16

### Added

- Detail: hovering a field's bytes, or an ATR, opens a tooltip that shows them whole with a
  **Copy** button. It copies them as hex with spaces, then says "Copied" for a moment. Before,
  only bytes too long for their column had a tooltip, without a button.

## [1.0.0] - 2026-11-11

First public release, under the Apache License 2.0.

### Added

- The window: connection to a card or a reader through PC/SC (share mode, protocol, what to do
  with the card on disconnection), APDUs typed in the command line, CCID escape commands, and
  the readings (readers' state, card state, reader info, contactless card's UID).
- Scripts (`.apdu`, format `apdustudio-script 1`) that send each command to the card or the
  reader and check its answer, step by step or in full, with iterations.
- The exchanges decoded by the standards that define them: commands by ISO/IEC 7816-4 (class FF
  by PC/SC part 3), status words, ATRs by ISO/IEC 7816-3, answer data read as BER-TLV.
- The log file, and `apdustudio-cli` to run scripts from a terminal, with its exit codes.
- A simulated reader and card (`--demo`), the help (F1), light and dark themes.
- Builds for Windows (x64), Linux (x64, glibc 2.28 and later) and macOS (Apple silicon).

[1.1.0]: https://github.com/Ialto-Paris/apdustudio/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/Ialto-Paris/apdustudio/releases/tag/v1.0.0
