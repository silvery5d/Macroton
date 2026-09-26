#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
launcher="$repo_root/scripts/open-steamcmd-login-terminal.sh"

if "$launcher" >/dev/null 2>&1; then
    echo "missing APPID was accepted" >&2
    exit 1
fi
if "$launcher" not-an-appid >/dev/null 2>&1; then
    echo "non-numeric APPID was accepted" >&2
    exit 1
fi
if "$launcher" 205100 not-a-limit >/dev/null 2>&1; then
    echo "non-numeric RSS limit was accepted" >&2
    exit 1
fi

printed=$("$launcher" --print-command 205100 700)
printf '%s\n' "$printed" | rg -q -- 'steamcmd login 205100'
if printf '%s\n' "$printed" | rg -i -- '(password|steam_guard|--password)'; then
    echo "printed login command contains a credential field" >&2
    exit 1
fi

# The script source must not contain credential-taking command-line fields.
if rg -n -- "(--password|password=|STEAM_PASSWORD|steam_guard_code)" "$launcher" >/dev/null; then
    echo "login launcher contains a credential field" >&2
    exit 1
fi

echo "visible SteamCMD login launcher safety tests passed"
