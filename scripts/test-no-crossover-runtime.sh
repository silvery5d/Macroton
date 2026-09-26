#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(dirname -- "$script_dir")

# Comments may explain the policy; only effective assignment values are
# executable inputs and therefore must be free of commercial runtime paths.
matches=$(rg -ni '^[[:space:]]*(runtime|entrypoint|path|artifact|source)[[:space:]]*=.*(crossover|codeweavers)' \
    "$repo_root/runtimes" "$repo_root/compatibility" 2>/dev/null || true)
if [ -n "$matches" ]; then
    echo "CrossOver/CodeWeavers path found in an effective runtime/profile assignment:" >&2
    echo "$matches" >&2
    exit 1
fi

echo "CrossOver runtime assignment audit passed"
