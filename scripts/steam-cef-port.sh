#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: $0 WEBHELPER_LOG" >&2
    exit 2
fi
log_path=$1
[ -f "$log_path" ] || {
    echo "Steam webhelper log does not exist: $log_path" >&2
    exit 1
}

# Steam appends startup records to this log. Use the newest command line that
# explicitly enables a localhost CEF debugger; never infer a port from an
# unrelated process or accept a remote endpoint.
port=$(awk '
    /--remote-debugging-address=127\.0\.0\.1/ && /--remote-debugging-port=[0-9]+/ {
        line = $0
        sub(/^.*--remote-debugging-port=/, "", line)
        sub(/[^0-9].*$/, "", line)
        if (line ~ /^[0-9]+$/) print line
    }
' "$log_path" | tail -n 1)
case "$port" in
    ''|*[!0-9]*)
        echo "no localhost Steam CEF debugging port found in $log_path" >&2
        exit 1
        ;;
esac
[ "$port" -ge 1 ] && [ "$port" -le 65535 ] || {
    echo "invalid Steam CEF debugging port in $log_path: $port" >&2
    exit 1
}
printf '%s\n' "$port"
