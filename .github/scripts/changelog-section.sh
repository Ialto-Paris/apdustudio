#!/usr/bin/env bash
# Prints the section of CHANGELOG.md for a version, without its heading: the notes of its
# GitHub release. Fails when the section is missing or empty, so that no version goes out
# without them. From the repository's root:
#
#   .github/scripts/changelog-section.sh 1.1.0 [CHANGELOG.md]
#
# The section runs from its heading, "## [1.1.0] - 2026-11-16", to the next "## [" heading. The
# link definitions at the end of the file ("[1.1.0]: https://...") and the blank lines around
# the section are left out.
set -euo pipefail

version=${1:?usage: changelog-section.sh <version> [changelog]}
file=${2:-CHANGELOG.md}

notes=$(awk -v heading="## [$version]" '
  /^## \[/ { if (found) exit; found = (index($0, heading) == 1); next }
  found && !/^\[[^]]+\]: / { print }
' "$file" | sed -e '/[^[:space:]]/,$!d')

if [ -z "$notes" ]; then
  echo "::error::$file has no section for $version: add one headed \"## [$version] - <date>\"" >&2
  exit 1
fi
printf '%s\n' "$notes"
