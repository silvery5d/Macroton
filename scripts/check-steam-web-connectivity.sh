#!/bin/sh
# Read-only host-side prerequisite for Steam's web-based install and EULA UI.
# A working Steam client connection does not prove these HTTPS endpoints work.
set -eu

if ! command -v curl >/dev/null 2>&1; then
    echo "curl is required for the Steam web connectivity check" >&2
    exit 2
fi

check_url() {
    label=$1
    url=$2
    result=$(curl -IL --connect-timeout 4 --max-time 10 --silent --show-error \
        --output /dev/null --write-out '%{http_code} %{remote_ip}' "$url" 2>&1) || {
        printf '%s: unavailable (%s)\n' "$label" "$result"
        return 1
    }
    status=${result%% *}
    case "$status" in
        2??|3??) printf '%s: available (HTTP %s)\n' "$label" "$status" ;;
        *) printf '%s: unavailable (HTTP %s)\n' "$label" "$status"; return 1 ;;
    esac
}

failed=0
check_url 'Steam Store / agreement UI' 'https://store.steampowered.com/' || failed=1
check_url 'Steam CDN / client assets' 'https://cdn.steamstatic.com/' || failed=1

if [ "$failed" -ne 0 ]; then
    echo 'Steam web content is not fully reachable; a signed-in client may still fail to display an EULA or store page.' >&2
    exit 1
fi
echo 'Steam web prerequisites are reachable; this does not prove the Wine client or game works.'
