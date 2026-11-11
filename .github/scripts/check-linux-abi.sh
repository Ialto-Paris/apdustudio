#!/usr/bin/env bash
# Checks that a Linux executable starts on the oldest systems ApduStudio supports:
#   - no glibc symbol newer than MAX_GLIBC (default 2.28: Debian 10, Ubuntu 20.04, RHEL 8);
#   - linked only to libraries every system has, plus pcsc-lite (the window's libraries,
#     X11, Wayland and OpenGL, are loaded when it starts, not linked).
# Usage: check-linux-abi.sh <executable>        (needs objdump, from binutils)
set -euo pipefail

exe=${1:?usage: check-linux-abi.sh <executable>}
max=${MAX_GLIBC:-2.28}
allowed=" libc.so.6 libm.so.6 libgcc_s.so.1 libpcsclite.so.1 libdl.so.2 libpthread.so.0 librt.so.1 ld-linux-x86-64.so.2 ld-linux-aarch64.so.1 "

# a is newer than b
newer() { [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -n 1)" = "$1" ]; }

failed=0
too_new=$(objdump -T "$exe" | awk '/GLIBC_[0-9]/ { v = $(NF-1); gsub(/[()]/, "", v); sub(/^GLIBC_/, "", v); print v, $NF }' | sort -u |
  while read -r version symbol; do
    if newer "$version" "$max"; then echo "  $symbol needs glibc $version"; fi
  done)
if [ -n "$too_new" ]; then
  echo "::error::$exe needs a glibc newer than $max:"
  echo "$too_new"
  failed=1
fi

for lib in $(objdump -p "$exe" | awk '$1 == "NEEDED" { print $2 }'); do
  case "$allowed" in
    *" $lib "*) ;;
    *) echo "::error::$exe is linked to $lib, which isn't on every system: load it at run time, or add it to the documented requirements and to this list"; failed=1 ;;
  esac
done

highest=$(objdump -T "$exe" | grep -oE 'GLIBC_[0-9]+(\.[0-9]+)+' | sed 's/^GLIBC_//' | sort -Vu | tail -n 1)
echo "$exe: highest glibc needed ${highest:-none} (at most $max); linked to: $(objdump -p "$exe" | awk '$1 == "NEEDED" { printf "%s ", $2 }')"
exit "$failed"
