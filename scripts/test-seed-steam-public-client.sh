#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
test_root=$(mktemp -d /private/tmp/macroton-steam-seed-test.XXXXXX)
cleanup() { rm -rf "$test_root"; }
trap cleanup EXIT HUP INT TERM

source_root=$test_root/source
prefix=$test_root/target-prefix
target_root="$prefix/drive_c/Program Files (x86)/Steam"
mkdir -p "$source_root/package" "$source_root/config" "$source_root/userdata/123" \
  "$source_root/steamapps" "$target_root/package"
printf current-client > "$source_root/Steam.exe"
printf current-manifest > "$source_root/package/steam_client_win32.installed"
printf current-package > "$source_root/package/current.zip.vz.hash_1"
printf account > "$source_root/config/loginusers.vdf"
printf account > "$source_root/userdata/123/localconfig.vdf"
printf game > "$source_root/steamapps/appmanifest_42.acf"
printf stale > "$target_root/package/obsolete.zip.vz.old_1"
printf bootstrap > "$prefix/.macroton-steam-bootstrap.json"

"$script_dir/seed-steam-public-client.sh" "$source_root" "$target_root" >/dev/null

[ "$(cat "$target_root/package/current.zip.vz.hash_1")" = current-package ]
[ ! -e "$target_root/package/obsolete.zip.vz.old_1" ]
[ "$(cat "$prefix/.macroton-steam-client-seed-quarantine/package/obsolete.zip.vz.old_1")" = stale ]
[ ! -e "$target_root/config/loginusers.vdf" ]
[ ! -e "$target_root/userdata/123" ]
[ ! -e "$target_root/steamapps/appmanifest_42.acf" ]
[ -f "$prefix/.macroton-steam-client-seed" ]

echo "Steam public-client seed tests passed"
