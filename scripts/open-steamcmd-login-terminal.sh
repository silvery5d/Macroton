#!/bin/sh
set -eu

[ "$#" -ge 1 ] && [ "$#" -le 3 ] || {
    echo "usage: $0 [--print-command] APPID [MAX_RSS_MIB]" >&2
    exit 2
}
print_command=0
if [ "${1:-}" = "--print-command" ]; then
    print_command=1
    shift
fi
app_id=${1:-}
max_rss_mib=${2:-700}
case "$app_id" in ''|*[!0-9]*) echo "APPID must be an unsigned integer" >&2; exit 2;; esac
case "$max_rss_mib" in ''|*[!0-9]*) echo "MAX_RSS_MIB must be an unsigned integer" >&2; exit 2;; esac

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
tool_root="$repo_root/.build-cache/steamcmd"
runtime_root="$HOME/Library/Application Support/Macroton/runtimes"
data_root="$HOME/Library/Application Support/Macroton"
macroton="$repo_root/target/debug/macroton"

[ -x "$macroton" ] || { echo "Macroton binary missing: $macroton" >&2; exit 1; }
[ -f "$tool_root/steamcmd.exe" ] || { echo "SteamCMD missing: $tool_root/steamcmd.exe" >&2; exit 1; }

# Open a real interactive Terminal window. No secret is placed in this
# command: SteamCMD reads the password at its own hidden prompt and handles
# Steam Guard interactively.
command_line="cd $(printf '%s' "$repo_root" | sed "s/'/'\\''/g"); exec '$macroton' steamcmd login $app_id --tool-root '$tool_root' --runtime-id macroton-winehq-macos-11.14 --runtime-root '$runtime_root' --data-root '$data_root' --max-rss-mib $max_rss_mib"
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

echo "Opened a visible Terminal login window for AppID $app_id."
