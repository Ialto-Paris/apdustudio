#!/usr/bin/env bash
# The checks CI doesn't run by itself: run them before pushing. From the repository's root:
#
#   scripts/check.sh            format, clippy and tests, for this system
#   scripts/check.sh --cross    also clippy for the other systems (Windows, Linux, macOS targets
#                               that rustup has: rustup target add x86_64-unknown-linux-gnu
#                               aarch64-apple-darwin x86_64-pc-windows-msvc)
#   scripts/check.sh --release  also the release build (target/release/ApduStudio)
#
# The flags combine. Cross-checking needs no PC/SC library for the other system: clippy doesn't
# link. It uses its own target folder, target/cross, so the normal builds aren't redone.
set -euo pipefail

cross=0
release=0
for arg in "$@"; do
  case "$arg" in
    --cross) cross=1 ;;
    --release) release=1 ;;
    -h | --help) sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "unknown option: $arg (see --help)" >&2; exit 2 ;;
  esac
done

export CARGO_INCREMENTAL=0
step() { printf '\n== %s\n' "$*"; }

step "format"
cargo fmt --all -- --check

step "clippy"
cargo clippy --all-targets --locked -- -D warnings

step "tests"
cargo test --locked

if [ "$cross" = 1 ]; then
  host=$(rustc -vV | sed -n 's/^host: //p')
  installed=$(rustup target list --installed)
  for target in x86_64-pc-windows-msvc x86_64-unknown-linux-gnu aarch64-apple-darwin; do
    [ "$target" = "$host" ] && continue
    if ! grep -qx "$target" <<< "$installed"; then
      echo "skipping $target: not installed (rustup target add $target)"
      continue
    fi
    step "clippy for $target"
    # pcsc-sys looks for the PC/SC library only to link, which clippy doesn't do.
    PCSC_LIB_DIR="${PCSC_LIB_DIR:-/tmp}" CARGO_TARGET_DIR=target/cross \
      cargo clippy --target "$target" --all-targets --locked -- -D warnings
  done
fi

if [ "$release" = 1 ]; then
  step "release build"
  cargo build --release --locked
  ls -l target/release/ApduStudio* 2>/dev/null | grep -v '\.d$' || true
fi

printf '\nAll checks passed.\n'
