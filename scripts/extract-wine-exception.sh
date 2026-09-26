#!/bin/sh
set -eu

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
  echo "usage: $0 LOG_PATH [BOUNDARY_OFFSET_OR_FILE]" >&2
  exit 2
fi

log_path=$1
offset=0
if [ "$#" -eq 2 ]; then
  boundary=$2
  if [ -f "$boundary" ]; then
    offset=$(tr -d '[:space:]' < "$boundary")
  else
    offset=$boundary
  fi
  case "$offset" in
    ''|*[!0-9]*) echo "boundary must be a non-negative byte offset or file" >&2; exit 2 ;;
  esac
fi

if [ ! -f "$log_path" ]; then
  echo "log does not exist: $log_path" >&2
  exit 1
fi

# Steam/Wine logs are small enough for this diagnostic path, and dd gives a
# byte-precise boundary even when the log contains non-UTF-8 diagnostics.
body=$(dd if="$log_path" bs=1 skip="$offset" 2>/dev/null || true)
printf '%s\n' "$body" | awk '
  /wine: Unhandled/ {
    unhandled++
    if (first == "") first=$0
  }
  /exit code/ && exit_code == "" { exit_code=$0 }
  END {
    print "unhandled_count=" (unhandled + 0)
    print "first_unhandled=" (first == "" ? "none" : first)
    print "first_exit_code=" (exit_code == "" ? "none" : exit_code)
  }
'
