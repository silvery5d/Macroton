#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(dirname -- "$script_dir")
test_root=$(mktemp -d /private/tmp/macroton-swift-tools-test.XXXXXX)
cleanup() {
    rm -rf "$test_root"
}
trap cleanup EXIT HUP INT TERM

module_cache=$test_root/module-cache
mkdir -p "$module_cache"
export CLANG_MODULE_CACHE_PATH=$module_cache
export SWIFT_MODULE_CACHE_PATH=$module_cache

compile() {
    source_path=$1
    output_path=$2
    swiftc -warnings-as-errors "$source_path" -o "$output_path"
    [ -x "$output_path" ] || {
        echo "Swift tool is not executable after compilation: $output_path" >&2
        exit 1
    }
}

compile "$repo_root/tools/macos_window_locator.swift" "$test_root/window-locator"
compile "$repo_root/tools/macos_focus_stream.swift" "$test_root/focus-stream"
compile "$repo_root/tools/macos_audio_probe.swift" "$test_root/audio-probe"
compile "$repo_root/tools/macos_interaction_probe.swift" "$test_root/interaction-probe"

if "$test_root/window-locator" >"$test_root/locator.out" 2>"$test_root/locator.err"; then
    echo "window locator unexpectedly accepted a missing PID" >&2
    exit 1
fi
grep -q 'usage: macos_window_locator PID' "$test_root/locator.err"

# The explicit Window ID form must parse independently of candidate discovery.
# Use this shell's PID and an impossible window ID so the test never activates
# or interacts with a real application.
if "$test_root/window-locator" "$$" 4294967295 --any >"$test_root/explicit.out" 2>"$test_root/explicit.err"; then
    echo "window locator unexpectedly accepted an impossible explicit Window ID" >&2
    exit 1
fi
if grep -q 'usage: macos_window_locator PID' "$test_root/explicit.err"; then
    echo "window locator rejected the explicit Window ID syntax" >&2
    exit 1
fi

echo "macOS Swift tool compile tests passed"
