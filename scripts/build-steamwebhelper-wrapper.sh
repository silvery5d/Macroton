#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
source_file="$repo_root/tools/steamwebhelper-wrapper/steamwebhelper-wrapper.c"
output_file="${1:-$repo_root/tools/steamwebhelper-wrapper/steamwebhelper-wrapper.exe}"

compiler=${MINGW64_CC:-x86_64-w64-mingw32-gcc}
"$compiler" -O2 -Wall -Wextra -municode -mwindows -static \
  -o "$output_file" "$source_file" -lshell32

file "$output_file"
echo "built Macroton Steam CEF wrapper: $output_file"
