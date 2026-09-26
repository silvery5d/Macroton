#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
appid=${1:?usage: install-steamwebhelper-wrapper.sh APPID [DATA_ROOT] [WRAPPER_EXE]}
data_root=${2:-"$HOME/Library/Application Support/Macroton"}
wrapper=${3:-"$repo_root/tools/steamwebhelper-wrapper/steamwebhelper-wrapper.exe"}
cef="$data_root/games/$appid/prefix/drive_c/Program Files (x86)/Steam/bin/cef/cef.win64"
steam_root="$data_root/games/$appid/prefix/drive_c/Program Files (x86)/Steam"

test -f "$wrapper"
test -f "$cef/steamwebhelper.exe"
test ! -L "$cef/steamwebhelper.exe"
test ! -L "$cef/steamwebhelper_real.exe"

# The managed runtime refuses to repair a live prefix. Apply the same rule to
# this manual opt-in installer: replacing an executable while Steam/CEF is
# running can leave an inconsistent session and invalidate the UI evidence.
macroton_bin=${MACROTON_BIN:-"$repo_root/target/debug/macroton"}
if [ ! -x "$macroton_bin" ]; then
  echo "Macroton binary is required for exact-prefix idle check: $macroton_bin" >&2
  echo "Build it with 'cargo build' or set MACROTON_BIN." >&2
  exit 1
fi
resource_status=$("$macroton_bin" resource status "$appid" --data-root "$data_root" --json) || {
  echo "cannot verify that AppID $appid prefix is idle; refusing CEF mutation" >&2
  exit 1
}
process_count=$(printf '%s\n' "$resource_status" | sed -n 's/^[[:space:]]*"process_count": \([0-9][0-9]*\),*$/\1/p')
if [ -z "$process_count" ]; then
  echo "cannot parse AppID $appid exact-prefix process count; refusing CEF mutation" >&2
  exit 1
fi
if [ "$process_count" -ne 0 ]; then
  echo "AppID $appid prefix has $process_count live process(es); stop it with --preserve-session before installing the CEF wrapper" >&2
  exit 1
fi

# Steam self-updates may replace the wrapper with a newer Valve helper. In that
# case preserve the previous real helper and use the newly installed one as the
# delegate, instead of silently running an obsolete helper forever.
current_hash=$(shasum -a 256 "$cef/steamwebhelper.exe" | cut -d ' ' -f 1)
wrapper_hash=$(shasum -a 256 "$wrapper" | cut -d ' ' -f 1)
if [ "$current_hash" != "$wrapper_hash" ]; then
  if [ -f "$cef/steamwebhelper_real.exe" ]; then
    backup="$cef/steamwebhelper_real.previous.$(date +%s).exe"
    mv "$cef/steamwebhelper_real.exe" "$backup"
    echo "preserved previous Valve helper at $backup"
  fi
  mv "$cef/steamwebhelper.exe" "$cef/steamwebhelper_real.exe"
elif [ ! -f "$cef/steamwebhelper_real.exe" ]; then
  echo "wrapper is present but Valve's real helper is missing" >&2
  exit 1
fi
cp "$wrapper" "$cef/steamwebhelper.exe"
cp "$repo_root/tools/steamwebhelper-wrapper/steam.cfg" "$steam_root/steam.cfg"
file "$cef/steamwebhelper.exe" "$cef/steamwebhelper_real.exe"
echo "installed reversible Macroton Steam CEF wrapper for AppID $appid"
