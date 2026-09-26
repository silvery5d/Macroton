#!/bin/sh
set -eu

[ "$#" -ge 1 ] && [ "$#" -le 3 ] || {
    echo "usage: $0 [--print-command] APPID [ACCOUNT_NAME]" >&2
    exit 2
}
print_command=0
if [ "$1" = "--print-command" ]; then
    print_command=1
    shift
fi
app_id=$1
account=${2:-}
case "$app_id" in ''|*[!0-9]*) echo "APPID must be an unsigned integer" >&2; exit 2;; esac

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
tool_root="$repo_root/.build-cache/steamcmd"
runtime_root="$HOME/Library/Application Support/Macroton/runtimes"
data_root="$HOME/Library/Application Support/Macroton"
macroton="$repo_root/target/debug/macroton"
profile="$repo_root/compatibility/experiments/"$app_id"-preflight.conf"

[ -x "$macroton" ] || { echo "Macroton binary missing: $macroton" >&2; exit 1; }
[ -f "$tool_root/steamcmd.exe" ] || { echo "SteamCMD missing: $tool_root/steamcmd.exe" >&2; exit 1; }
[ -f "$profile" ] || { echo "preflight profile missing: $profile" >&2; exit 1; }

account_arg=""
if [ -n "$account" ]; then
    case "$account" in *[!A-Za-z0-9_.-]*) echo "ACCOUNT_NAME contains unsupported characters" >&2; exit 2;; esac
    account_arg=" --account '$account'"
fi
command_line="cd $(printf '%s' "$repo_root" | sed "s/'/'\\''/g"); exec '$macroton' steamcmd acquire $app_id --profile '$profile' --tool-root '$tool_root' --runtime-id macroton-winehq-macos-11.14 --runtime-root '$runtime_root' --data-root '$data_root'$account_arg"
if [ "$print_command" -eq 1 ]; then
    printf '%s\n' "$command_line"
    exit 0
fi
osascript <<EOF
tell application "Terminal"
    activate
    do script "$command_line"
end tell
EOF

echo "Opened a visible Terminal depot-acquisition window for AppID $app_id."
