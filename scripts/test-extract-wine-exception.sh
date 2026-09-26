#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/macroton-exception-test.XXXXXX")
trap 'rm -rf "$WORK"' EXIT INT TERM

printf '%s\n' \
  'wine: Unhandled page fault before launch' \
  'old exit code 1' \
  > "$WORK/log"
boundary=$(wc -c < "$WORK/log" | tr -d '[:space:]')
printf '%s\n' \
  'wine: Unhandled page fault on read access to 00002DC1 at address 7BDE1239' \
  'game exit code -1073741819' \
  'wine: Unhandled division by zero at address 000000014000D7CD' \
  >> "$WORK/log"
printf '%s' "$boundary" > "$WORK/boundary"

output=$("$ROOT/scripts/extract-wine-exception.sh" "$WORK/log" "$WORK/boundary")
printf '%s\n' "$output" | grep -F 'unhandled_count=2' >/dev/null
printf '%s\n' "$output" | grep -F 'first_unhandled=wine: Unhandled page fault on read access' >/dev/null
printf '%s\n' "$output" | grep -F 'first_exit_code=game exit code -1073741819' >/dev/null
