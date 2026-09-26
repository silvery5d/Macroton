#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
helper="$repo_root/scripts/open-steamcmd-acquire-terminal.sh"

[ -x "$helper" ]
if "$helper" >/tmp/macroton-acquire-helper-missing.out 2>&1; then
    echo "expected missing-argument failure" >&2
    exit 1
fi
grep -q "usage:" /tmp/macroton-acquire-helper-missing.out

command_line=$("$helper" --print-command 205100)
printf '%s\n' "$command_line" | grep -q "steamcmd acquire 205100"
printf '%s\n' "$command_line" | grep -q "205100-preflight.conf"
if printf '%s\n' "$command_line" | grep -Eq -- "--password|STEAM_PASSWORD|Steam Guard"; then
    echo "credential material leaked into acquisition command" >&2
    exit 1
fi

echo "open-steamcmd-acquire-terminal tests passed"
