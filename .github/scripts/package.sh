#!/usr/bin/env bash
# Packages the release build of this runner's system into dist/, for CI artifacts and releases:
#   Windows       dist/ApduStudio-windows-<arch>.exe and dist/apdustudio-cli-windows-<arch>.exe
#   Linux, macOS  dist/ApduStudio-<os>-<arch>.tar.gz, holding ApduStudio and apdustudio-cli with
#                 their execute bit (a plain file would lose it on the way through GitHub).
# Run after `cargo build --release`, from the repository's root.
set -euo pipefail

case "$(uname -m)" in
  x86_64 | amd64) arch=x64 ;;
  arm64 | aarch64) arch=arm64 ;;
  *) echo "Unknown machine: $(uname -m)" >&2; exit 1 ;;
esac

rm -rf dist
mkdir -p dist
case "${RUNNER_OS:-$(uname -s)}" in
  Windows | MINGW* | MSYS*)
    cp target/release/ApduStudio.exe "dist/ApduStudio-windows-$arch.exe"
    cp target/release/apdustudio-cli.exe "dist/apdustudio-cli-windows-$arch.exe"
    ;;
  Linux)
    # Refuse an executable that wouldn't start on the oldest supported systems.
    "$(dirname "$0")/check-linux-abi.sh" target/release/ApduStudio
    "$(dirname "$0")/check-linux-abi.sh" target/release/apdustudio-cli
    tar -czf "dist/ApduStudio-linux-$arch.tar.gz" -C target/release ApduStudio apdustudio-cli
    ;;
  macOS | Darwin)
    tar -czf "dist/ApduStudio-macos-$arch.tar.gz" -C target/release ApduStudio apdustudio-cli
    ;;
  *)
    echo "Unknown system: ${RUNNER_OS:-$(uname -s)}" >&2
    exit 1
    ;;
esac
ls -l dist
