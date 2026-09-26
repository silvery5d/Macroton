#!/bin/sh
# Capture a short-lived Wine game process map without tracing the whole Steam client.
set -eu

if [ "$#" -ne 2 ]; then
    echo "usage: sh scripts/capture-wine-pe-map.sh GAME_EXE_NAME OUTPUT_PATH" >&2
    exit 2
fi

game_exe=$1
output_path=$2
case "$game_exe" in
    */*|*\\*|'') echo "GAME_EXE_NAME must be an executable basename" >&2; exit 2 ;;
esac
case "$output_path" in
    /*) ;;
    *) echo "OUTPUT_PATH must be absolute" >&2; exit 2 ;;
esac

attempt=0
while [ "$attempt" -lt 400 ]; do
    game_pid=$(ps -axo pid=,command= | awk -v name="$game_exe" '
        $0 ~ /^[[:space:]]*[0-9]+[[:space:]]+C:\\/ && index($0, "\\" name) {
            print $1
            exit
        }
    ')
    if [ -n "$game_pid" ]; then
        echo "capturing $game_exe PID $game_pid to $output_path"
        # Wine's macOS host process is x86_64 under Rosetta. Running vmmap
        # under the same architecture avoids its failed auto-respawn path.
        if arch -x86_64 /usr/bin/vmmap -w "$game_pid" > "$output_path" 2>&1; then
            echo "captured PID $game_pid"
            exit 0
        fi
        echo "vmmap failed for PID $game_pid (the process may have exited)" >&2
    fi
    attempt=$((attempt + 1))
    sleep 0.05
done

echo "no matching Wine game process observed within 20 seconds" >&2
exit 1
